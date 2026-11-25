//! Отдельное чтение структуры, чтобы выбор функции не загружал все колонки схемы.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    connection_id: u64,
    database: String,
    schema: Option<String>,
    relation_oid: Option<u32>,
    #[serde(default)]
    types_only: bool,
}

pub(super) async fn browse(State(state): State<AppState>, Json(input): Json<Input>) -> Reply {
    let permit = state
        .inner
        .database_slot
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            failure(
                StatusCode::CONFLICT,
                "Дождитесь завершения текущего запроса или отладки.",
            )
        })?;
    let mut connection = state
        .inner
        .connection
        .lock()
        .map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "Подключение недоступно."))?
        .as_ref()
        .filter(|(id, _)| *id == input.connection_id)
        .map(|(_, c)| c.clone())
        .ok_or_else(|| {
            failure(
                StatusCode::CONFLICT,
                "Подключение изменилось. Подключитесь заново.",
            )
        })?;
    if input.database.is_empty() {
        return Err(failure(StatusCode::BAD_REQUEST, "Выберите базу данных."));
    }
    connection.database = input.database.clone();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if input.types_only {
            let schema = input.schema.as_deref()
                .ok_or_else(|| failure(StatusCode::BAD_REQUEST, "Выберите схему типов."))?;
            let types = pg_debugger_core::catalog::constructor_types(&connection, schema).map_err(catalog_error)?;
            return Ok(Json(json!({"connection_id": input.connection_id, "database": input.database, "types": types})));
        }
        let metadata = pg_debugger_core::metadata::browse(&connection, input.schema.as_deref(), input.relation_oid)
            .map_err(catalog_error)?;
        Ok(Json(json!({"connection_id":input.connection_id,"database":input.database,"metadata":metadata})))
    }).await.map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "Чтение структуры не завершилось."))?
}
