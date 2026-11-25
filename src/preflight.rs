use postgres::{Client, NoTls};
use std::{error::Error, io};

// Результат проверки существующей функции без вызова её тела.
pub struct PreflightReport {
    pub kind: String,
    pub source: String,
}

pub fn check(connection_string: &str, target: &str) -> Result<PreflightReport, Box<dyn Error>> {
    let mut client = Client::connect(connection_string, NoTls)?;
    // regprocedure учитывает типы аргументов и перегрузки.
    let row = client
        .query_opt(
            "SELECT p.oid, p.prokind::text AS kind, l.lanname, p.prosrc \
             FROM pg_proc p JOIN pg_language l ON l.oid = p.prolang \
             WHERE p.oid = to_regprocedure($1)",
            &[&target],
        )?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("Не найдена функция или процедура {target}"),
            )
        })?;

    let oid: u32 = row.try_get("oid")?;
    let kind: String = row.try_get("kind")?;
    let language: String = row.try_get("lanname")?;
    let source: String = row.try_get("prosrc")?;
    if language != "plpgsql" || !matches!(kind.as_str(), "f" | "p") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{target} должен быть функцией или процедурой на PL/pgSQL (сейчас: {language}, тип {kind})"),
        )
        .into());
    }

    // Валидатор проверяет тело без выполнения функции. SQL внутри тела он не исполняет.
    let mut transaction = client.transaction()?;
    transaction.batch_execute("SET LOCAL check_function_bodies = on")?;
    transaction
        .query_one("SELECT pg_catalog.plpgsql_validator($1::oid)", &[&oid])
        .map_err(|error| {
            if let Some(db) = error.as_db_error() {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Проверка PL/pgSQL не прошла (SQLSTATE {}): {}",
                        db.code().code(),
                        db.message()
                    ),
                )
            } else {
                io::Error::other(error.to_string())
            }
        })?;
    transaction.rollback()?;
    Ok(PreflightReport { kind, source })
}
