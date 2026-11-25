use crate::outcome::Diagnostic;
use postgres::config::SslMode;
use postgres::error::SqlState;
use postgres::{Client, Config, NoTls};
use std::collections::BTreeMap;
use std::error::Error;
use std::io;
use std::net::IpAddr;
use std::time::Duration;

#[derive(Clone)]
pub struct Connection {
    host: IpAddr,
    port: u16,
    pub database: String,
    user: String,
    password: String,
}

#[derive(Debug)]
pub struct FieldError {
    pub field: &'static str,
    pub message: &'static str,
}

impl Connection {
    pub fn new(
        host: &str,
        port: &str,
        database: &str,
        user: &str,
        password: &str,
    ) -> Result<Self, FieldError> {
        let host = match host.trim() {
            "localhost" => IpAddr::from([127, 0, 0, 1]),
            value => value
                .parse::<IpAddr>()
                .ok()
                .filter(IpAddr::is_loopback)
                .ok_or(FieldError {
                    field: "Адрес",
                    message: "Укажите локальный адрес, например 127.0.0.1, ::1 или localhost.",
                })?,
        };
        let port = port
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or(FieldError {
                field: "Порт",
                message: "Укажите число от 1 до 65535.",
            })?;
        if database.is_empty() {
            return Err(FieldError {
                field: "База данных",
                message: "Укажите имя базы, например postgres.",
            });
        }
        if user.is_empty() {
            return Err(FieldError {
                field: "Пользователь",
                message: "Укажите имя пользователя PostgreSQL.",
            });
        }
        for (field, value) in [
            ("База данных", database),
            ("Пользователь", user),
            ("Пароль", password),
        ] {
            if value.contains('\0') {
                return Err(FieldError {
                    field,
                    message: "Поле не должно содержать нулевой символ.",
                });
            }
        }
        Ok(Self {
            host,
            port,
            database: database.into(),
            user: user.into(),
            password: password.into(),
        })
    }

    /// Адрес подключения для привязки черновика; пароль не раскрывается.
    pub fn endpoint(&self) -> (String, u16) {
        (self.host.to_string(), self.port)
    }

    pub(crate) fn connect(&self) -> Result<Client, postgres::Error> {
        self.config()
            .options("-c statement_timeout=5000")
            .connect(NoTls)
    }

    pub(crate) fn config(&self) -> Config {
        Config::new()
            .hostaddr(self.host)
            .port(self.port)
            .dbname(&self.database)
            .user(&self.user)
            .password(&self.password)
            .ssl_mode(SslMode::Disable)
            .connect_timeout(Duration::from_secs(4))
            .clone()
    }

    pub(crate) fn diagnostic(&self, error: postgres::Error) -> Diagnostic {
        Diagnostic::from_postgres(error, &self.password)
    }

    pub(crate) fn request_error(&self, error: postgres::Error, connecting: bool) -> RequestError {
        RequestError::from_postgres(error, &self.password, connecting)
    }

    pub(crate) fn redact(&self, text: &str) -> String {
        if self.password.is_empty() {
            text.into()
        } else {
            text.replace(&self.password, "[скрыто]")
        }
    }
}

#[derive(Debug)]
pub struct RequestError {
    pub message: &'static str,
    pub detail: String,
}

impl RequestError {
    fn from_postgres(error: postgres::Error, password: &str, connecting: bool) -> Self {
        let message = match error.code() {
            Some(code) if code == &SqlState::INVALID_PASSWORD => {
                "PostgreSQL отклонил пароль. Проверьте пользователя и пароль."
            }
            Some(code) if code == &SqlState::INVALID_AUTHORIZATION_SPECIFICATION => {
                "PostgreSQL отклонил вход. Проверьте пользователя и правила доступа."
            }
            Some(code) if code == &SqlState::INVALID_CATALOG_NAME => {
                "База данных не найдена. Проверьте её имя."
            }
            Some(code) if code == &SqlState::INSUFFICIENT_PRIVILEGE => {
                "У пользователя недостаточно прав для этой операции."
            }
            _ if connecting && caused_by_network(&error) => {
                "Нет ответа от PostgreSQL. Проверьте адрес, порт и работу сервера."
            }
            _ if connecting => "Не удалось подключиться к PostgreSQL.",
            _ => "Не удалось прочитать сведения из PostgreSQL.",
        };
        let detail = Diagnostic::from_postgres(error, password).render();
        Self { message, detail }
    }
}

fn caused_by_network(error: &postgres::Error) -> bool {
    let mut cause = error.source();
    while let Some(err) = cause {
        if err.downcast_ref::<io::Error>().is_some() {
            return true;
        }
        cause = err.source();
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoutineKind {
    Function,
    Procedure,
}

impl RoutineKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Function => "Функция",
            Self::Procedure => "Процедура",
        }
    }
}

pub struct Routine {
    pub oid: u32,
    pub name: String,
    pub kind: RoutineKind,
    pub signature: String,
    pub debug_allowed: bool,
    pub owner: String,
}

#[derive(Clone)]
pub struct Argument {
    pub name: Option<String>,
    pub type_name: String,
    pub mode: String,
    pub type_oid: u32,
    pub custom_type: Option<ArgumentType>,
    pub has_default: bool,
}

#[derive(Clone)]
pub struct ArgumentType {
    pub schema: String,
    pub name: String,
    pub base_oid: u32,
    pub enum_labels: Option<Vec<String>>,
    pub array: bool,
    pub fields: Option<Vec<ArgumentField>>,
}

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArgumentField {
    pub name: String,
    pub type_name: String,
}

impl Argument {
    pub fn input_format(&self) -> &'static str {
        match &self.custom_type {
            Some(ty) if ty.array => "array",
            Some(ty) if ty.fields.is_some() => "composite",
            Some(ty) if ty.enum_labels.is_some() => "enum",
            Some(ty) => crate::scalar::input_format(ty.base_oid),
            None => crate::scalar::input_format(self.type_oid),
        }
    }

    pub fn accepts_input(&self) -> bool {
        matches!(self.mode.as_str(), "IN" | "INOUT")
            && (self.custom_type.is_some() || crate::scalar::supported(self.type_oid))
    }

    pub fn value_is_valid(&self, value: &str) -> bool {
        self.value_error(value).is_none()
    }

    pub fn value_error(&self, value: &str) -> Option<&'static str> {
        let Some(ty) = &self.custom_type else {
            return crate::scalar::value_error(self.type_oid, value);
        };
        if ty.array || ty.fields.is_some() {
            return value.contains('\0').then_some("Нулевой символ недопустим.");
        }
        if let Some(labels) = &ty.enum_labels {
            return (!labels.iter().any(|label| label == value))
                .then_some("Выберите значение из списка PostgreSQL.");
        }
        crate::scalar::value_error(ty.base_oid, value)
    }
}

pub struct RoutineDetails {
    pub oid: u32,
    pub source: String,
    pub definition: String,
    pub arguments: Vec<Argument>,
    pub returns_set: bool,
    pub trigger_targets: Option<Vec<crate::trigger::TriggerTarget>>,
}

#[derive(Clone, Default)]
pub struct Selection {
    pub schema: Option<String>,
    pub routine_oid: Option<u32>,
}

pub struct Catalog {
    pub server_version: String,
    pub current_user: String,
    pub databases: Vec<String>,
    pub schemas: Vec<String>,
    pub schema: Option<String>,
    pub routines: Vec<Routine>,
    pub selected: Option<RoutineDetails>,
    pub debugger: DebuggerStatus,
    pub debugger_setup: Option<DebuggerSetup>,
}

#[derive(serde::Serialize)]
pub struct DebuggerSetup {
    extension_created: bool,
    package_available: bool,
    preload: Option<bool>,
    restart_pending: Option<bool>,
    superuser: bool,
    read_only: bool,
    api_allowed: bool,
}

impl DebuggerSetup {
    fn status(&self) -> DebuggerStatus {
        match (self.extension_created, self.api_allowed, self.preload) {
            (false, _, _) => DebuggerStatus::MissingExtension,
            (_, false, _) => DebuggerStatus::AccessDenied,
            (_, _, None) => DebuggerStatus::PluginUnverified,
            (_, _, Some(true)) => DebuggerStatus::PluginConfigured,
            (_, _, Some(false)) => DebuggerStatus::PluginNotConfigured,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DebuggerStatus {
    MissingExtension,
    PluginNotConfigured,
    PluginConfigured,
    PluginUnverified,
    AccessDenied,
    Unknown,
}

impl DebuggerStatus {
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingExtension => "missing_extension",
            Self::PluginNotConfigured => "plugin_not_configured",
            Self::PluginConfigured => "configured",
            Self::PluginUnverified => "unverified",
            Self::AccessDenied => "access_denied",
            Self::Unknown => "unknown",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::MissingExtension => {
                "Каталог доступен. Для отладки попроси администратора создать расширение pldbgapi в выбранной базе."
            }
            Self::PluginNotConfigured => {
                "Каталог доступен. Для отладки администратор должен добавить plugin_debugger в shared_preload_libraries и перезапустить PostgreSQL."
            }
            Self::PluginConfigured => "Компоненты отладки доступны.",
            Self::PluginUnverified => {
                "Расширение доступно. PostgreSQL скрывает настройку загрузки плагина для этой роли. Готовность проверится при запуске."
            }
            Self::AccessDenied => {
                "Каталог доступен. Для отладки попроси администратора проверить права на схему и функции расширения pldbgapi."
            }
            Self::Unknown => {
                "Не удалось проверить компоненты отладки. Попроси администратора проверить состояние PostgreSQL и расширения pldbgapi."
            }
        }
    }
}

pub fn browse(connection: &Connection, selection: &Selection) -> Result<Catalog, RequestError> {
    let mut client = connection
        .connect()
        .map_err(|err| RequestError::from_postgres(err, &connection.password, true))?;
    load_catalog(&mut client, selection)
        .map_err(|err| RequestError::from_postgres(err, &connection.password, false))
}

fn load_catalog(client: &mut Client, selection: &Selection) -> Result<Catalog, postgres::Error> {
    let environment = client.query_one(
        "SELECT pg_catalog.current_setting('server_version'), current_user::text",
        &[],
    )?;
    let databases = client
        .query(
            "SELECT datname::text FROM pg_catalog.pg_database
             WHERE datallowconn AND pg_catalog.has_database_privilege(oid, 'CONNECT')
             ORDER BY datname",
            &[],
        )?
        .into_iter()
        .map(|row| row.try_get(0))
        .collect::<Result<Vec<String>, _>>()?;
    let schemas = client
        .query(
            "SELECT nspname::text FROM pg_catalog.pg_namespace
             WHERE pg_catalog.left(nspname::text, 3) <> 'pg_'
             AND nspname <> 'information_schema'
             AND pg_catalog.has_schema_privilege(oid, 'USAGE')
             ORDER BY nspname",
            &[],
        )?
        .into_iter()
        .map(|row| row.try_get(0))
        .collect::<Result<Vec<String>, _>>()?;
    let schema = selection
        .schema
        .as_ref()
        .filter(|name| schemas.contains(name))
        .or_else(|| schemas.iter().find(|name| *name == "public"))
        .or_else(|| schemas.first())
        .cloned();

    let routines = match &schema {
        Some(schema) => routines(client, schema)?,
        None => Vec::new(),
    };
    let selected = match selection
        .routine_oid
        .filter(|oid| routines.iter().any(|routine| routine.oid == *oid))
    {
        Some(oid) => routine_details(client, oid)?,
        None => None,
    };
    let debugger_setup = debugger_setup(client).ok();
    Ok(Catalog {
        server_version: environment.try_get(0)?,
        current_user: environment.try_get(1)?,
        databases,
        schemas,
        schema,
        routines,
        selected,
        debugger: debugger_setup
            .as_ref()
            .map_or(DebuggerStatus::Unknown, DebuggerSetup::status),
        debugger_setup,
    })
}

fn routines(client: &mut Client, schema: &str) -> Result<Vec<Routine>, postgres::Error> {
    client
        .query(
            "SELECT p.oid, p.proname::text, p.prokind::text,
                    pg_catalog.pg_get_function_identity_arguments(p.oid),
                    EXISTS (
                        SELECT FROM pg_catalog.pg_roles AS r
                        WHERE r.rolname = current_user AND (r.rolsuper OR r.oid = p.proowner)
                    ), pg_catalog.pg_get_userbyid(p.proowner)
             FROM pg_catalog.pg_proc AS p
             JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
             JOIN pg_catalog.pg_language AS l ON l.oid = p.prolang
             WHERE n.nspname::text = $1 AND l.lanname = 'plpgsql'
             AND p.prokind IN ('f', 'p')
             AND pg_catalog.has_function_privilege(p.oid, 'EXECUTE')
             ORDER BY p.proname, 4, p.oid",
            &[&schema],
        )?
        .into_iter()
        .map(|row| {
            let kind: &str = row.try_get(2)?;
            Ok(Routine {
                oid: row.try_get(0)?,
                name: row.try_get(1)?,
                kind: if kind == "p" {
                    RoutineKind::Procedure
                } else {
                    RoutineKind::Function
                },
                signature: row.try_get(3)?,
                debug_allowed: row.try_get(4)?,
                owner: row.try_get(5)?,
            })
        })
        .collect()
}

fn routine_details(
    client: &mut Client,
    oid: u32,
) -> Result<Option<RoutineDetails>, postgres::Error> {
    // Тело и определение передаются один раз, даже при сотне аргументов.
    // LATERAL собирает аргументы в том же снимке PostgreSQL.
    let row = client.query_opt(
        "SELECT p.prosrc, pg_catalog.pg_get_functiondef(p.oid),
                a.types, a.names, a.type_names, a.modes, p.pronargdefaults, p.proretset,
                p.prorettype = 'pg_catalog.trigger'::pg_catalog.regtype
         FROM pg_catalog.pg_proc p
         LEFT JOIN LATERAL (
             SELECT pg_catalog.array_agg(arg.type_oid ORDER BY arg.ord) AS types,
                    pg_catalog.array_agg(NULLIF(p.proargnames[arg.ord::int], '') ORDER BY arg.ord) AS names,
                    pg_catalog.array_agg(pg_catalog.format_type(arg.type_oid, NULL) ORDER BY arg.ord) AS type_names,
                    pg_catalog.array_agg(COALESCE(p.proargmodes[arg.ord::int]::text, 'i') ORDER BY arg.ord) AS modes
             FROM unnest(COALESCE(p.proallargtypes, p.proargtypes::oid[]))
                  WITH ORDINALITY arg(type_oid, ord)
         ) a ON true
         WHERE p.oid = $1 AND pg_catalog.has_function_privilege(p.oid, 'EXECUTE')",
        &[&oid],
    )?;
    let Some(row) = row else {
        return Ok(None);
    };
    let source = row.try_get(0)?;
    let definition = row.try_get(1)?;
    let types: Vec<u32> = row.try_get::<_, Option<Vec<u32>>>(2)?.unwrap_or_default();
    let names: Vec<Option<String>> = row
        .try_get::<_, Option<Vec<Option<String>>>>(3)?
        .unwrap_or_default();
    let type_names: Vec<String> = row
        .try_get::<_, Option<Vec<String>>>(4)?
        .unwrap_or_default();
    let modes: Vec<String> = row
        .try_get::<_, Option<Vec<String>>>(5)?
        .unwrap_or_default();
    let defaults = usize::try_from(row.try_get::<_, i16>(6)?).unwrap_or(0);
    let mut inputs_left = modes
        .iter()
        .filter(|mode| matches!(mode.as_str(), "i" | "b" | "v"))
        .count();
    let unresolved = types
        .iter()
        .copied()
        .filter(|oid| !crate::scalar::supported(*oid))
        .collect::<Vec<_>>();
    let custom_types = argument_types(client, &unresolved)?;
    let arguments = types
        .into_iter()
        .zip(names)
        .zip(type_names)
        .zip(modes)
        .map(|(((type_oid, name), type_name), mode)| {
            let mut has_default = false;
            if matches!(mode.as_str(), "i" | "b" | "v") {
                has_default = inputs_left <= defaults;
                inputs_left -= 1;
            }
            Argument {
                name,
                type_oid,
                type_name,
                custom_type: custom_types.get(&type_oid).cloned(),
                has_default,
                mode: match mode.as_str() {
                    "o" => "OUT",
                    "b" => "INOUT",
                    "v" => "VARIADIC",
                    "t" => "TABLE",
                    _ => "IN",
                }
                .into(),
            }
        })
        .collect();

    Ok(Some(RoutineDetails {
        oid,
        source,
        definition,
        arguments,
        returns_set: row.try_get(7)?,
        trigger_targets: if row.try_get(8)? {
            Some(crate::trigger::targets(client, oid)?)
        } else {
            None
        },
    }))
}

fn argument_types(
    client: &mut Client,
    oids: &[u32],
) -> Result<BTreeMap<u32, ArgumentType>, postgres::Error> {
    if oids.is_empty() {
        return Ok(BTreeMap::new());
    }
    // У домена и массива проверяем вложенный тип, а при вызове приводим к исходному.
    let rows = client.query(
        "WITH RECURSIVE chain(original, current, is_array) AS (
              SELECT oid, oid, false FROM pg_catalog.pg_type WHERE oid = ANY($1)
              UNION
              SELECT c.original, CASE WHEN t.typtype = 'd' THEN t.typbasetype ELSE t.typelem END,
                     c.is_array OR t.typtype <> 'd'
              FROM chain c JOIN pg_catalog.pg_type t ON t.oid = c.current
              WHERE t.typtype = 'd' OR (t.typtype = 'b' AND t.typcategory = 'A' AND t.typelem <> 0)
         )
         SELECT t.oid, n.nspname::text, t.typname::text, b.oid,
                CASE WHEN b.typtype = 'e' AND NOT c.is_array THEN ARRAY(
                    SELECT e.enumlabel::text FROM pg_catalog.pg_enum e
                    WHERE e.enumtypid = b.oid ORDER BY e.enumsortorder
                ) END,
                c.is_array, b.typtype::text,
                CASE WHEN b.typtype = 'c' AND NOT c.is_array THEN ARRAY(
                    SELECT a.attname::text FROM pg_catalog.pg_attribute a
                    WHERE a.attrelid = b.typrelid AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum
                ) END,
                CASE WHEN b.typtype = 'c' AND NOT c.is_array THEN ARRAY(
                    SELECT pg_catalog.format_type(a.atttypid, a.atttypmod) FROM pg_catalog.pg_attribute a
                    WHERE a.attrelid = b.typrelid AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum
                ) END
         FROM chain c JOIN pg_catalog.pg_type t ON t.oid = c.original
         JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace
         JOIN pg_catalog.pg_type b ON b.oid = c.current
         WHERE b.typtype <> 'd' AND NOT (b.typtype = 'b' AND b.typcategory = 'A' AND b.typelem <> 0)",
        &[&oids],
    )?;
    let mut result = BTreeMap::new();
    for row in rows {
        let base_oid = row.try_get(3)?;
        let enum_labels: Option<Vec<String>> = row.try_get(4)?;
        let kind: String = row.try_get(6)?;
        if matches!(kind.as_str(), "c" | "e") || crate::scalar::supported(base_oid) {
            let field_names: Option<Vec<String>> = row.try_get(7)?;
            let field_types: Option<Vec<String>> = row.try_get(8)?;
            let fields = field_names.zip(field_types).map(|(names, types)| {
                names
                    .into_iter()
                    .zip(types)
                    .map(|(name, type_name)| ArgumentField { name, type_name })
                    .collect()
            });
            result.insert(
                row.try_get(0)?,
                ArgumentType {
                    schema: row.try_get(1)?,
                    name: row.try_get(2)?,
                    base_oid,
                    enum_labels,
                    array: row.try_get(5)?,
                    fields,
                },
            );
        }
    }
    Ok(result)
}

pub fn constructor_types(
    connection: &Connection,
    schema: &str,
) -> Result<Vec<crate::project::NamedType>, RequestError> {
    let mut client = connection
        .connect()
        .map_err(|error| connection.request_error(error, true))?;
    load_constructor_types(&mut client, schema)
        .map_err(|error| connection.request_error(error, false))
}

fn load_constructor_types(
    client: &mut Client,
    schema: &str,
) -> Result<Vec<crate::project::NamedType>, postgres::Error> {
    let rows = client.query(
        "SELECT t.oid, t.typname::text,
                CASE WHEN t.typarray <> 0 THEN pg_catalog.has_type_privilege(t.typarray, 'USAGE') ELSE false END
         FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace
         WHERE n.nspname = $1 AND t.typtype IN ('c', 'd', 'e')
           AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
           AND pg_catalog.has_type_privilege(t.oid, 'USAGE')
         ORDER BY t.typname",
        &[&schema],
    )?;
    let ids = rows
        .iter()
        .map(|row| row.get::<_, u32>(0))
        .collect::<Vec<_>>();
    let supported = argument_types(client, &ids)?;
    let mut result = Vec::new();
    for row in rows {
        if supported.contains_key(&row.get::<_, u32>(0)) {
            let ty = crate::project::NamedType {
                schema: schema.into(),
                name: row.try_get(1)?,
                array: false,
            };
            if ty.valid() {
                if row.try_get(2)? {
                    result.push(crate::project::NamedType {
                        array: true,
                        ..ty.clone()
                    });
                }
                result.push(ty);
            }
        }
    }
    Ok(result)
}

pub fn matches_signature(connection: &Connection, signature: &str, oid: u32) -> bool {
    connection
        .config()
        .connect(NoTls)
        .and_then(|mut client| {
            client.query_one(
                "SELECT COALESCE(pg_catalog.to_regprocedure($1)::oid = $2, false)",
                &[&signature, &oid],
            )
        })
        .and_then(|row| row.try_get(0))
        .unwrap_or(false)
}

pub(crate) fn plugin_configured(libraries: &str) -> bool {
    libraries.split(',').any(|name| {
        let name = name.trim().trim_matches('"');
        matches!(
            name.rsplit(['/', '\\']).next(),
            Some("plugin_debugger" | "plugin_debugger.so" | "plugin_debugger.dll")
        )
    })
}

fn debugger_setup(
    client: &mut impl postgres::GenericClient,
) -> Result<DebuggerSetup, postgres::Error> {
    let row = client.query_one(
        "SELECT EXISTS (SELECT FROM pg_catalog.pg_extension WHERE extname = 'pldbgapi'),
                EXISTS (SELECT FROM pg_catalog.pg_available_extensions WHERE name = 'pldbgapi'),
                (SELECT setting FROM pg_catalog.pg_settings WHERE name = 'shared_preload_libraries'),
                (SELECT pending_restart FROM pg_catalog.pg_settings WHERE name = 'shared_preload_libraries'),
                (SELECT rolsuper FROM pg_catalog.pg_roles WHERE rolname = current_user),
                pg_catalog.current_setting('transaction_read_only') = 'on',
                COALESCE((
                    SELECT pg_catalog.has_schema_privilege(e.extnamespace, 'USAGE')
                        AND EXISTS (
                            SELECT FROM pg_catalog.pg_proc AS p
                            WHERE p.pronamespace = e.extnamespace
                              AND p.proname = 'pldbg_oid_debug'
                              AND p.pronargs = 1 AND p.proargtypes[0] = 'pg_catalog.oid'::regtype
                              AND pg_catalog.has_function_privilege(p.oid, 'EXECUTE')
                        )
                    FROM pg_catalog.pg_extension e WHERE e.extname = 'pldbgapi'
                ), false)",
        &[],
    )?;
    Ok(DebuggerSetup {
        extension_created: row.try_get(0)?,
        package_available: row.try_get(1)?,
        preload: row
            .try_get::<_, Option<String>>(2)?
            .as_deref()
            .map(plugin_configured),
        restart_pending: row.try_get(3)?,
        superuser: row.try_get(4)?,
        read_only: row.try_get(5)?,
        api_allowed: row.try_get(6)?,
    })
}

pub fn enable_debugger(
    connection: &Connection,
    selection: &Selection,
) -> Result<Catalog, RequestError> {
    let mut client = connection
        .connect()
        .map_err(|err| connection.request_error(err, true))?;
    let mut transaction = client
        .transaction()
        .map_err(|err| enable_error(connection, err))?;
    let setup = debugger_setup(&mut transaction).map_err(|err| enable_error(connection, err))?;
    if !setup.extension_created {
        let problem = if !setup.package_available {
            Some(
                "На сервере нет файлов pldbgapi. Установите пакет pldebugger для этой версии PostgreSQL, затем обновите проверку.",
            )
        } else if setup.read_only {
            Some(
                "База доступна только для чтения. Подключитесь к основному серверу или попросите администратора включить запись.",
            )
        } else if !setup.superuser {
            Some(
                "Для установки pldbgapi нужна роль суперпользователя PostgreSQL. Попросите администратора включить расширение в этой базе.",
            )
        } else {
            None
        };
        if let Some(message) = problem {
            return Err(RequestError {
                message,
                detail: String::new(),
            });
        }
        // Установочные скрипты расширения запускаются в новой схеме без чужих объектов.
        transaction
            .batch_execute(
                "SET LOCAL lock_timeout = '4s';
             SET LOCAL statement_timeout = '20s';
             SET LOCAL search_path = pg_catalog;
             CREATE SCHEMA pgdebugger_tools;
             CREATE EXTENSION pldbgapi WITH SCHEMA pgdebugger_tools;",
            )
            .map_err(|err| enable_error(connection, err))?;
    }
    transaction
        .commit()
        .map_err(|err| enable_error(connection, err))?;
    load_catalog(&mut client, selection).map_err(|err| connection.request_error(err, false))
}

fn enable_error(connection: &Connection, error: postgres::Error) -> RequestError {
    let message = match error.code().map(SqlState::code) {
        Some("42501") => {
            "PostgreSQL отказал в установке. Попросите администратора проверить права роли."
        }
        Some("25006") => {
            "База доступна только для чтения. Подключитесь к основной базе с доступом на запись."
        }
        Some("58P01" | "0A000") => {
            "Сервер не смог загрузить файлы расширения. Проверьте, что пакет pldebugger установлен для этой версии PostgreSQL."
        }
        Some("42710" | "42P06" | "23505") => {
            "Схема pgdebugger_tools или расширение уже существует. Обновите проверку. Если pldbgapi ещё не включено, администратор может установить его в другую доверенную схему."
        }
        Some("55P03" | "57014" | "40P01" | "40001") => {
            "PostgreSQL не завершил установку вовремя. Дождитесь окончания других операций и повторите проверку."
        }
        _ => {
            "Не удалось подтвердить установку pldbgapi. Обновите проверку. Если проблема повторится, передайте администратору технические подробности."
        }
    };
    RequestError {
        message,
        detail: connection.diagnostic(error).render(),
    }
}
