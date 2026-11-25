use super::*;
use pg_debugger_core::{
    body,
    definition::{self, ChangeError, Prepared},
    project::Project,
};
use std::time::{Duration, Instant};

pub(super) struct Pending {
    id: String,
    connection_id: u64,
    database: String,
    payload: Payload,
    created: Instant,
}
// Старый запрос не должен сбрасывать свежий предпросмотр.
fn take_preview(
    state: &AppState,
    matches: impl FnOnce(&Pending) -> bool,
    stale_message: &str,
) -> Result<Pending, (StatusCode, Json<Value>)> {
    let mut slot = state.inner.pending.lock().map_err(|_| {
        failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Предпросмотр недоступен.",
        )
    })?;
    if slot.as_ref().is_some_and(|preview| {
        preview.created.elapsed() < Duration::from_secs(600) && matches(preview)
    }) {
        if let Some(preview) = slot.take() {
            return Ok(preview);
        }
    }
    Err(failure(StatusCode::CONFLICT, stale_message))
}

enum Payload {
    Project(Project, Prepared),
    Body(body::Prepared),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    connection_id: u64,
    database: String,
    project: Project,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ApplyInput {
    connection_id: u64,
    database: String,
    preview_id: String,
    project: Project,
}
fn error(error: ChangeError) -> (StatusCode, Json<Value>) {
    if let ChangeError::DatabaseAt {
        diagnostic,
        line,
        column,
    } = &error
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":diagnostic.render(),"location":{"line":line,"column":column}})),
        );
    }
    failure(
        if matches!(error, ChangeError::Conflict) {
            StatusCode::CONFLICT
        } else {
            StatusCode::BAD_REQUEST
        },
        error.to_string(),
    )
}
fn connection(
    state: &AppState,
    id: u64,
    database: &str,
) -> Result<Connection, (StatusCode, Json<Value>)> {
    let mut c = state
        .inner
        .connection
        .lock()
        .map_err(|_| {
            failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Состояние подключения недоступно.",
            )
        })?
        .as_ref()
        .filter(|(i, _)| *i == id)
        .map(|(_, c)| c.clone())
        .ok_or_else(|| {
            failure(
                StatusCode::CONFLICT,
                "Подключение изменилось. Подключитесь заново.",
            )
        })?;
    if database.is_empty() {
        return Err(failure(StatusCode::BAD_REQUEST, "Выберите базу данных."));
    }
    c.database = database.to_owned();
    Ok(c)
}
pub(super) async fn prepare(State(state): State<AppState>, Json(input): Json<Input>) -> Reply {
    let permit = state
        .inner
        .database_slot
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            failure(
                StatusCode::CONFLICT,
                "Дождитесь завершения предыдущего запроса.",
            )
        })?;
    let c = connection(&state, input.connection_id, &input.database)?;
    // Старый предпросмотр недействителен и при ошибке новой проверки.
    *state.inner.pending.lock().map_err(|_| {
        failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Предпросмотр недоступен.",
        )
    })? = None;
    tokio::task::spawn_blocking(move||{
        let _permit=permit;
        let change=definition::prepare(&c,&input.project).map_err(error)?;
        let mut bytes=[0u8;24];getrandom::fill(&mut bytes).map_err(|_|failure(StatusCode::INTERNAL_SERVER_ERROR,"Не удалось создать идентификатор проверки."))?;
        let id:String=bytes.iter().map(|b|format!("{b:02x}")).collect();
        let result=json!({"preview_id":id,"database":input.database,"signature":change.generated.signature,"sql":change.generated.sql,"sourceMap":change.generated.source_map,"before":change.before.as_ref().map(|s|&s.definition),"after":change.after,"action":if change.before.is_some(){"replace"}else{"create"},"message":"Определение принято PostgreSQL. Проверочная транзакция откачена; тело не вызывалось. Применение сохранит определение в базе."});
        *state.inner.pending.lock().map_err(|_|failure(StatusCode::INTERNAL_SERVER_ERROR,"Предпросмотр недоступен."))?=Some(Pending{id,connection_id:input.connection_id,database:input.database,payload:Payload::Project(input.project,change),created:Instant::now()});
        Ok(Json(result))
    }).await.map_err(|_|failure(StatusCode::INTERNAL_SERVER_ERROR,"Проверка определения не завершилась."))?
}
pub(super) async fn apply(State(state): State<AppState>, Json(input): Json<ApplyInput>) -> Reply {
    let permit = state
        .inner
        .database_slot
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            failure(
                StatusCode::CONFLICT,
                "Дождитесь завершения предыдущего запроса.",
            )
        })?;
    let c = connection(&state, input.connection_id, &input.database)?;
    let pending = take_preview(
        &state,
        |preview| {
            preview.id == input.preview_id
                && preview.connection_id == input.connection_id
                && preview.database == input.database
                && matches!(&preview.payload, Payload::Project(project, _) if project == &input.project)
        },
        "Проверка устарела, проект или подключение изменились. Проверьте определение заново.",
    )?;
    tokio::task::spawn_blocking(move||{
        let _permit=permit;
        let Payload::Project(_,change)=pending.payload else {return Err(failure(StatusCode::CONFLICT,"Повторите проверку конструктора."));};
        definition::apply(&c,&input.project,&change).map_err(error)?;
        Ok(Json(json!({"applied":true,"database":pending.database,"signature":change.generated.signature,"message":"Определение сохранено в базе. Тело не вызывалось."})))
    }).await.map_err(|_|failure(StatusCode::INTERNAL_SERVER_ERROR,"Применение не завершилось. Проверьте актуальное определение в каталоге перед повторной попыткой."))?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BodyInput {
    connection_id: u64,
    database: String,
    routine_oid: u32,
    expected_definition: String,
    body: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BodyApplyInput {
    connection_id: u64,
    database: String,
    routine_oid: u32,
    expected_definition: String,
    body: String,
    preview_id: String,
}
pub(super) async fn prepare_body(
    State(state): State<AppState>,
    Json(input): Json<BodyInput>,
) -> Reply {
    let permit = state
        .inner
        .database_slot
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            failure(
                StatusCode::CONFLICT,
                "Дождитесь завершения запроса или отладки.",
            )
        })?;
    let c = connection(&state, input.connection_id, &input.database)?;
    *state.inner.pending.lock().map_err(|_| {
        failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Предпросмотр недоступен.",
        )
    })? = None;
    tokio::task::spawn_blocking(move||{
        let _permit=permit;
        let change=body::prepare(&c,input.routine_oid,&input.expected_definition,&input.body).map_err(error)?;
        let mut bytes=[0u8;24];getrandom::fill(&mut bytes).map_err(|_|failure(StatusCode::INTERNAL_SERVER_ERROR,"Не удалось создать идентификатор проверки."))?;
        let id:String=bytes.iter().map(|b|format!("{b:02x}")).collect();
        let result=json!({"preview_id":id,"database":input.database,"routine_oid":input.routine_oid,"before":change.before.definition,"after":change.after,
            "message":"Тело принято PostgreSQL. Проверка завершена откатом; функция или процедура не вызывалась. Просмотрите определения и отдельно примените изменение."});
        *state.inner.pending.lock().map_err(|_|failure(StatusCode::INTERNAL_SERVER_ERROR,"Предпросмотр недоступен."))?=Some(Pending{id,connection_id:input.connection_id,database:input.database,payload:Payload::Body(change),created:Instant::now()});
        Ok(Json(result))
    }).await.map_err(|_|failure(StatusCode::INTERNAL_SERVER_ERROR,"Проверка тела не завершилась."))?
}
pub(super) async fn apply_body(
    State(state): State<AppState>,
    Json(request): Json<BodyApplyInput>,
) -> Reply {
    let input = BodyInput {
        connection_id: request.connection_id,
        database: request.database,
        routine_oid: request.routine_oid,
        expected_definition: request.expected_definition,
        body: request.body,
    };
    let permit = state
        .inner
        .database_slot
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            failure(
                StatusCode::CONFLICT,
                "Дождитесь завершения запроса или отладки.",
            )
        })?;
    let c = connection(&state, input.connection_id, &input.database)?;
    let pending = take_preview(
        &state,
        |preview| {
            preview.id == request.preview_id
                && preview.connection_id == input.connection_id
                && preview.database == input.database
                && matches!(&preview.payload, Payload::Body(change)
                    if change.before.oid == input.routine_oid
                        && change.before.definition == input.expected_definition
                        && change.body == input.body)
        },
        "Проверка устарела или черновик изменился. Проверьте тело заново.",
    )?;
    let Payload::Body(change) = pending.payload else {
        return Err(failure(StatusCode::CONFLICT, "Повторите проверку тела."));
    };
    tokio::task::spawn_blocking(move||{
        let _permit=permit;
        body::apply(&c,&change).map_err(error)?;
        Ok(Json(json!({"applied":true,"database":pending.database,"routine_oid":change.before.oid,"message":"Тело сохранено в базе. Сигнатура и свойства объекта сохранены. Вызов не выполнялся."})))
    }).await.map_err(|_|failure(StatusCode::INTERNAL_SERVER_ERROR,"Применение не завершилось. Обновите каталог и проверьте определение перед повтором."))?
}
