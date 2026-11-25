use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{RecvTimeoutError, channel},
};
use std::time::Duration;
#[cfg(windows)]
mod demo;
#[cfg(target_os = "linux")]
use pg_debugger::demo_linux as demo;
mod preflight;
mod target;

use pg_debugger::pldbg::{Breakpoint, DebugEvent, PldbgClient};
use std::{
    env,
    error::Error,
    io::{self, Write},
    net::TcpListener,
    process, thread,
};
use target::TargetOutcome;

const USAGE: &str = "Usage: pg-debugger [help | info | check TARGET | listen TARGET | interactive TARGET | interactive-line TARGET LINE | interactive-run TARGET SQL]";

// Без аргументов открываем учебное меню; прежние CLI-режимы сохраняются.
fn main() {
    let demo_mode = env::args_os().len() == 1;
    if let Err(error) = run() {
        eprintln!("Ошибка: {error}");
        if demo_mode {
            eprintln!("Нажмите Enter, чтобы закрыть окно.");
            let mut input = String::new();
            let _ = io::stdin().read_line(&mut input);
        }
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let Some(mode) = args.next() else {
        return run_demo();
    };
    if mode == "help" {
        if args.next().is_some() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, USAGE).into());
        }
        println!("{USAGE}");
        return Ok(());
    }

    let connection_string = env::var("PGDEBUG_DATABASE_URL").map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Set PGDEBUG_DATABASE_URL to a PostgreSQL connection string",
        )
    })?;

    if mode == "check" {
        let target = args
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, USAGE))?;
        if args.next().is_some() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, USAGE).into());
        }
        print_preflight(&connection_string, &target)?;
        return Ok(());
    }

    let mut debugger = PldbgClient::connect(&connection_string)?;
    print_proxy_info(&mut debugger)?;

    match mode.as_str() {
        "info" if args.next().is_none() => Ok(()),
        "listen" | "interactive" | "interactive-line" | "interactive-run" => {
            // По сигнатуре выбираем функцию для точки остановки.
            let target = args
                .next()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, USAGE))?;
            let requested_line = if mode == "interactive-line" {
                let raw = args
                    .next()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, USAGE))?;
                Some(parse_line(&raw)?)
            } else {
                None
            };
            // Сам SQL нужен только тогда, когда запускаем вызов из Rust.
            let sql = if mode == "interactive-run" {
                Some(
                    args.next()
                        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, USAGE))?,
                )
            } else {
                None
            };
            // Лишние аргументы считаем ошибкой ввода.
            if args.next().is_some() {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, USAGE).into());
            }
            let mut line = requested_line;
            if let Some(requested) = requested_line {
                let report = preflight::check(&connection_string, &target)?;
                if requested as usize > report.source.lines().count() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Строка за пределами тела функции",
                    )
                    .into());
                }
                line = initial_breakpoint_line(&report.source, requested);
            }

            // Выбираем, кто запускает SQL: Rust или внешний клиент.
            match sql {
                Some(sql) => {
                    interactive_run(&connection_string, &mut debugger, &target, &sql, None)
                }
                None if mode == "interactive" || mode == "interactive-line" => {
                    interactive(&mut debugger, &target, line)
                }
                None => listen(&mut debugger, &target),
            }
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, USAGE).into()),
    }
}

fn parse_line(raw: &str) -> Result<i32, Box<dyn Error>> {
    let line: i32 = raw.parse()?;
    if line <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Номер строки должен быть положительным",
        )
        .into());
    }
    Ok(line)
}

// До первого оператора плагин может не остановиться на объявлении или BEGIN.
// Для такой строки ставим точку на входе: остановка покажет фактический оператор.
fn initial_breakpoint_line(source: &str, requested: i32) -> Option<i32> {
    let lines: Vec<_> = source.lines().collect();
    let Some(begin) = lines
        .iter()
        .position(|line| line.trim().eq_ignore_ascii_case("BEGIN"))
    else {
        return Some(requested);
    };
    let first_statement = lines
        .iter()
        .enumerate()
        .skip(begin + 1)
        .find(|(_, line)| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with("--")
        })
        .map(|(index, _)| index + 1)
        .unwrap_or(begin + 1);
    if requested as usize <= first_statement {
        println!(
            "Строка {requested} находится до первого оператора или на нём; ставлю точку на входе. Фактическую строку покажу при остановке."
        );
        None
    } else {
        Some(requested)
    }
}

fn print_preflight(connection_string: &str, target: &str) -> Result<(), Box<dyn Error>> {
    let report = preflight::check(connection_string, target)?;
    show_preflight(target, &report);
    Ok(())
}

fn show_preflight(target: &str, report: &preflight::PreflightReport) {
    let kind = if report.kind == "p" {
        "процедура"
    } else {
        "функция"
    };
    println!("{target}: {kind} на PL/pgSQL найдена.");
    println!("Синтаксис тела принят валидатором PL/pgSQL. Тело не выполнялось.");
    println!("SQL-операторы внутри тела могут обнаружить ошибки только при выполнении.");
    for (index, source_line) in report.source.lines().enumerate() {
        println!("{:>3} | {source_line}", index + 1);
    }
}

// Сведения о PostgreSQL читаем через тот же proxy backend, который будет отлаживать функцию.
fn print_proxy_info(debugger: &mut PldbgClient) -> Result<(), postgres::Error> {
    let info = debugger.proxy_info()?;
    println!("PostgreSQL: {}", info.server_version);
    println!("Server version number: {}", info.server_version_num);
    println!("pldbgapi protocol version: {}", info.proxy_api_version);
    println!("Plugin build PostgreSQL: {}", info.plugin_build_version);
    println!("Proxy backend PID: {}", info.proxy_pid);
    Ok(())
}

// Меню готовит среду один раз и затем запускает выбранные сценарии.
fn run_demo() -> Result<(), Box<dyn Error>> {
    println!("Отладчик PL/pgSQL");
    let (mut owned, connection_string, port) = loop {
        let port = demo::choose_port()?;
        if TcpListener::bind(("127.0.0.1", port)).is_err() {
            if let Some(connection) = connect_existing(port)? {
                break (None, connection, port);
            }
            continue;
        }
        match demo::DemoServer::start(port) {
            Ok(server) => {
                let connection = server.connection_string().to_owned();
                break (Some(server), connection, port);
            }
            Err(error)
                if error.downcast_ref::<io::Error>().is_some_and(|error| {
                    matches!(
                        error.kind(),
                        io::ErrorKind::AddrInUse | io::ErrorKind::PermissionDenied
                    )
                }) =>
            {
                eprintln!("{error}");
            }
            Err(error) => return Err(error),
        }
    };
    if owned.is_some() {
        println!("Тестовый PostgreSQL готов на 127.0.0.1:{port}.");
    } else {
        println!(
            "Подключено к существующему PostgreSQL на 127.0.0.1:{port}. Его данные и служба не изменяются."
        );
    }

    loop {
        println!();
        if owned.is_some() {
            println!("1 - быстрый показ: автоматический шаг (Enter)");
            println!("2 - ручная отладка тестовой функции");
            println!("3 - вызвать тестовую функцию через свой psql");
            println!("4 - пример функции с ошибкой");
        }
        println!(
            "5 - проверить выбранную функцию или процедуру до запуска{}",
            if owned.is_none() { " (Enter)" } else { "" }
        );
        println!("6 - отладить выбранную функцию или процедуру");
        println!("0 - выход");
        print!("Режим: ");
        io::stdout().flush()?;

        let mut choice = String::new();
        if io::stdin().read_line(&mut choice)? == 0 {
            break;
        }
        if choice.trim() == "0" {
            break;
        }
        let selected = if choice.trim().is_empty() {
            if owned.is_some() { "1" } else { "5" }
        } else {
            choice.trim()
        };
        if !matches!(selected, "1" | "2" | "3" | "4" | "5" | "6") {
            println!("Неизвестный режим.");
            continue;
        }
        let result = (|| -> Result<(), Box<dyn Error>> {
            match selected {
                "5" => {
                    let target = prompt("Сигнатура, например public.debug_smoke(integer): ")?;
                    print_preflight(&connection_string, target.trim())
                }
                "6" => custom_session(&connection_string),
                "1" | "2" | "3" | "4" => {
                    let server = owned.as_ref().ok_or_else(|| {
                        io::Error::other(
                            "Тестовые сценарии доступны только для созданного программой кластера",
                        )
                    })?;
                    let mut debugger = PldbgClient::connect(&connection_string)?;
                    print_proxy_info(&mut debugger)?;
                    match selected {
                        "1" => external_demo(server, &mut debugger, false),
                        "2" => external_demo(server, &mut debugger, true),
                        "3" => manual_psql_demo(server, &mut debugger),
                        "4" => interactive_run(
                            &connection_string,
                            &mut debugger,
                            "public.debug_failure(integer)",
                            "SELECT public.debug_failure(3);",
                            None,
                        ),
                        _ => unreachable!(),
                    }
                }
                _ => unreachable!(),
            }
        })();
        if let Err(error) = result {
            if selected == "4" && error.to_string() == "target SQL failed" {
                println!("Это ожидаемая ошибка тестовой функции.");
            } else {
                eprintln!("Сценарий завершился с ошибкой: {error}");
            }
        }
    }

    if let Some(server) = owned.as_mut() {
        println!("Останавливаю тестовый PostgreSQL...");
        server.shutdown()?;
    } else {
        println!("Отключаюсь от существующего PostgreSQL; сервер продолжает работать.");
    }
    println!("Готово.");
    Ok(())
}

fn prompt(label: &str) -> io::Result<String> {
    print!("{label}");
    io::stdout().flush()?;
    let mut input = String::new();
    if io::stdin().read_line(&mut input)? == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Ввод закрыт"));
    }
    Ok(input.trim().to_owned())
}

fn quote_conn(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn verify_postgres(connection: &str) -> Result<(), postgres::Error> {
    let mut client = postgres::Client::connect(connection, postgres::NoTls)?;
    client.query_one("SELECT version()", &[])?;
    Ok(())
}

// Занятый порт используем как существующий PostgreSQL, не создавая там объекты.
fn connect_existing(port: u16) -> Result<Option<String>, Box<dyn Error>> {
    let default =
        format!("host=127.0.0.1 port={port} user=debugger_test dbname=postgres connect_timeout=3");
    if verify_postgres(&default).is_ok() {
        println!("Найден работающий PostgreSQL; подключаюсь как debugger_test к базе postgres.");
        return Ok(Some(default));
    }
    println!("Порт {port} занят. Для подключения к PostgreSQL укажите пользователя и базу.");
    loop {
        let user = prompt("Пользователь [postgres], 0 - выбрать другой порт: ")?;
        if user == "0" {
            return Ok(None);
        }
        let user = if user.is_empty() { "postgres" } else { &user };
        let database = prompt("База данных [postgres]: ")?;
        let database = if database.is_empty() {
            "postgres"
        } else {
            &database
        };
        let password = prompt("Пароль (Enter, если не нужен; ввод виден): ")?;
        let mut connection = format!(
            "host=127.0.0.1 port={port} user={} dbname={} connect_timeout=3",
            quote_conn(user),
            quote_conn(database)
        );
        if !password.is_empty() {
            connection.push_str(&format!(" password={}", quote_conn(&password)));
        }
        match verify_postgres(&connection) {
            Ok(()) => return Ok(Some(connection)),
            Err(error) => eprintln!("Подключение не удалось: {error}"),
        }
    }
}

fn custom_session(connection: &str) -> Result<(), Box<dyn Error>> {
    let target = prompt("Сигнатура функции/процедуры: ")?;
    let report = preflight::check(connection, &target)?;
    show_preflight(&target, &report);
    let raw_line = prompt("Строка первой остановки (Enter - на входе): ")?;
    let requested_line = if raw_line.is_empty() {
        None
    } else {
        Some(parse_line(&raw_line)?)
    };
    let mut line = requested_line;
    if let Some(requested) = requested_line {
        if requested as usize > report.source.lines().count() {
            return Err(
                io::Error::new(io::ErrorKind::InvalidInput, "Строка за пределами тела").into(),
            );
        }
        line = initial_breakpoint_line(&report.source, requested);
    }
    let mode = prompt("Вызов: 1 - выполнить SQL здесь, 2 - ждать другой клиент/ORM [1]: ")?;
    let mut debugger = PldbgClient::connect(connection)?;
    print_proxy_info(&mut debugger).map_err(|error| {
        io::Error::other(format!(
            "Отладка недоступна: проверьте pldbgapi и plugin_debugger на этом сервере: {error}"
        ))
    })?;
    match mode.as_str() {
        "" | "1" => {
            let sql = prompt("SQL-вызов, например SELECT public.debug_smoke(3);: ")?;
            if sql.is_empty() {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "SQL-вызов пуст").into());
            }
            interactive_run(connection, &mut debugger, &target, &sql, line)
        }
        "2" => {
            println!(
                "После установки точки вызовите {target} через другое соединение PostgreSQL или ORM."
            );
            interactive(&mut debugger, &target, line)
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Выберите 1 или 2").into()),
    }
}

// Ждём вызов из окна psql, которое пользователь открыл самостоятельно.
fn manual_psql_demo(
    server: &demo::DemoServer,
    debugger: &mut PldbgClient,
) -> Result<(), Box<dyn Error>> {
    let listener = prepare_session(debugger, "public.debug_smoke(integer)", None)?;
    println!("Откройте второй терминал и выполните команду:");
    println!(
        "\"{}\" -h 127.0.0.1 -p {} -U debugger_test -d postgres -X -c \"SELECT public.debug_smoke(3);\"",
        server.psql_path().display(),
        server.port()
    );
    println!("Второе окно будет ждать, пока вы управляете отладкой здесь.");
    wait_for_session(debugger, listener)?;
    interactive_loop(debugger, listener, "public.debug_smoke(integer)")
}

// Внешний psql запускается сам: проверяющему не нужно открывать второе окно.
fn external_demo(
    server: &demo::DemoServer,
    debugger: &mut PldbgClient,
    interactive_mode: bool,
) -> Result<(), Box<dyn Error>> {
    let listener = prepare_session(debugger, "public.debug_smoke(integer)", None)?;
    let mut child = server.launch_psql("SELECT public.debug_smoke(3);")?;
    println!("Запущен отдельный клиент psql (PID {}).", child.id());

    let debug_result = (|| -> Result<(), Box<dyn Error>> {
        wait_for_session(debugger, listener)?;
        if interactive_mode {
            interactive_loop(debugger, listener, "public.debug_smoke(integer)")
        } else {
            auto_session(debugger, listener)
        }
    })();
    if debug_result.is_err() {
        let _ = child.kill();
    }
    let output = child.wait_with_output()?;
    debug_result?;

    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        return Err(io::Error::other(format!("psql завершился с кодом {}", output.status)).into());
    }
    Ok(())
}
// Создаём сеанс и ставим глобальную точку до запуска функции.
fn prepare_session(
    debugger: &mut PldbgClient,
    target: &str,
    line: Option<i32>,
) -> Result<i32, Box<dyn Error>> {
    // Создаём сеанс отладки до вызова целевой функции.
    let listener = debugger.create_listener()?;
    println!("Listener: {listener}");

    // Ставим точку заранее, чтобы не пропустить вызов функции.
    if !debugger.set_global_breakpoint(listener, target, line)? {
        return Err(io::Error::other("PostgreSQL did not set the breakpoint").into());
    }
    match line {
        Some(line) => println!("Global breakpoint set for {target} at source line {line}"),
        None => println!("Global breakpoint set for {target}"),
    }
    println!("Waiting for another PostgreSQL session to call the target...");
    Ok(listener)
}

// Дожидаемся целевого процесса и первой остановки.
fn wait_for_session(debugger: &mut PldbgClient, listener: i32) -> Result<(), Box<dyn Error>> {
    // Ждём, пока другой backend дойдёт до точки остановки.
    let target_pid = debugger.wait_for_target(listener)?;
    println!("Target PID: {target_pid}");

    // Теперь читаем строку, на которой остановилась функция.
    let stop = debugger.wait_for_breakpoint(listener)?;
    print_actual_stop(debugger, listener, &stop, "Stopped at")?;
    Ok(())
}

// Общая подготовка для режимов с внешним вызовом функции.
fn start_session(debugger: &mut PldbgClient, target: &str) -> Result<i32, Box<dyn Error>> {
    let listener = prepare_session(debugger, target, None)?;
    wait_for_session(debugger, listener)?;
    Ok(listener)
}
// Автоматически показываем состояние, делаем шаг и продолжаем вызов.
fn listen(debugger: &mut PldbgClient, target: &str) -> Result<(), Box<dyn Error>> {
    let listener = start_session(debugger, target)?;
    auto_session(debugger, listener)
}

// Показываем состояние, делаем шаг и продолжаем вызов без команд пользователя.
fn auto_session(debugger: &mut PldbgClient, listener: i32) -> Result<(), Box<dyn Error>> {
    print_stack(debugger, listener)?;
    print_variables(debugger, listener)?;

    match debugger.step_over(listener)? {
        DebugEvent::Paused(stop) => {
            print_actual_stop(debugger, listener, &stop, "After step over:")?;
            print_variables(debugger, listener)?;
            continue_until_closed(debugger, listener)
        }
        DebugEvent::ChannelClosed => {
            report_channel_closed("step");
            Ok(())
        }
    }
}
// Ждём вызов функции из psql или другого внешнего клиента.
fn interactive(
    debugger: &mut PldbgClient,
    target: &str,
    line: Option<i32>,
) -> Result<(), Box<dyn Error>> {
    let listener = prepare_session(debugger, target, line)?;
    wait_for_session(debugger, listener)?;
    interactive_loop(debugger, listener, target)
}

// Запускаем SQL сами и после отладки получаем его результат.
fn interactive_run(
    connection_string: &str,
    debugger: &mut PldbgClient,
    target: &str,
    sql: &str,
    line: Option<i32>,
) -> Result<(), Box<dyn Error>> {
    let listener = prepare_session(debugger, target, line)?;
    let connection_string = connection_string.to_owned();
    let sql = sql.to_owned();
    let cancellation = Arc::new(target::Cancellation::default());
    let target_cancel = cancellation.clone();
    let (done, result) = channel();
    let target_thread = thread::spawn(move || {
        let outcome = target::execute(&connection_string, &sql, &target_cancel);
        let _ = done.send(outcome);
    });
    let waiting = Arc::new(AtomicBool::new(true));
    let monitor_waiting = waiting.clone();
    let proxy_cancel = debugger.cancel_token();
    let monitor_cancel = cancellation.clone();
    // Ранняя ошибка SQL должна прервать серверное ожидание listener.
    // Повторная отмена закрывает гонку, когда proxy ещё не начал ждать.
    let monitor = thread::spawn(move || -> Result<TargetOutcome, io::Error> {
        let outcome = loop {
            match result.recv_timeout(Duration::from_millis(150)) {
                Ok(outcome) => break Ok(outcome),
                Err(RecvTimeoutError::Timeout) => {
                    if monitor_cancel.requested() {
                        monitor_cancel.cancel();
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    break Err(io::Error::other("Поток целевого SQL прервался."));
                }
            }
        };
        while monitor_waiting.load(Ordering::Acquire) {
            let _ = proxy_cancel.cancel_query(postgres::NoTls);
            thread::sleep(Duration::from_millis(100));
        }
        outcome
    });
    let first_stop = wait_for_session(debugger, listener);
    waiting.store(false, Ordering::Release);
    let attached = first_stop.is_ok();
    let debug_result = first_stop.and_then(|()| interactive_loop(debugger, listener, target));
    if debug_result.is_err() {
        if attached {
            let _ = debugger.abort(listener);
        }
        cancellation.cancel();
    }
    let outcome = monitor
        .join()
        .map_err(|_| io::Error::other("Поток наблюдения за SQL прервался."))?;
    target_thread
        .join()
        .map_err(|_| io::Error::other("Target SQL worker panicked"))?;
    let target_result = report_target_outcome(outcome?);
    if !attached {
        target_result?;
        return Err(io::Error::other(
            "SQL завершился до первой остановки. Проверь соответствие выбранной функции и вызова.",
        )
        .into());
    }
    target_result?;
    debug_result
}

// Читаем команды, пока целевая функция остановлена.
fn interactive_loop(
    debugger: &mut PldbgClient,
    listener: i32,
    target: &str,
) -> Result<(), Box<dyn Error>> {
    print_stack(debugger, listener)?;
    print_variables(debugger, listener)?;
    print_help();

    loop {
        print!("debug> ");
        io::stdout().flush()?;

        let mut input = String::new();
        // При закрытии ввода продолжаем функцию, чтобы она не зависла.
        if io::stdin().read_line(&mut input)? == 0 {
            println!("Input closed; continuing target.");
            return continue_until_closed(debugger, listener);
        }

        // Команды из консоли переводим в вызовы pldbgapi.
        let words: Vec<_> = input.split_whitespace().collect();
        match words.as_slice() {
            ["b" | "break", raw] => {
                let line = parse_line(raw)?;
                if debugger.set_breakpoint(listener, target, line)? {
                    println!("Breakpoint set at source line {line}.");
                } else {
                    println!("PostgreSQL rejected breakpoint at line {line}.");
                }
            }
            ["d" | "drop", raw] => {
                let line = parse_line(raw)?;
                if debugger.drop_breakpoint(listener, target, line)? {
                    println!("Breakpoint removed from source line {line}.");
                } else {
                    println!("No breakpoint removed from line {line}.");
                }
            }
            ["bl" | "breakpoints"] => print_breakpoints(debugger, listener)?,
            ["src" | "source"] => print_source(debugger, listener, target)?,
            ["n" | "next"] => match debugger.step_over(listener)? {
                DebugEvent::Paused(stop) => print_stop_and_state(debugger, listener, stop)?,
                DebugEvent::ChannelClosed => {
                    report_channel_closed("step");
                    return Ok(());
                }
            },
            ["v" | "vars"] => print_variables(debugger, listener)?,
            ["s" | "stack"] => print_stack(debugger, listener)?,
            ["c" | "continue"] => match debugger.continue_target(listener)? {
                DebugEvent::Paused(stop) => print_stop_and_state(debugger, listener, stop)?,
                DebugEvent::ChannelClosed => {
                    report_channel_closed("continue");
                    return Ok(());
                }
            },
            ["h" | "help"] => print_help(),
            [] => {}
            _ => {
                println!("Unknown command.");
                print_help();
            }
        }
    }
}

// Показываем результат SQL; при ошибке завершаемся с кодом 1.
fn report_target_outcome(outcome: TargetOutcome) -> Result<(), Box<dyn Error>> {
    match outcome {
        TargetOutcome::Succeeded(result) => {
            println!("Target SQL succeeded ({} row(s)).", result.rows.len());
            for row in result.rows {
                let values = row
                    .into_iter()
                    .map(|value| value.unwrap_or_else(|| "<NULL>".to_owned()))
                    .collect::<Vec<_>>();
                println!("  {}", values.join(" | "));
            }
            Ok(())
        }
        TargetOutcome::Failed { sqlstate, message } => {
            let sqlstate = sqlstate.as_deref().unwrap_or("<unavailable>");
            println!("Target SQL failed (SQLSTATE {sqlstate}): {message}");
            Err(io::Error::other("target SQL failed").into())
        }
    }
}
// Показываем доступные команды.
fn print_help() {
    println!(
        "Commands: next (n), continue (c), vars (v), stack (s), source (src), break LINE (b), drop LINE (d), breakpoints (bl), help (h)"
    );
}

fn print_source(
    debugger: &mut PldbgClient,
    listener: i32,
    target: &str,
) -> Result<(), postgres::Error> {
    let source = debugger.get_source(listener, target)?;
    for (index, line) in source.lines().enumerate() {
        println!("{:>3} | {line}", index + 1);
    }
    Ok(())
}

fn print_breakpoints(debugger: &mut PldbgClient, listener: i32) -> Result<(), postgres::Error> {
    let points = debugger.get_breakpoints(listener)?;
    if points.is_empty() {
        println!("No breakpoints in this session.");
    }
    for point in points {
        if point.line_number < 1 {
            println!(
                "{} (OID {}), на входе",
                point.target_name, point.function_oid
            );
        } else {
            println!(
                "{} (OID {}), исходная строка {}",
                point.target_name, point.function_oid, point.line_number
            );
        }
    }
    Ok(())
}

// Сообщаем только о закрытии канала, не о результате функции.
fn report_channel_closed(action: &str) {
    println!("Debug channel closed after {action}; target outcome is separate.");
}

// После остановки показываем позицию, стек и переменные.
fn print_stop_and_state(
    debugger: &mut PldbgClient,
    listener: i32,
    stop: Breakpoint,
) -> Result<(), Box<dyn Error>> {
    print_actual_stop(debugger, listener, &stop, "Stopped at")?;
    print_stack(debugger, listener)?;
    print_variables(debugger, listener)?;
    Ok(())
}

// pldbgapi сообщает номер на единицу больше строки оператора. Стек содержит
// фактическую строку текущего кадра, поэтому показываем её пользователю.
fn print_actual_stop(
    debugger: &mut PldbgClient,
    listener: i32,
    stop: &Breakpoint,
    label: &str,
) -> Result<(), postgres::Error> {
    let stack = debugger.get_stack(listener)?;
    if let Some(frame) = stack.iter().find(|frame| frame.level == 0) {
        println!("{label} {}:{}", frame.target_name, frame.line_number);
    } else {
        println!(
            "{label} точная строка недоступна (pldbgapi сообщил {}:{})",
            stop.target_name, stop.line_number
        );
    }
    Ok(())
}

// Продолжаем после каждой остановки, пока канал не закроется.
fn continue_until_closed(debugger: &mut PldbgClient, listener: i32) -> Result<(), Box<dyn Error>> {
    loop {
        match debugger.continue_target(listener)? {
            DebugEvent::Paused(stop) => print_stop_and_state(debugger, listener, stop)?,
            DebugEvent::ChannelClosed => {
                report_channel_closed("continue");
                return Ok(());
            }
        }
    }
}
// Печатаем значения переменных на текущей остановке.
fn print_variables(debugger: &mut PldbgClient, listener: i32) -> Result<(), postgres::Error> {
    println!("Variables:");
    for variable in debugger.get_variables(listener)? {
        let value = variable.value.as_deref().unwrap_or("<NULL>");
        let suffix = if variable.value_truncated {
            " … [показан фрагмент]"
        } else {
            ""
        };
        println!(
            "  {} (declared at line {}): {value}{suffix}",
            variable.name, variable.declaration_line
        );
    }
    Ok(())
}

// Печатаем кадры стека на текущей остановке.
fn print_stack(debugger: &mut PldbgClient, listener: i32) -> Result<(), postgres::Error> {
    println!("Call stack:");
    for frame in debugger.get_stack(listener)? {
        let arguments = frame.arguments.as_deref().unwrap_or("<none>");
        let suffix = if frame.arguments_truncated {
            " … [показан фрагмент]"
        } else {
            ""
        };
        println!(
            "  #{} {} (line {}, args: {arguments}{suffix})",
            frame.level, frame.target_name, frame.line_number
        );
    }
    Ok(())
}
