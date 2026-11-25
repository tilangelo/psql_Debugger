//! Предпросмотр DDL с откатом и отдельное применение проверенного проекта.
use crate::{
    catalog::Connection,
    project::{Generated, Kind, Project},
};
use postgres::{GenericClient, NoTls};

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub oid: u32,
    pub revision: String,
    pub definition: String,
}
pub struct Prepared {
    pub generated: Generated,
    pub before: Option<Snapshot>,
    pub after: String,
    type_oids: Vec<u32>,
}

fn check_named_types(
    client: &mut impl GenericClient,
    connection: &Connection,
    project: &Project,
) -> Result<Vec<u32>, ChangeError> {
    let names = project.named_types();
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let rows = client
        .query(
            "SELECT name, oid::oid, pg_catalog.has_type_privilege(oid, 'USAGE')
         FROM unnest($1::text[]) WITH ORDINALITY AS types(name, position)
         CROSS JOIN LATERAL (SELECT pg_catalog.to_regtype(name) AS oid) AS resolved
         ORDER BY position",
            &[&names],
        )
        .map_err(|e| ChangeError::Database(connection.diagnostic(e)))?;
    rows.into_iter()
        .map(|row| {
            let name: &str = row.get(0);
            let oid: Option<u32> = row.get(1);
            match oid {
                Some(oid) if row.get::<_, Option<bool>>(2) == Some(true) => Ok(oid),
                Some(_) => Err(ChangeError::Invalid(format!(
                    "Нет права USAGE на тип {name}."
                ))),
                None => Err(ChangeError::Invalid(format!(
                    "Тип {name} не найден в выбранной базе."
                ))),
            }
        })
        .collect()
}
#[derive(Debug)]
pub enum ChangeError {
    Invalid(String),
    Database(crate::outcome::Diagnostic),
    DatabaseAt {
        diagnostic: crate::outcome::Diagnostic,
        line: usize,
        column: usize,
    },
    Conflict,
}
impl std::fmt::Display for ChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) => f.write_str(s),
            Self::Database(d) => f.write_str(&d.render()),
            Self::DatabaseAt { diagnostic, .. } => f.write_str(&diagnostic.render()),
            Self::Conflict => {
                f.write_str("Объект изменился после проверки. Получите новый предпросмотр.")
            }
        }
    }
}
fn snapshot(
    client: &mut impl GenericClient,
    project: &Project,
    signature: &str,
) -> Result<Option<Snapshot>, ChangeError> {
    let row=client.query_opt("SELECT p.oid, p.xmin::text, p.prokind::text, l.lanname, p.prosecdef, p.proisstrict, p.provolatile::text, p.proparallel::text, p.proconfig IS NULL, p.pronargdefaults, p.proretset, p.proargmodes IS NULL, p.proleakproof, p.prosupport=0, p.procost::float8 FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_language l ON l.oid=p.prolang WHERE p.oid=pg_catalog.to_regprocedure($1)",&[&signature]).map_err(|_|ChangeError::Invalid("Не удалось прочитать свойства выбранного объекта.".into()))?;
    let Some(row) = row else { return Ok(None) };
    let kind: String = row.get(2);
    if kind
        != if project.kind == Kind::Function {
            "f"
        } else {
            "p"
        }
    {
        return Err(ChangeError::Invalid(
            "С этой сигнатурой уже существует объект другого вида.".into(),
        ));
    }
    if row.get::<_, String>(3) != "plpgsql"
        || row.get::<_, bool>(4)
        || row.get::<_, bool>(5)
        || row.get::<_, String>(6) != "v"
        || row.get::<_, String>(7) != "u"
        || !row.get::<_, bool>(8)
        || row.get::<_, i16>(9) != 0
        || row.get::<_, bool>(10)
        || !row.get::<_, bool>(11)
        || row.get::<_, bool>(12)
        || !row.get::<_, bool>(13)
        || (project.kind == Kind::Function && row.get::<_, f64>(14) != 100.0)
    {
        return Err(ChangeError::Invalid("У объекта есть настройки или параметры, которые конструктор пока не сохраняет (например SECURITY DEFINER, STRICT, SET или OUT). Выберите новое имя; автоматическая замена запрещена.".into()));
    }
    let oid: u32 = row.get(0);
    let definition = client
        .query_one("SELECT pg_catalog.pg_get_functiondef($1)", &[&oid])
        .map_err(|_| ChangeError::Invalid("Не удалось прочитать определение объекта.".into()))?
        .get(0);
    Ok(Some(Snapshot {
        oid,
        revision: row.get(1),
        definition,
    }))
}
pub(crate) fn setup(
    client: &mut impl GenericClient,
    signature: &str,
) -> Result<(), postgres::Error> {
    client.batch_execute("SET LOCAL check_function_bodies=on; SET LOCAL lock_timeout='3s'; SET LOCAL statement_timeout='8s'")?;
    // Блокировка согласует операции нашего приложения; чужой DDL ей не подчиняется.
    client.query_one(
        "SELECT pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended($1, 7381))",
        &[&signature],
    )?;
    Ok(())
}
pub fn prepare(connection: &Connection, project: &Project) -> Result<Prepared, ChangeError> {
    let generated = project.generate().map_err(ChangeError::Invalid)?;
    let mut client = connection
        .config()
        .connect(NoTls)
        .map_err(|e| ChangeError::Database(connection.diagnostic(e)))?;
    let mut tx = client
        .transaction()
        .map_err(|e| ChangeError::Database(connection.diagnostic(e)))?;
    setup(&mut tx, &generated.signature)
        .map_err(|e| ChangeError::Database(connection.diagnostic(e)))?;
    let type_oids = check_named_types(&mut tx, connection, project)?;
    let before = snapshot(&mut tx, project, &generated.signature)?;
    tx.batch_execute(&generated.sql)
        .map_err(|e| ChangeError::Database(connection.diagnostic(e)))?;
    let after: String = tx
        .query_one(
            "SELECT pg_catalog.pg_get_functiondef(pg_catalog.to_regprocedure($1))",
            &[&generated.signature],
        )
        .map_err(|e| ChangeError::Database(connection.diagnostic(e)))?
        .get(0);
    tx.rollback()
        .map_err(|e| ChangeError::Database(connection.diagnostic(e)))?;
    Ok(Prepared {
        generated,
        before,
        after,
        type_oids,
    })
}
pub fn apply(
    connection: &Connection,
    project: &Project,
    expected: &Prepared,
) -> Result<(), ChangeError> {
    let generated = project.generate().map_err(ChangeError::Invalid)?;
    if generated.sql != expected.generated.sql {
        return Err(ChangeError::Conflict);
    }
    let mut client = connection
        .config()
        .connect(NoTls)
        .map_err(|e| ChangeError::Database(connection.diagnostic(e)))?;
    let mut tx = client
        .transaction()
        .map_err(|e| ChangeError::Database(connection.diagnostic(e)))?;
    setup(&mut tx, &generated.signature)
        .map_err(|e| ChangeError::Database(connection.diagnostic(e)))?;
    if check_named_types(&mut tx, connection, project)? != expected.type_oids {
        return Err(ChangeError::Conflict);
    }
    if snapshot(&mut tx, project, &generated.signature)? != expected.before {
        return Err(ChangeError::Conflict);
    }
    tx.batch_execute(&generated.sql)
        .map_err(|e| ChangeError::Database(connection.diagnostic(e)))?;
    tx.commit()
        .map_err(|e| ChangeError::Database(connection.diagnostic(e)))?;
    Ok(())
}
