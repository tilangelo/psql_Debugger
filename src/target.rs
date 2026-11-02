use postgres::{Client, NoTls, SimpleQueryMessage};

// Строки, которые вернул целевой SQL.
pub struct TargetResult {
    pub rows: Vec<Vec<Option<String>>>,
}

// Исход SQL-вызова: результат или ошибка.
pub enum TargetOutcome {
    Succeeded(TargetResult),
    Failed {
        sqlstate: Option<String>,
        message: String,
    },
}

// Выполняем SQL и сохраняем его исход отдельно от отладки.
pub fn execute(connection_string: &str, sql: &str) -> TargetOutcome {
    // Исход SQL хранится отдельно от состояния отладчика.
    match run(connection_string, sql) {
        Ok(result) => TargetOutcome::Succeeded(result),
        Err(error) => {
            // У ошибки PostgreSQL берём SQLSTATE; у ошибки соединения его может не быть.
            let sqlstate = error
                .as_db_error()
                .map(|db_error| db_error.code().code().to_owned());
            let message = error
                .as_db_error()
                .map(|db_error| db_error.message().to_owned())
                .unwrap_or_else(|| error.to_string());
            TargetOutcome::Failed { sqlstate, message }
        }
    }
}

// Отправляем целевой SQL через отдельное соединение с PostgreSQL.
fn run(connection_string: &str, sql: &str) -> Result<TargetResult, postgres::Error> {
    // Здесь открываем второе соединение, как раньше делали через psql.
    let mut client = Client::connect(connection_string, NoTls)?;
    let mut rows = Vec::new();

    // simple_query отдаёт значения текстом; оставляем строки результата.
    for message in client.simple_query(sql)? {
        if let SimpleQueryMessage::Row(row) = message {
            let values = (0..row.len())
                .map(|index| row.try_get(index).map(|value| value.map(str::to_owned)))
                .collect::<Result<Vec<_>, _>>()?;
            rows.push(values);
        }
    }

    Ok(TargetResult { rows })
}
