use pg_debugger_web::{AppState, router};

const HELP: &str = "\
Локальный редактор и отладчик PostgreSQL.

Запуск: pg-debugger-web [--port PORT] [--no-browser] [--demo]
  --port PORT   HTTP-порт приложения (по умолчанию 18765, 0 - свободный порт).
  --no-browser  Не открывать браузер автоматически.
  --version     Версия программы.
  --demo        Linux: запустить временный PostgreSQL для примера.

1. Оставьте программу работающей и откройте показанный адрес в браузере.
2. Выберите свою базу или новый тестовый пример. Укажите параметры PostgreSQL.
3. Выберите базу, схему и функцию/процедуру. Задайте аргументы и начните отладку.
4. Для подсказок нажмите «Справка» в интерфейсе.
5. Для завершения нажмите Ctrl+C и дождитесь остановки вызова.

HTTP-порт приложения и порт PostgreSQL - разные настройки.
Для запуска готового файла Rust, Cargo и Node.js не нужны.
Для отладки нужны локальный PostgreSQL, pldbgapi, plugin_debugger и права владельца объекта.
Отладочный вызов откатывает транзакционные изменения; применение определения сохраняет его отдельно.";

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("Не удалось запустить приложение: {message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    // Читаем параметры запуска.
    let mut args = std::env::args().skip(1);
    let mut port = 18765;
    let mut open_browser = true;
    let mut isolated_demo = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("pg-debugger-web {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!("pg-debugger-web {}\n{HELP}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--port" => {
                port = args
                    .next()
                    .ok_or("Укажите HTTP-порт после --port.")?
                    .parse::<u16>()
                    .map_err(|_| "HTTP-порт должен быть числом от 0 до 65535.")?;
            }
            "--no-browser" => open_browser = false,
            "--demo" => isolated_demo = true,
            _ => return Err("Неизвестный параметр. Используйте --help для списка команд.".into()),
        }
    }
    // Открываем порт только на этом компьютере.
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,port)).await.map_err(|error| {
        if error.kind()==std::io::ErrorKind::AddrInUse {
            format!("HTTP-порт {port} занят. Завершите прежний экземпляр или запустите pg-debugger-web --port 18767.")
        } else { format!("Не удалось открыть HTTP-порт {port}: {error}") }
    })?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let shutdown_signal = Box::pin(tokio::signal::ctrl_c());
    #[cfg(target_os = "linux")]
    let mut shutdown_signal = shutdown_signal;
    #[cfg(target_os = "linux")]
    let mut demo_server = None;
    let state = if isolated_demo {
        #[cfg(not(target_os = "linux"))]
        return Err("Автозапуск временного PostgreSQL доступен в Linux. На Windows подключитесь к установленному PostgreSQL через форму.".into());
        #[cfg(target_os = "linux")]
        {
            println!("Готовим временный PostgreSQL для тестового примера…");
            let mut startup = tokio::task::spawn_blocking(move || AppState::start_demo(port));
            tokio::select! {
                result = &mut startup => {
                    let (state, server) = result.map_err(|error| error.to_string())??;
                    println!("Временный PostgreSQL: 127.0.0.1:{}", server.port());
                    demo_server = Some(server);
                    state
                }
                _ = &mut shutdown_signal => {
                    // Дожидаемся старта и сразу закрываем тестовый сервер.
                    if let Ok(Ok((_, server))) = startup.await { stop_demo(server).await?; }
                    return Ok(());
                }
            }
        }
    } else {
        AppState::new(port).map_err(|_| "Не удалось подготовить защиту локального приложения.")?
    };
    println!(
        "PostgreSQL Debugger {}\nОткройте http://127.0.0.1:{port}\nОставьте это окно открытым. Для остановки нажмите Ctrl+C.",
        env!("CARGO_PKG_VERSION")
    );
    if open_browser {
        open_page(port);
    }
    let shutdown_state = state.clone();
    let result = axum::serve(listener, router(state))
        .with_graceful_shutdown(async move {
            let _ = shutdown_signal.await;
            shutdown_state.shutdown().await;
        })
        .await
        .map_err(|e| format!("Локальный сервер завершился с ошибкой: {e}"));
    #[cfg(target_os = "linux")]
    if let Some(server) = demo_server {
        stop_demo(server).await?;
    }
    result
}

#[cfg(target_os = "linux")]
async fn stop_demo(mut server: pg_debugger_core::demo::DemoServer) -> Result<(), String> {
    tokio::task::spawn_blocking(move || server.shutdown().map_err(|error| error.to_string()))
        .await
        .map_err(|error| error.to_string())?
}

fn open_page(port: u16) {
    let url = format!("http://127.0.0.1:{port}");
    let mut command = if cfg!(target_os = "windows") {
        let mut command = std::process::Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler");
        command
    } else {
        std::process::Command::new("xdg-open")
    };
    if command
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_err()
    {
        eprintln!("Не удалось открыть браузер. Откройте показанный адрес вручную.");
    }
}
