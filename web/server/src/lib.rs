//! Локальный транспорт каталога. Пароль хранится только в памяти процесса.
mod changes;
mod debugger;
mod demo;
mod metadata;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use pg_debugger_core::catalog::{self, Catalog, Connection, Selection};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}
struct Inner {
    host: String,
    origin: String,
    token: String,
    connection: Mutex<Option<(u64, Connection)>>,
    isolated_demo: Option<Connection>,
    pending: Mutex<Option<changes::Pending>>,
    debug: Mutex<Option<Arc<debugger::Control>>>,
    next_id: AtomicU64,
    closing: AtomicBool,
    // Одновременно обрабатывается один запрос к PostgreSQL, включая переподключение.
    database_slot: Arc<Semaphore>,
}

impl AppState {
    pub async fn shutdown(&self) {
        debugger::shutdown(self).await;
    }

    pub fn new(port: u16) -> std::io::Result<Self> {
        Self::with_demo(port, None)
    }

    fn with_demo(port: u16, isolated_demo: Option<Connection>) -> std::io::Result<Self> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
        let host = format!("127.0.0.1:{port}");
        Ok(Self {
            inner: Arc::new(Inner {
                origin: format!("http://{host}"),
                host,
                token: bytes.iter().map(|b| format!("{b:02x}")).collect(),
                connection: Mutex::new(None),
                isolated_demo,
                pending: Mutex::new(None),
                debug: Mutex::new(None),
                next_id: AtomicU64::new(1),
                closing: AtomicBool::new(false),
                database_slot: Arc::new(Semaphore::new(1)),
            }),
        })
    }

    #[cfg(target_os = "linux")]
    pub fn start_demo(port: u16) -> Result<(Self, pg_debugger_core::demo::DemoServer), String> {
        let (server, password) = pg_debugger_core::demo::DemoServer::start_isolated()
            .map_err(|error| error.to_string())?;
        let bootstrap = Connection::new(
            "127.0.0.1",
            &server.port().to_string(),
            "postgres",
            "debugger_test",
            &password,
        )
        .map_err(|error| error.message.to_owned())?;
        let created = pg_debugger_core::demo::create(&bootstrap, "pgdebug_demo_0000000000000000")?;
        let state =
            Self::with_demo(port, Some(created.connection)).map_err(|error| error.to_string())?;
        Ok((state, server))
    }
}

type Reply = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn failure(code: StatusCode, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (code, Json(json!({"error":message.into()})))
}

// Host защищает loopback от подмены DNS; нестандартный заголовок требует same-origin запроса.
async fn guard(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if state.inner.closing.load(Ordering::Acquire)
        && !request.uri().path().starts_with("/api/debug/")
    {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "Приложение закрывается.").into_response();
    }
    let headers = request.headers();
    if headers.get(header::HOST).and_then(|v| v.to_str().ok()) != Some(&state.inner.host) {
        return failure(StatusCode::FORBIDDEN, "Недопустимый адрес приложения.").into_response();
    }
    if headers
        .get(header::ORIGIN)
        .is_some_and(|v| v.to_str().ok() != Some(&state.inner.origin))
    {
        return failure(
            StatusCode::FORBIDDEN,
            "Запрос должен приходить со страницы приложения.",
        )
        .into_response();
    }
    if request.uri().path().starts_with("/api/")
        && headers
            .get("x-pg-debugger-token")
            .and_then(|v| v.to_str().ok())
            != Some(&state.inner.token)
    {
        return failure(StatusCode::UNAUTHORIZED, "Обновите страницу приложения.").into_response();
    }
    let api_request = request.uri().path().starts_with("/api/");
    let mut response = next.run(request).await;
    // Ошибки стандартного JSON-парсера тоже возвращаем в понятном клиенту формате.
    if api_request
        && response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .is_none_or(|h| !h.starts_with("application/json"))
    {
        let message = match response.status() {
            StatusCode::UNPROCESSABLE_ENTITY => Some(
                "Некорректный формат проекта или запроса: проверьте версию, поля и типы значений.",
            ),
            StatusCode::BAD_REQUEST => Some("Некорректный JSON. Проверьте формат запроса."),
            StatusCode::PAYLOAD_TOO_LARGE => Some("Запрос слишком большой. Допускается до 2 МБ."),
            _ => None,
        };
        if let Some(message) = message {
            response = failure(response.status(), message).into_response();
        }
    }
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    // CodeMirror задаёт геометрию через style-атрибуты; скрипты по-прежнему только из приложения.
    headers.insert(header::CONTENT_SECURITY_POLICY,format!("default-src 'self'; script-src 'self'; style-src 'self' 'nonce-{}'; style-src-attr 'unsafe-inline'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",state.inner.token).parse().unwrap());
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    response
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route(
            "/style.css",
            get(|| async {
                asset(
                    "text/css; charset=utf-8",
                    include_str!("../../prototype/style.css"),
                )
            }),
        )
        .route(
            "/help.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/help.js"),
                )
            }),
        )
        .route(
            "/app.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/app.js"),
                )
            }),
        )
        .route(
            "/model.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/model.js"),
                )
            }),
        )
        .route(
            "/reverse-project.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/reverse-project.js"),
                )
            }),
        )
        .route(
            "/catalog.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/catalog.js"),
                )
            }),
        )
        .route("/api/debug/resolve", post(debugger::resolve))
        .route("/api/debug/start", post(debugger::start))
        .route("/api/debug/current", post(debugger::current))
        .route("/api/debug/poll", post(debugger::poll))
        .route("/api/debug/command", post(debugger::command))
        .route(
            "/body-editor.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/body-editor.js"),
                )
            }),
        )
        .route("/api/body/prepare", post(changes::prepare_body))
        .route("/api/body/apply", post(changes::apply_body))
        .route("/api/prepare", post(changes::prepare))
        .route("/api/apply", post(changes::apply))
        .route(
            "/debug.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/debug.js"),
                )
            }),
        )
        .route(
            "/readiness.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/readiness.js"),
                )
            }),
        )
        .route(
            "/metadata-reader.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/metadata-reader.js"),
                )
            }),
        )
        .route(
            "/metadata-client.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/metadata-client.js"),
                )
            }),
        )
        .route(
            "/sql-completions.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/sql-completions.js"),
                )
            }),
        )
        .route(
            "/sql-hints.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/sql-hints.js"),
                )
            }),
        )
        .route(
            "/vendor/code-editor.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/vendor/code-editor.js"),
                )
            }),
        )
        .route(
            "/vendor/THIRD_PARTY_LICENSES.txt",
            get(|| async {
                asset(
                    "text/plain; charset=utf-8",
                    include_str!("../../prototype/vendor/THIRD_PARTY_LICENSES.txt"),
                )
            }),
        )
        .route(
            "/sql-field.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/sql-field.js"),
                )
            }),
        )
        .route(
            "/working-draft.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/working-draft.js"),
                )
            }),
        )
        .route(
            "/debug-presentation.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../../prototype/debug-presentation.js"),
                )
            }),
        )
        .route(
            "/data-types.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/data-types.js"),
                )
            }),
        )
        .route(
            "/work-presets.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/work-presets.js"),
                )
            }),
        )
        .route(
            "/nested-point-plan.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/nested-point-plan.js"),
                )
            }),
        )
        .route(
            "/trigger-call.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/trigger-call.js"),
                )
            }),
        )
        .route(
            "/declarations.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/declarations.js"),
                )
            }),
        )
        .route(
            "/history.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/history.js"),
                )
            }),
        )
        .route(
            "/history-ui.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/history-ui.js"),
                )
            }),
        )
        .route(
            "/recovery-store.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/recovery-store.js"),
                )
            }),
        )
        .route(
            "/recovery.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/recovery.js"),
                )
            }),
        )
        .route(
            "/body-target.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/body-target.js"),
                )
            }),
        )
        .route("/api/connect", post(connect))
        .route("/api/demo", post(demo::create))
        .route("/api/demo/open-isolated", post(demo::open_isolated))
        .route("/api/catalog", post(browse))
        .route("/api/debug/setup", post(enable_debugger))
        .route("/api/metadata", post(metadata::browse))
        .route(
            "/api/info",
            post(|| async { Json(json!({"version":env!("CARGO_PKG_VERSION")})) }),
        )
        .route(
            "/diagnostics.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/diagnostics.js"),
                )
            }),
        )
        .route(
            "/diagnostic-report.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/diagnostic-report.js"),
                )
            }),
        )
        .route(
            "/select-settings.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/select-settings.js"),
                )
            }),
        )
        .route(
            "/templates.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/templates.js"),
                )
            }),
        )
        .route(
            "/template-panel.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/template-panel.js"),
                )
            }),
        )
        .route(
            "/workspace.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/workspace.js"),
                )
            }),
        )
        .route(
            "/workspace.css",
            get(|| async {
                asset(
                    "text/css; charset=utf-8",
                    include_str!("../../prototype/workspace.css"),
                )
            }),
        )
        .route(
            "/model-hints.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/model-hints.js"),
                )
            }),
        )
        .route(
            "/hints-editor.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/hints-editor.js"),
                )
            }),
        )
        .route(
            "/database-client.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/database-client.js"),
                )
            }),
        )
        .route(
            "/metadata.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/metadata.js"),
                )
            }),
        )
        .route(
            "/select-builder.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("../../prototype/select-builder.js"),
                )
            }),
        )
        .route("/api/disconnect", post(disconnect))
        .layer(DefaultBodyLimit::max(2_000_000))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}
fn asset(content_type: &'static str, content: &'static str) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, content_type)], content)
}
async fn index(State(state): State<AppState>) -> Html<String> {
    let demo = if state.inner.isolated_demo.is_some() {
        "<meta name=\"pg-debugger-isolated-demo\" content=\"true\">"
    } else {
        ""
    };
    Html(include_str!("../../prototype/index.html").replace(
        "<!--SERVER_BOOT-->",
        &format!(
            "<meta name=\"pg-debugger-token\" content=\"{}\"><meta name=\"pg-debugger-version\" content=\"{}\">{demo}",
            state.inner.token, env!("CARGO_PKG_VERSION"),
        ),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials {
    host: String,
    port: String,
    database: String,
    user: String,
    password: String,
}
async fn connect(State(state): State<AppState>, Json(input): Json<Credentials>) -> Reply {
    let connection = Connection::new(
        &input.host,
        &input.port,
        &input.database,
        &input.user,
        &input.password,
    )
    .map_err(|e| {
        failure(
            StatusCode::BAD_REQUEST,
            format!("{}: {}", e.field, e.message),
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
                "Дождитесь завершения предыдущего запроса.",
            )
        })?;
    // Синхронный клиент PostgreSQL не должен блокировать асинхронный обработчик.
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let catalog = catalog::browse(&connection, &Selection::default()).map_err(catalog_error)?;
        *state.inner.pending.lock().map_err(|_| {
            failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Предпросмотр недоступен.",
            )
        })? = None;
        let id = state.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let database = connection.database.clone();
        let endpoint = connection.endpoint();
        *state.inner.connection.lock().map_err(|_| {
            failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Состояние подключения недоступно.",
            )
        })? = Some((id, connection));
        Ok(Json(catalog_json(id, &database, catalog, endpoint)))
    })
    .await
    .map_err(|_| {
        failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Запрос каталога не завершился.",
        )
    })?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Browse {
    connection_id: u64,
    database: String,
    schema: Option<String>,
    routine_oid: Option<u32>,
}
async fn browse(State(state): State<AppState>, Json(input): Json<Browse>) -> Reply {
    load_catalog_for_request(state, input, catalog::browse).await
}

async fn enable_debugger(State(state): State<AppState>, Json(input): Json<Browse>) -> Reply {
    load_catalog_for_request(state, input, catalog::enable_debugger).await
}

async fn load_catalog_for_request(
    state: AppState,
    input: Browse,
    load: fn(&catalog::Connection, &Selection) -> Result<Catalog, catalog::RequestError>,
) -> Reply {
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
    if input.database.is_empty() {
        return Err(failure(StatusCode::BAD_REQUEST, "Укажите базу данных."));
    }
    connection.database = input.database;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let selection = Selection {
            schema: input.schema,
            routine_oid: input.routine_oid,
        };
        let catalog = load(&connection, &selection).map_err(catalog_error)?;
        Ok(Json(catalog_json(
            input.connection_id,
            &connection.database,
            catalog,
            connection.endpoint(),
        )))
    })
    .await
    .map_err(|_| {
        failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Запрос каталога не завершился.",
        )
    })?
}
async fn disconnect(State(state): State<AppState>) -> Reply {
    let _permit = state
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
    *state.inner.connection.lock().map_err(|_| {
        failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Состояние подключения недоступно.",
        )
    })? = None;
    *state.inner.pending.lock().map_err(|_| {
        failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Предпросмотр недоступен.",
        )
    })? = None;
    Ok(Json(json!({"disconnected":true})))
}
fn catalog_error(error: catalog::RequestError) -> (StatusCode, Json<Value>) {
    failure(
        StatusCode::BAD_REQUEST,
        if error.detail.is_empty() {
            error.message.to_owned()
        } else {
            format!("{}\n{}", error.message, error.detail)
        },
    )
}
fn catalog_json(id: u64, database: &str, catalog: Catalog, endpoint: (String, u16)) -> Value {
    json!({"endpoint":{"host":endpoint.0,"port":endpoint.1},"connection_id":id,"database":database,"server_version":catalog.server_version,"current_user":catalog.current_user,"databases":catalog.databases,"schemas":catalog.schemas,"schema":catalog.schema,
        "debugger":catalog.debugger.message(),"debugger_code":catalog.debugger.code(),"debugger_setup":catalog.debugger_setup,
        "routines":catalog.routines.into_iter().map(|r|json!({"oid":r.oid,"name":r.name,"kind":r.kind.label(),"signature":r.signature,"debug_allowed":r.debug_allowed,"owner":r.owner})).collect::<Vec<_>>(),
        "selected":catalog.selected.map(|r|json!({"oid":r.oid,"source":r.source,"definition":r.definition,"trigger_targets":r.trigger_targets,
            "arguments":r.arguments.into_iter().map(|a|json!({"name":a.name,"type":a.type_name,"type_oid":a.type_oid,"mode":a.mode,"format":a.input_format(),"supported":a.accepts_input(),"has_default":a.has_default,"enum_labels":a.custom_type.as_ref().and_then(|ty|ty.enum_labels.as_deref()),"fields":a.custom_type.as_ref().and_then(|ty|ty.fields.as_deref())})).collect::<Vec<_>>()}))})
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;

    async fn call(
        path: &str,
        host: &str,
        token: bool,
        origin: Option<&str>,
        body: &str,
    ) -> (StatusCode, String) {
        let state = AppState::new(18765).unwrap();
        let mut request = Request::builder()
            .method(if path == "/" { "GET" } else { "POST" })
            .uri(path)
            .header("host", host)
            .header("content-type", "application/json");
        if token {
            request = request.header("x-pg-debugger-token", &state.inner.token);
        }
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let response = router(state)
            .oneshot(request.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap();
        (
            response.status(),
            String::from_utf8(
                to_bytes(response.into_body(), 1_000_000)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap(),
        )
    }
    #[tokio::test]
    async fn cross_origin_and_missing_token_are_rejected() {
        assert_eq!(
            call("/api/disconnect", "127.0.0.1:18765", false, None, "")
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(
                "/api/disconnect",
                "127.0.0.1:18765",
                true,
                Some("http://example.com"),
                ""
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call("/", "example.com:18765", false, None, "").await.0,
            StatusCode::FORBIDDEN
        );
    }
    #[tokio::test]
    async fn nonlocal_database_and_stale_connection_rejected() {
        let body = r#"{"host":"192.0.2.1","port":"5432","database":"postgres","user":"test","password":"do-not-return-this"}"#;
        let response = call("/api/connect", "127.0.0.1:18765", true, None, body).await;
        assert_eq!(response.0, StatusCode::BAD_REQUEST);
        assert!(!response.1.contains("do-not-return-this"));
        for field in ["database", "user", "password"] {
            let mut value = json!({"host":"127.0.0.1","port":"5432","database":"postgres","user":"test","password":"private-password"});
            value[field] = json!("value\0tail");
            let response = call(
                "/api/connect",
                "127.0.0.1:18765",
                true,
                None,
                &value.to_string(),
            )
            .await;
            assert_eq!(response.0, StatusCode::BAD_REQUEST);
            assert!(!response.1.contains("private-password"));
            assert!(!response.1.contains("tail"));
        }
        let stale = r#"{"connection_id":9,"database":"postgres"}"#;
        assert_eq!(
            call("/api/catalog", "127.0.0.1:18765", true, None, stale)
                .await
                .0,
            StatusCode::CONFLICT
        );
    }
    #[tokio::test]
    async fn malformed_request_is_json_and_does_not_echo_input() {
        let response = call(
            "/api/prepare",
            "127.0.0.1:18765",
            true,
            None,
            r#"{"password":"private-value"}"#,
        )
        .await;
        assert_eq!(response.0, StatusCode::UNPROCESSABLE_ENTITY);
        let value: Value = serde_json::from_str(&response.1).unwrap();
        assert!(value["error"].as_str().unwrap().contains("формат"));
        assert!(!response.1.contains("private-value"));
    }
    #[tokio::test]
    async fn assets_and_body_limit() {
        let index = call("/", "127.0.0.1:18765", false, None, "").await;
        assert_eq!(index.0, StatusCode::OK);
        assert!(index.1.contains("pg-debugger-token"));
        assert!(!index.1.contains("<!--SERVER_BOOT-->"));
        let huge = " ".repeat(2_000_001);
        assert_eq!(
            call("/api/connect", "127.0.0.1:18765", true, None, &huge)
                .await
                .0,
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }
}
