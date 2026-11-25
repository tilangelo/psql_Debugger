use crate::breakpoints::SourceMap;
use crate::catalog::{Argument, Connection, RoutineKind};
use crate::outcome::{
    CallOutcome, ResultCell, ResultPreview, ResultTable, RollbackOutcome, SessionOutcome,
};
pub use pg_debugger::pldbg::Variable;
use pg_debugger::pldbg::{Breakpoint, DebugEvent, PldbgClient, StackFrame};
use postgres::fallible_iterator::FallibleIterator;
use postgres::types::{ToSql, Type};
use postgres::{CancelToken, Client, NoTls, Row};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Call {
    pub oid: u32,
    kind: RoutineKind,
    invocation: String,
    table_result: bool,
    values: Vec<Option<String>>,
    expected_definition: Option<String>,
    default_target: Option<DefaultCall>,
    nested_breakpoints: Vec<NestedPlan>,
    trigger: Option<crate::trigger::TriggerCall>,
}

#[derive(Clone)]
pub struct NestedPlan {
    oid: u32,
    name: String,
    definition: String,
    lines: BTreeMap<i32, i32>,
}

impl NestedPlan {
    pub fn new(
        oid: u32,
        name: String,
        definition: String,
        source: &str,
        lines: Vec<i32>,
    ) -> Result<Self, &'static str> {
        Ok(Self {
            oid,
            name,
            definition,
            lines: resolve_nested_lines(source, lines)?,
        })
    }
}

struct DefaultCall {
    schema: String,
    name: String,
    count: i32,
    named: Vec<String>,
}

impl Call {
    pub fn new(
        oid: u32,
        kind: RoutineKind,
        schema: &str,
        name: &str,
        arguments: &[Argument],
        values: Vec<Option<String>>,
    ) -> Result<Self, String> {
        Self::with_default_arguments(oid, kind, schema, name, arguments, values, &BTreeSet::new())
    }

    pub fn with_default_arguments(
        oid: u32,
        kind: RoutineKind,
        schema: &str,
        name: &str,
        arguments: &[Argument],
        values: Vec<Option<String>>,
        defaults: &BTreeSet<usize>,
    ) -> Result<Self, String> {
        if arguments.len() != values.len() {
            return Err("Выберите объект заново, чтобы обновить аргументы.".into());
        }
        if defaults.iter().any(|&index| {
            !arguments.get(index).is_some_and(|arg| {
                arg.has_default
                    && matches!(arg.mode.as_str(), "IN" | "INOUT")
                    && values[index].is_none()
            })
        }) {
            return Err("Значение по умолчанию доступно только для соответствующего входного аргумента. Уберите также введённое значение.".into());
        }
        let mut params = Vec::new();
        let mut expressions = Vec::new();
        let mut named = false;
        let mut argument_names = Vec::new();
        for (index, (arg, value)) in arguments.iter().zip(values).enumerate() {
            if kind == RoutineKind::Function && matches!(arg.mode.as_str(), "OUT" | "TABLE") {
                continue;
            }
            if defaults.contains(&index) {
                named = true;
                continue;
            }
            let cast_type = if let Some(ty) = &arg.custom_type {
                Some(format!("{}.{}", quote(&ty.schema), quote(&ty.name)))
            } else {
                crate::scalar::call_type(arg.type_oid)
                    .map(|ty| format!("pg_catalog.{}", quote(ty.name())))
            };
            let Some(cast_type) = cast_type else {
                return Err(format!(
                    "Аргумент {}: тип {} пока не поддерживается.",
                    index + 1,
                    arg.type_name
                ));
            };
            let expression = if arg.mode == "OUT" {
                format!("NULL::{cast_type}")
            } else {
                if !arg.accepts_input()
                    || value
                        .as_deref()
                        .is_some_and(|text| !arg.value_is_valid(text))
                {
                    return Err(format!(
                        "Аргумент {}: {}",
                        index + 1,
                        value
                            .as_deref()
                            .and_then(|v| arg.value_error(v))
                            .unwrap_or("Входной тип пока не поддерживается.")
                    ));
                }
                params.push(value);
                format!("(${}::pg_catalog.text)::{cast_type}", params.len())
            };
            if named {
                let argument_name = arg.name.as_deref().ok_or_else(||
                    "После пропущенного аргумента есть аргумент без имени. Передайте значения для обоих или пропустите их, если доступны значения по умолчанию.".to_string()
                )?;
                argument_names.push(argument_name.to_owned());
                expressions.push(format!("{} => {expression}", quote(argument_name)));
            } else {
                expressions.push(expression);
            }
        }
        let invocation = format!(
            "{}.{}({})",
            quote(schema),
            quote(name),
            expressions.join(", ")
        );
        Ok(Self {
            oid,
            kind,
            invocation,
            table_result: false,
            values: params,
            expected_definition: None,
            nested_breakpoints: Vec::new(),
            trigger: None,
            default_target: (!defaults.is_empty()).then(|| DefaultCall {
                schema: schema.into(),
                name: name.into(),
                count: expressions.len() as i32,
                named: argument_names,
            }),
        })
    }

    // Полное определение включает имя, параметры и настройки объекта.
    pub fn with_definition(mut self, definition: String) -> Self {
        self.expected_definition = Some(definition);
        self
    }

    pub fn with_table_result(mut self, enabled: bool) -> Self {
        self.table_result = enabled && self.kind == RoutineKind::Function;
        self
    }

    pub fn with_nested_breakpoints(mut self, plan: Vec<NestedPlan>) -> Self {
        self.nested_breakpoints = plan;
        self
    }

    pub fn with_trigger(mut self, trigger: Option<crate::trigger::TriggerCall>) -> Self {
        self.trigger = trigger;
        self
    }
}

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

pub enum Command {
    Step,
    StepInto,
    StepOut,
    SelectFrame {
        level: i32,
    },
    Continue,
    Breakpoint {
        line: i32,
        enabled: bool,
    },
    NestedBreakpoints {
        oid: u32,
        lines: Vec<i32>,
        enabled: bool,
    },
    Stop,
}

pub enum Event {
    BreakpointRejected {
        line: i32,
        enabled: bool,
        message: String,
    },
    NestedBreakpointsChanged {
        oid: u32,
        name: Option<String>,
        lines: Vec<i32>,
        enabled: bool,
        warning: Option<String>,
    },
    Paused {
        position: Breakpoint,
        stack: Vec<StackFrame>,
        variables: Vec<Variable>,
        source: Option<String>,
    },
    FrameSelected {
        level: i32,
        variables: Vec<Variable>,
        source: Option<String>,
    },
    Warning(String),
    Finished(SessionOutcome),
}

pub struct Session {
    commands: Sender<Command>,
    pub events: Receiver<Event>,
    stop: Arc<Stop>,
}

#[derive(Default)]
struct Stop {
    requested: AtomicBool,
    finished: AtomicBool,
    token: Mutex<Option<CancelToken>>,
}

impl Stop {
    fn requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }

    fn cancel_query(&self) {
        let token = self.token.lock().ok().and_then(|token| token.clone());
        if let Some(token) = token {
            let _ = token.cancel_query(NoTls);
        }
    }
}

impl Session {
    pub fn start(
        connection: Connection,
        call: Call,
        points: BTreeSet<i32>,
    ) -> Result<Self, std::io::Error> {
        let (commands, command_rx) = channel();
        let (event_tx, events) = channel();
        let stop = Arc::new(Stop::default());
        let worker_stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("plpgsql-debugger".into())
            .spawn(move || {
                let mut report = run(
                    &connection,
                    call,
                    points,
                    &command_rx,
                    &event_tx,
                    &worker_stop,
                );
                if let Ok(mut token) = worker_stop.token.lock() {
                    token.take();
                }
                report.cancellation_requested = worker_stop.requested();
                worker_stop.finished.store(true, Ordering::Release);
                let _ = event_tx.send(Event::Finished(report));
            })?;
        Ok(Self {
            commands,
            events,
            stop,
        })
    }

    pub fn command(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn stop(&self) {
        if self.stop.finished.load(Ordering::Acquire) {
            return;
        }
        if !self.stop.requested.swap(true, Ordering::AcqRel) {
            self.command(Command::Stop);
            let stop = Arc::clone(&self.stop);
            let _ = std::thread::Builder::new().spawn(move || stop.cancel_query());
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

enum TargetEvent {
    Waiting(i32),
    Notice(String),
    Done(SessionOutcome),
}

fn run(
    connection: &Connection,
    call: Call,
    points: BTreeSet<i32>,
    commands: &Receiver<Command>,
    events: &Sender<Event>,
    stop: &Stop,
) -> SessionOutcome {
    let oid = call.oid;
    let (target_tx, target_rx) = channel();
    let setup = (|| {
        let mut client = connection.config().connect(NoTls)?;
        // Ищем расширение в выбранной базе.
        let schema: Option<String> = client
            .query_opt(
                "SELECT n.nspname::text FROM pg_catalog.pg_extension e
             JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace
             WHERE e.extname = 'pldbgapi'",
                &[],
            )?
            .map(|row| row.try_get(0))
            .transpose()?;
        let Some(schema) = schema else {
            return Ok(None);
        };
        let schema = quote(&schema);
        let path = format!("pg_catalog, {schema}");
        client.query_one(
            "SELECT pg_catalog.set_config('search_path', $1, false)",
            &[&path],
        )?;
        Ok::<_, postgres::Error>(Some((PldbgClient::from_client(client), schema)))
    })();
    let (proxy, schema) = match setup {
        Ok(Some(setup)) => setup,
        Ok(None) => {
            return SessionOutcome::not_started(
                "В выбранной базе не установлено расширение pldbgapi.",
            );
        }
        Err(error) => return SessionOutcome::setup_failed(connection.diagnostic(error)),
    };
    // Перечитываем тело: код мог поменяться.
    let source = match connection.config().connect(NoTls).and_then(|mut client| {
        client.query_opt("SELECT prosrc, pg_catalog.pg_get_functiondef(oid) FROM pg_catalog.pg_proc WHERE oid = $1", &[&oid])
    }) {
        Ok(Some(row)) if call.expected_definition.as_ref().is_none_or(|expected| row.get::<_, String>(1) == *expected) => SourceMap::new(&row.get::<_, String>(0)),
        Ok(_) => {
            return SessionOutcome::new(CallOutcome::DefinitionChanged, RollbackOutcome::NotNeeded);
        },
        Err(error) => return SessionOutcome::setup_failed(connection.diagnostic(error)),
    };
    let points = match points
        .into_iter()
        .map(|line| source.resolve(line))
        .collect::<Result<BTreeSet<_>, _>>()
    {
        Ok(points) => points,
        Err(message) => {
            return SessionOutcome::not_started(format!(
                "Не удалось задать точку остановки: {message} Вызов не выполнялся."
            ));
        }
    };
    let notice_tx = target_tx.clone();
    let notice_count = AtomicUsize::new(0);
    let notice_settings = connection.clone();
    let client = connection
        .config()
        .options("-c client_min_messages=notice")
        .notice_callback(move |notice| {
            let message = notice.message();
            let event = match message
                .strip_prefix("PLDBGBREAK:")
                .and_then(|slot| slot.parse().ok())
            {
                Some(slot) => TargetEvent::Waiting(slot),
                None => {
                    let count = notice_count.fetch_add(1, Ordering::Relaxed);
                    if count > 100 {
                        return;
                    }
                    TargetEvent::Notice(if count == 100 {
                        "Остальные сообщения PostgreSQL скрыты: достигнут лимит 100.".into()
                    } else {
                        clip_text(&notice_settings.redact(message), 4096).0
                    })
                }
            };
            let _ = notice_tx.send(event);
        })
        .connect(NoTls);
    let mut target = match client {
        Ok(client) => client,
        Err(error) => return SessionOutcome::setup_failed(connection.diagnostic(error)),
    };
    if let Ok(mut token) = stop.token.lock() {
        *token = Some(target.cancel_token());
    }
    if stop.requested() {
        return SessionOutcome::not_started("Отладка остановлена до вызова.");
    }
    let settings = connection.clone();
    let nested_breakpoints = call.nested_breakpoints.clone();
    let start = std::thread::Builder::new()
        .name("plpgsql-target".into())
        .spawn(move || {
            let report = execute(&mut target, &settings, &schema, &call);
            drop(target);
            let _ = target_tx.send(TargetEvent::Done(report));
        });
    let worker = match start {
        Ok(worker) => worker,
        Err(error) => {
            return SessionOutcome::not_started(format!("Не удалось начать вызов: {error}"));
        }
    };
    let mut report = supervise(
        proxy,
        &target_rx,
        oid,
        points,
        &source,
        &nested_breakpoints,
        commands,
        events,
        stop,
        connection,
    );
    // Finished отправляется только после выхода целевого потока и закрытия соединения.
    if worker.join().is_err() {
        report.call = CallOutcome::Unknown("Рабочий поток вызова аварийно завершился.".into());
        report.rollback =
            RollbackOutcome::Unconfirmed("Соединение вызова завершилось без отчёта.".into());
    }
    report
}

fn supervise(
    mut proxy: PldbgClient,
    target_rx: &Receiver<TargetEvent>,
    oid: u32,
    points: BTreeSet<i32>,
    source: &SourceMap,
    prepared: &[NestedPlan],
    commands: &Receiver<Command>,
    events: &Sender<Event>,
    stop: &Stop,
    connection: &Connection,
) -> SessionOutcome {
    let mut notices = Vec::new();
    let started = Instant::now();
    let mut warned = false;
    let slot = loop {
        match target_rx.recv_timeout(Duration::from_millis(150)) {
            Ok(TargetEvent::Waiting(slot)) => break slot,
            Ok(TargetEvent::Notice(notice)) => notices.push(notice),
            Ok(TargetEvent::Done(mut report)) => {
                if matches!(report.call, CallOutcome::Succeeded(_)) && !stop.requested() {
                    report
                        .notes
                        .push("Вызов завершился до первой остановки.".into());
                }
                return with_notices(report, notices, stop.requested());
            }
            Err(RecvTimeoutError::Timeout) => {
                if !warned && started.elapsed() >= Duration::from_secs(20) {
                    warned = true;
                    let _ = events.send(Event::Warning("Ожидаем первую остановку. Возможно, выполняется инициализация переменных или запрос ждёт блокировку. Можно нажать «Остановить».".into()));
                }
                if stop.requested() {
                    stop.cancel_query();
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return with_notices(
                    SessionOutcome::unknown("Связь с потоком вызова потеряна."),
                    notices,
                    stop.requested(),
                );
            }
        }
    };
    let proxy_result = debug(
        &mut proxy,
        slot,
        oid,
        points,
        source,
        prepared,
        commands,
        events,
        stop,
        target_rx,
        &mut notices,
    );
    drop(proxy);
    let proxy_result = match proxy_result {
        Ok(Some(report)) => return with_notices(report, notices, stop.requested()),
        other => other,
    };
    let proxy_error = proxy_result.err().map(|err| connection.diagnostic(err));
    if proxy_error.is_some() {
        stop.cancel_query();
    }
    let mut warned = false;
    let waiting = Instant::now();
    let report = loop {
        match target_rx.recv_timeout(Duration::from_secs(8)) {
            Ok(TargetEvent::Done(report)) => break report,
            Ok(TargetEvent::Notice(notice)) => notices.push(notice),
            Ok(TargetEvent::Waiting(_)) => {}
            Err(RecvTimeoutError::Timeout) => {
                stop.cancel_query();
                if !warned && waiting.elapsed() >= Duration::from_secs(8) {
                    warned = true;
                    let _ = events.send(Event::Warning("Ожидаем завершение вызова и подтверждение отката. Новый запуск пока недоступен.".into()));
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                break SessionOutcome::unknown("Связь с потоком вызова потеряна.");
            }
        }
    };
    let mut report = report;
    report.debugger_error = proxy_error.filter(|_| !stop.requested());
    with_notices(report, notices, stop.requested())
}

fn with_notices(mut report: SessionOutcome, notices: Vec<String>, stopped: bool) -> SessionOutcome {
    report.cancellation_requested = stopped;
    report.notices = notices;
    report
}

fn execute(
    client: &mut Client,
    settings: &Connection,
    extension: &str,
    call: &Call,
) -> SessionOutcome {
    let mut transaction = match client.transaction() {
        Ok(transaction) => transaction,
        Err(error) => return SessionOutcome::setup_failed(settings.diagnostic(error)),
    };
    let result = (|| -> Result<CallOutcome, postgres::Error> {
        // Блокировка удерживается до отката и не даёт изменить таблицу или её триггеры
        // между проверкой и тестовым DML.
        if let Some(trigger) = &call.trigger {
            if !trigger.validate(&mut transaction, call.oid)? {
                return Ok(CallOutcome::NotStarted(
                    "Таблица или триггер изменились. Обновите каталог и повторите запуск.".into(),
                ));
            }
        }
        if let Some(expected) = &call.expected_definition {
            let actual = transaction.query_opt(
                "SELECT pg_catalog.pg_get_functiondef(oid) FROM pg_catalog.pg_proc WHERE oid = $1",
                &[&call.oid],
            )?;
            if actual.is_none_or(|row| row.get::<_, String>(0) != *expected) {
                return Ok(CallOutcome::DefinitionChanged);
            }
        }
        for group in &call.nested_breakpoints {
            let actual = transaction.query_opt(
                "SELECT pg_catalog.pg_get_functiondef(oid) FROM pg_catalog.pg_proc WHERE oid = $1 AND prokind IN ('f','p')",
                &[&group.oid],
            )?;
            if actual.is_none_or(|row| row.get::<_, String>(0) != group.definition) {
                return Ok(CallOutcome::NotStarted(format!(
                    "Определение вложенного объекта {} изменилось. Обновите точки перед запуском.",
                    group.name
                )));
            }
        }
        if let Some(target) = &call.default_target {
            // При пропуске аргументов PostgreSQL может выбрать другую перегрузку.
            let has_overload: bool = transaction
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_proc p
                   JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
                   CROSS JOIN LATERAL (SELECT CASE WHEN $6 THEN
                     COALESCE(pg_catalog.cardinality(p.proallargtypes), p.pronargs)
                     ELSE p.pronargs END AS total) AS args
                   WHERE n.nspname = $1 AND p.proname = $2 AND p.oid <> $3
                     AND $4 >= args.total - p.pronargdefaults - (p.provariadic <> 0)::int
                     AND (p.provariadic <> 0 OR $4 <= args.total)
                     AND COALESCE(p.proargnames, ARRAY[]::text[]) @> $5::text[])",
                    &[
                        &target.schema,
                        &target.name,
                        &call.oid,
                        &target.count,
                        &target.named,
                        &(call.kind == RoutineKind::Procedure),
                    ],
                )?
                .try_get(0)?;
            if has_overload {
                return Ok(CallOutcome::NotStarted(
                    "После пропуска аргументов есть подходящие по форме перегрузки. Передайте значения всех аргументов для выбранной сигнатуры.".into(),
                ));
            }
        }
        let sql = format!("SELECT {extension}.pldbg_oid_debug($1)");
        if let Some(trigger) = &call.trigger {
            transaction.query_one(&sql, &[&call.oid])?;
            let rows = trigger.execute(&mut transaction)?;
            return Ok(CallOutcome::Succeeded(ResultPreview {
                text: format!("{}. Затронуто строк: {rows}.", trigger.operation().label()),
                scalar: None,
                total_rows: rows,
                shown_rows: 0,
                truncated: false,
                table: None,
            }));
        }
        let invocation = &call.invocation;
        let (query, columns) = if call.table_result {
            // Prepare даёт имена столбцов даже при пустом результате и не вызывает функцию.
            let shape = transaction.prepare(&format!("SELECT * FROM {invocation}"))?;
            let columns: Vec<_> = shape
                .columns()
                .iter()
                .map(|column| column.name().to_owned())
                .collect();
            let names: Vec<_> = (0..columns.len())
                .map(|index| format!("c{index}"))
                .collect();
            let projection = names
                .iter()
                .map(|name| {
                    format!(
                        "pg_catalog.left(preview.{name}::pg_catalog.text, {})",
                        MAX_CELL + 1
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let aliases = if names.is_empty() {
                String::new()
            } else {
                format!("({})", names.join(", "))
            };
            (
                format!("SELECT {projection} FROM {invocation} AS preview{aliases}"),
                Some(columns),
            )
        } else {
            let query = match call.kind {
                RoutineKind::Function => format!(
                    "SELECT pg_catalog.left(pg_catalog.to_jsonb(result)::text, {}) FROM {invocation} AS result",
                    MAX_CELL + 1
                ),
                RoutineKind::Procedure => format!("CALL {invocation}"),
            };
            (query, None)
        };
        let params: Vec<&(dyn ToSql + Sync)> = call.values.iter().map(|value| value as _).collect();
        transaction.query_one(&sql, &[&call.oid])?;
        let mut rows = transaction.query_raw(&query, params.iter().copied())?;
        let result = if let Some(columns) = columns {
            format_table(columns, &mut rows)?
        } else if call.kind == RoutineKind::Procedure {
            // Дочитываем CALL перед переводом значений OUT в текст.
            let row = rows.next()?;
            if row.is_some() {
                while rows.next()?.is_some() {}
            }
            drop(rows);
            transaction.batch_execute("SET LOCAL DateStyle = 'ISO, YMD'")?;
            let mut captured = postgres::fallible_iterator::convert(row.into_iter().map(Ok));
            format_rows(call.kind, &mut captured, Some(&mut transaction))?
        } else {
            format_rows(call.kind, &mut rows, None)?
        };
        Ok(CallOutcome::Succeeded(result))
    })();
    let call = result.unwrap_or_else(|error| CallOutcome::Failed(settings.diagnostic(error)));
    let rollback = match transaction.rollback() {
        Ok(()) => RollbackOutcome::Confirmed,
        Err(error) => RollbackOutcome::Unconfirmed(settings.diagnostic(error).render()),
    };
    SessionOutcome::new(call, rollback)
}

const MAX_ROWS: usize = 100;
const MAX_TEXT: usize = 64 * 1024;
const MAX_CELL: usize = 16 * 1024;

fn format_table(
    columns: Vec<String>,
    rows: &mut impl FallibleIterator<Item = Row, Error = postgres::Error>,
) -> Result<ResultPreview, postgres::Error> {
    let mut shown = Vec::new();
    let mut total_rows = 0;
    let mut budget = MAX_TEXT;
    let mut shortened = false;
    let row_overhead = (columns.len() * std::mem::size_of::<ResultCell>()).max(1);
    while let Some(row) = rows.next()? {
        total_rows += 1;
        if shown.len() >= MAX_ROWS || budget <= row_overhead {
            continue;
        }
        budget -= row_overhead;
        let mut cells = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            let raw = row.try_get::<_, Option<&str>>(index)?;
            let (value, truncated) = match raw {
                Some(raw) => {
                    let (value, cut) = clip_text(raw, budget.min(MAX_CELL));
                    budget -= value.len();
                    (Some(value), cut)
                }
                None => (None, false),
            };
            shortened |= truncated;
            cells.push(ResultCell { value, truncated });
        }
        shown.push(cells);
    }
    Ok(ResultPreview {
        text: format!("Строк в результате: {total_rows}."),
        scalar: None,
        total_rows,
        shown_rows: shown.len(),
        truncated: shortened || (shown.len() as u64) < total_rows,
        table: Some(ResultTable {
            columns,
            rows: shown,
        }),
    })
}

fn format_rows(
    kind: RoutineKind,
    rows: &mut impl FallibleIterator<Item = Row, Error = postgres::Error>,
    mut transaction: Option<&mut postgres::Transaction<'_>>,
) -> Result<ResultPreview, postgres::Error> {
    let mut lines = Vec::new();
    let mut count = 0u64;
    let mut used = 0;
    let mut truncated = false;
    while let Some(row) = rows.next()? {
        count += 1;
        if lines.len() >= MAX_ROWS || used >= MAX_TEXT {
            truncated = true;
            continue;
        }
        let mut values = Vec::new();
        for (index, column) in row.columns().iter().enumerate() {
            let value = if kind == RoutineKind::Function {
                text_value::<String>(&row, index)
            } else {
                match *column.type_() {
                    Type::BOOL => text_value::<bool>(&row, index),
                    Type::INT2 => text_value::<i16>(&row, index),
                    Type::INT4 => text_value::<i32>(&row, index),
                    Type::INT8 => text_value::<i64>(&row, index),
                    Type::FLOAT4 => text_value::<f32>(&row, index),
                    Type::FLOAT8 => text_value::<f64>(&row, index),
                    Type::TEXT | Type::VARCHAR => text_value::<String>(&row, index),
                    ref ty if crate::output_value::supported(ty) => crate::output_value::render(
                        transaction.as_deref_mut().expect("procedure transaction"),
                        &row,
                        index,
                    )?,
                    _ => format!("<{}>", column.type_().name()),
                }
            };
            let rendered = if kind == RoutineKind::Procedure {
                format!("{} = {value}", column.name())
            } else {
                value
            };
            let (part, cut) = clip_text(&rendered, (MAX_TEXT - used).min(MAX_CELL));
            used += part.len();
            truncated |= cut;
            values.push(part);
        }
        lines.push(values.join(" | "));
    }
    if count == 0 {
        return Ok(ResultPreview {
            text: if kind == RoutineKind::Procedure {
                "Процедура выполнена.".into()
            } else {
                "Функция выполнена. Строк результата нет.".into()
            },
            scalar: None,
            total_rows: 0,
            shown_rows: 0,
            truncated: false,
            table: None,
        });
    }
    let (content, cut) = clip_text(&lines.join("\n"), MAX_TEXT);
    let footer = if truncated || cut {
        format!(
            "\nПоказано {} из {count} строк; объём текста ограничен.",
            lines.len()
        )
    } else {
        String::new()
    };
    Ok(ResultPreview {
        text: format!("Результат:\n{content}{footer}"),
        scalar: (kind == RoutineKind::Function && count == 1).then(|| {
            let (mut value, shortened) = clip_text(&content, 120);
            if shortened || truncated || cut {
                value.push('…');
            }
            value
        }),
        total_rows: count,
        shown_rows: lines.len(),
        truncated: truncated || cut,
        table: None,
    })
}

fn clip_text(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.into(), false);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

fn text_value<'a, T: postgres::types::FromSql<'a> + ToString>(
    row: &'a Row,
    index: usize,
) -> String {
    match row.try_get::<_, Option<T>>(index) {
        Ok(Some(value)) => value.to_string(),
        Ok(None) => "NULL".into(),
        Err(_) => "<значение недоступно>".into(),
    }
}

fn debug(
    proxy: &mut PldbgClient,
    slot: i32,
    oid: u32,
    points: BTreeSet<i32>,
    source: &SourceMap,
    prepared: &[NestedPlan],
    commands: &Receiver<Command>,
    events: &Sender<Event>,
    stop: &Stop,
    target: &Receiver<TargetEvent>,
    notices: &mut Vec<String>,
) -> Result<Option<SessionOutcome>, postgres::Error> {
    let handle = proxy.attach(slot)?;
    // Ждём первую паузу.
    let first = proxy.wait_for_breakpoint(handle)?;
    if stop.requested() {
        return proxy.abort(handle).map(|()| None);
    }
    let top_line = proxy
        .get_stack(handle)?
        .first()
        .map(|frame| frame.line_number);
    let mut root_points = BTreeSet::new();
    for line in points
        .iter()
        .filter(|line| top_line.is_none_or(|entry| **line > entry))
    {
        if update_breakpoint(proxy, handle, oid, *line, true, top_line, events)? {
            root_points.insert(*line);
        }
    }
    let mut nested_points = BTreeMap::new();
    // Корневой кадр ещё на входе. Ставим точки в его сессии до продолжения.
    for group in prepared {
        for (&line, &api_line) in &group.lines {
            if stop.requested() {
                return proxy.abort(handle).map(|()| None);
            }
            if !proxy.update_local_breakpoint(handle, group.oid, api_line, true)? {
                let _ = events.send(Event::Warning(format!(
                    "PostgreSQL не подтвердил точку {} на строке {line}. Вызов остановлен.",
                    group.name
                )));
                return proxy.abort(handle).map(|()| None);
            }
            nested_points.insert((group.oid, line), api_line);
        }
        let _ = events.send(Event::NestedBreakpointsChanged {
            oid: group.oid,
            name: Some(group.name.clone()),
            lines: group.lines.keys().copied().collect(),
            enabled: true,
            warning: None,
        });
    }
    // Без точек оставляем паузу на входе. С одними вложенными ждём их срабатывания.
    let mut state = if (points.is_empty() && prepared.is_empty())
        || (!points.is_empty()
            && top_line.is_none_or(|line| points.range(..=line).next().is_some()))
    {
        DebugEvent::Paused(first)
    } else {
        proxy.continue_target(handle)?
    };
    while let DebugEvent::Paused(position) = state {
        if stop.requested() {
            return proxy.abort(handle).map(|()| None);
        }
        let stack = proxy.get_stack(handle)?;
        let frame_source = |proxy: &mut PldbgClient, frame: &StackFrame| {
            // Корневое тело уже загружено. Для вложенного берём исходник из целевого процесса.
            if frame.function_oid == oid {
                None
            } else {
                proxy
                    .frame_source(handle, frame.function_oid)
                    .ok()
                    .flatten()
            }
        };
        let mut inspected_oid = stack.first().map(|frame| frame.function_oid);
        let mut inspected_source = stack.first().and_then(|frame| frame_source(proxy, frame));
        let variables = proxy.get_variables(handle)?;
        if events
            .send(Event::Paused {
                position,
                stack: stack.clone(),
                variables,
                source: inspected_source.clone(),
            })
            .is_err()
        {
            return proxy.abort(handle).map(|()| None);
        }
        state = loop {
            if let Some(report) = poll_target(target, notices) {
                return Ok(Some(report));
            }
            match commands.recv_timeout(Duration::from_millis(150)) {
                Ok(Command::Step) if !stop.requested() => break proxy.step_over(handle)?,
                Ok(Command::StepInto) if !stop.requested() => break proxy.step_into(handle)?,
                Ok(Command::StepOut) if !stop.requested() => {
                    if stack.len() > 1 {
                        // Берём глубину выполняемого кадра, даже если пользователь смотрит родителя.
                        break step_out(proxy, handle, stack.len(), stop, events, |frame| {
                            if frame.function_oid == oid {
                                root_points.contains(&frame.line_number)
                            } else {
                                nested_points.contains_key(&(frame.function_oid, frame.line_number))
                            }
                        })?;
                    }
                    let _ = events.send(Event::Warning(
                        "У выполняемого кадра нет вызывающего кадра PL/pgSQL.".into(),
                    ));
                }
                Ok(Command::SelectFrame { level }) if !stop.requested() => {
                    if let Some(frame) = stack.iter().find(|frame| frame.level == level) {
                        proxy.select_frame(handle, level)?;
                        let variables = proxy.get_variables(handle)?;
                        inspected_oid = Some(frame.function_oid);
                        inspected_source = frame_source(proxy, frame);
                        let _ = events.send(Event::FrameSelected {
                            level,
                            variables,
                            source: inspected_source.clone(),
                        });
                    }
                }
                Ok(Command::Continue) if !stop.requested() => {
                    break proxy.continue_target(handle)?;
                }
                Ok(Command::Breakpoint { line, enabled }) if !stop.requested() => {
                    match source.resolve(line) {
                        Ok(actual) => {
                            if update_breakpoint(
                                proxy, handle, oid, actual, enabled, top_line, events,
                            )? {
                                if enabled {
                                    root_points.insert(actual);
                                } else {
                                    root_points.remove(&actual);
                                }
                            }
                        }
                        Err(message) => {
                            let _ = events.send(Event::BreakpointRejected {
                                line,
                                enabled,
                                message: message.into(),
                            });
                        }
                    }
                }
                Ok(Command::NestedBreakpoints {
                    oid,
                    lines,
                    enabled,
                }) if !stop.requested() => {
                    let source = inspected_source
                        .as_deref()
                        .filter(|_| inspected_oid == Some(oid));
                    let result = change_nested_points(
                        proxy,
                        handle,
                        oid,
                        lines,
                        enabled,
                        source,
                        &mut nested_points,
                    )?;
                    let _ = events.send(result);
                }
                Err(RecvTimeoutError::Timeout) => continue,
                _ => return proxy.abort(handle).map(|()| None),
            }
        };
    }
    Ok(None)
}

fn step_out(
    proxy: &mut PldbgClient,
    handle: i32,
    start_depth: usize,
    stop: &Stop,
    events: &Sender<Event>,
    is_breakpoint: impl Fn(&StackFrame) -> bool,
) -> Result<DebugEvent, postgres::Error> {
    // Ограничиваем серию запросов к плагину. В длинном цикле можно продолжить с новой паузы.
    const MAX_STEPS: usize = 16;
    let mut remaining = MAX_STEPS;
    loop {
        let event = proxy.step_over(handle)?;
        if stop.requested() || matches!(event, DebugEvent::ChannelClosed) {
            return Ok(event);
        }
        let stack = proxy.get_stack(handle)?;
        if stop.requested()
            || stack.len() < start_depth
            || stack.first().is_some_and(&is_breakpoint)
        {
            return Ok(event);
        }
        remaining -= 1;
        if remaining == 0 {
            let _ = events.send(Event::Warning(format!(
                "После {MAX_STEPS} шагов возврат ещё не произошёл. Можно повторить «Шаг наружу»."
            )));
            return Ok(event);
        }
    }
}

fn resolve_nested_lines(source: &str, lines: Vec<i32>) -> Result<BTreeMap<i32, i32>, &'static str> {
    let map = SourceMap::new(source);
    let entry = map.entry_line();
    lines
        .into_iter()
        .map(|line| {
            map.resolve(line)
                .map(|actual| (actual, if entry == Some(actual) { -1 } else { actual }))
        })
        .collect()
}

fn change_nested_points(
    proxy: &mut PldbgClient,
    handle: i32,
    oid: u32,
    lines: Vec<i32>,
    enabled: bool,
    source: Option<&str>,
    installed: &mut BTreeMap<(u32, i32), i32>,
) -> Result<Event, postgres::Error> {
    let mut edits = Vec::new();
    let mut warning = None;
    if enabled {
        let resolved = source
            .ok_or("Выберите кадр с доступным исходным кодом.")
            .and_then(|source| resolve_nested_lines(source, lines));
        match resolved {
            Ok(resolved) => edits.extend(
                resolved
                    .into_iter()
                    .filter(|(line, _)| !installed.contains_key(&(oid, *line))),
            ),
            Err(message) => warning = Some(message.into()),
        }
    } else {
        edits.extend(
            lines
                .into_iter()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter_map(|line| {
                    installed
                        .get(&(oid, line))
                        .map(|&api_line| (line, api_line))
                }),
        );
    }
    let mut changed = Vec::new();
    for (line, api_line) in edits {
        if proxy.update_local_breakpoint(handle, oid, api_line, enabled)? {
            if enabled {
                installed.insert((oid, line), api_line);
            } else {
                installed.remove(&(oid, line));
            }
            changed.push(line);
        } else {
            warning = Some(format!(
                "PostgreSQL не подтвердил изменение точки на строке {line}."
            ));
        }
    }
    Ok(Event::NestedBreakpointsChanged {
        oid,
        name: None,
        lines: changed,
        enabled,
        warning,
    })
}

fn poll_target(
    target: &Receiver<TargetEvent>,
    notices: &mut Vec<String>,
) -> Option<SessionOutcome> {
    loop {
        match target.try_recv() {
            Ok(TargetEvent::Done(report)) => return Some(report),
            Ok(TargetEvent::Notice(message)) => notices.push(message),
            Ok(TargetEvent::Waiting(_)) => {}
            Err(std::sync::mpsc::TryRecvError::Empty) => return None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Some(SessionOutcome::unknown("Связь с потоком вызова потеряна."));
            }
        }
    }
}

fn update_breakpoint(
    proxy: &mut PldbgClient,
    handle: i32,
    oid: u32,
    line: i32,
    enabled: bool,
    entry: Option<i32>,
    events: &Sender<Event>,
) -> Result<bool, postgres::Error> {
    // Первому оператору в протоколе плагина соответствует специальная точка -1.
    let api_line = if entry == Some(line) { -1 } else { line };
    let confirmed = proxy.update_local_breakpoint(handle, oid, api_line, enabled)?;
    if !confirmed {
        let _ = events.send(Event::BreakpointRejected { line, enabled, message: format!(
            "PostgreSQL не подтвердил изменение точки на строке {line}. Перезапусти отладку и задай её до вызова."
        ) });
    }
    Ok(confirmed)
}
