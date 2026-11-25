//! Создание примера разрешено только явным запросом страницы и занимает общий слот базы.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Create {
    connection_id: u64,
    database: String,
    confirmed: bool,
}

pub(super) async fn create(State(state): State<AppState>, Json(input): Json<Create>) -> Reply {
    if !input.confirmed {
        return Err(failure(
            StatusCode::BAD_REQUEST,
            "Подтвердите создание новой отдельной тестовой базы.",
        ));
    }
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
        .map_err(|_| {
            failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Состояние подключения недоступно.",
            )
        })?
        .as_ref()
        .filter(|(id, _)| *id == input.connection_id)
        .map(|(_, c)| c.clone())
        .ok_or_else(|| {
            failure(
                StatusCode::CONFLICT,
                "Подключение изменилось. Подключитесь заново.",
            )
        })?;
    connection.database = input.database;
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|_| {
        failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Не удалось выбрать имя новой базы.",
        )
    })?;
    let name = format!(
        "pgdebug_demo_{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let created = pg_debugger_core::demo::create(&connection, &name)
            .map_err(|err| failure(StatusCode::BAD_REQUEST, err))?;
        select_example(&state, created.connection, created.catalog,
            "Новая тестовая база сохранена на сервере. Её имя можно использовать для повторного подключения. Автоматического удаления нет.")
    }).await.map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "Подготовка тестовой базы не завершилась. Проверьте список баз перед повтором."))?
}

pub(super) async fn open_isolated(State(state): State<AppState>) -> Reply {
    let connection = state.inner.isolated_demo.clone().ok_or_else(|| {
        failure(
            StatusCode::NOT_FOUND,
            "Изолированный пример доступен при запуске с --demo.",
        )
    })?;
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
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let catalog = pg_debugger_core::demo::browse(&connection)
            .map_err(|error| failure(StatusCode::BAD_REQUEST, error))?;
        select_example(&state, connection, catalog, "Пример работает во временном PostgreSQL. При штатном выходе из приложения его данные удаляются.")
    }).await.map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось открыть временную тестовую базу."))?
}

fn select_example(
    state: &AppState,
    connection: Connection,
    catalog: Catalog,
    message: &str,
) -> Reply {
    let endpoint = connection.endpoint();
    let name = connection.database.clone();
    let id = state.inner.next_id.fetch_add(1, Ordering::Relaxed);
    *state.inner.pending.lock().map_err(|_| {
        failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Предпросмотр недоступен.",
        )
    })? = None;
    *state.inner.connection.lock().map_err(|_| {
        failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Состояние подключения недоступно.",
        )
    })? = Some((id, connection));
    let mut result = catalog_json(id, &name, catalog, endpoint);
    result["demo"] = json!({"database":name,"argument":"3","expected":"8","message":message});
    Ok(Json(result))
}
