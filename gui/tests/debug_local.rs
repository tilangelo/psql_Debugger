use pg_debugger_gui::catalog::{Connection, Selection, browse};
use pg_debugger_gui::session::{Call, Command, Event, Session};
use postgres::{Config, NoTls};
use std::collections::BTreeSet;
use std::time::Duration;

#[test]
#[ignore = "requires the isolated debugger fixture documented in the GUI README"]
fn local_debugging_and_rollback() {
    let port = std::env::var("PGDEBUG_GUI_TEST_PORT").expect("fixture port");
    let password = std::env::var("PGDEBUG_GUI_TEST_PASSWORD").unwrap_or_default();
    let connection = Connection::new(
        "127.0.0.1",
        &port,
        "gui_plugin_test",
        "debugger_test",
        &password,
    )
    .unwrap();
    let mut observer = Config::new()
        .host("127.0.0.1")
        .port(port.parse().unwrap())
        .dbname("gui_plugin_test")
        .user("debugger_test")
        .password(&password)
        .connect(NoTls)
        .unwrap();
    let start = |name, lines: BTreeSet<i32>| {
        let selection = Selection {
            schema: Some("public".into()),
            routine_oid: None,
        };
        let catalog = browse(&connection, &selection).unwrap();
        let item = catalog
            .routines
            .iter()
            .find(|item| item.name == name)
            .expect("fixture routine");
        let detail = browse(
            &connection,
            &Selection {
                routine_oid: Some(item.oid),
                ..selection
            },
        )
        .unwrap()
        .selected
        .expect("routine details");
        let values = detail
            .arguments
            .iter()
            .map(|arg| arg.accepts_input().then(|| "3".to_owned()))
            .collect();
        let call = Call::new(
            item.oid,
            item.kind,
            "public",
            &item.name,
            &detail.arguments,
            values,
        )
        .unwrap();
        Session::start(connection.clone(), call, lines).unwrap()
    };
    let verify_no_writes = |client: &mut postgres::Client| {
        let count: i64 = client
            .query_one("SELECT count(*) FROM public.debug_writes", &[])
            .unwrap()
            .get(0);
        assert_eq!(count, 0);
    };

    let function = start("debug_smoke", BTreeSet::new());
    assert_pause(&function, 5, Some("3"));
    function.command(Command::Step);
    assert_pause(&function, 6, Some("4"));
    function.command(Command::Continue);
    assert!(finish(&function).contains("Результат:\n8\nТранзакция откачена."));

    let before_begin = start("debug_smoke", BTreeSet::from([4]));
    assert_pause(&before_begin, 5, Some("3"));
    before_begin.command(Command::Continue);
    assert!(finish(&before_begin).contains("Результат:\n8"));

    let function = start("debug_smoke", BTreeSet::from([6]));
    assert_pause(&function, 6, Some("4"));
    function.command(Command::Breakpoint {
        line: 7,
        enabled: true,
    });
    function.command(Command::Continue);
    assert_pause(&function, 7, Some("8"));
    function.command(Command::Continue);
    assert!(finish(&function).contains("Результат:\n8"));

    let procedure = start("write_for_debug", BTreeSet::new());
    assert_pause(&procedure, 3, None);
    procedure.command(Command::Step);
    assert_pause(&procedure, 4, None);
    procedure.stop();
    let report = finish(&procedure);
    assert!(
        report.contains("Запрошена остановка") && report.contains("Транзакция откачена."),
        "{report}"
    );
    verify_no_writes(&mut observer);

    let retry = start("write_for_debug", BTreeSet::new());
    assert_pause(&retry, 3, None);
    retry.command(Command::Continue);
    assert!(finish(&retry).contains("result = 4"));
    verify_no_writes(&mut observer);

    let failing = start("debug_failure", BTreeSet::new());
    assert_pause(&failing, 5, Some("3"));
    failing.command(Command::Continue);
    let report = finish(&failing);
    assert!(
        report.contains("SQLSTATE P0001") && report.contains("Транзакция откачена."),
        "{report}"
    );

    let commit = start("write_then_commit", BTreeSet::new());
    assert_pause(&commit, 3, None);
    commit.command(Command::Continue);
    let report = finish(&commit);
    assert!(
        report.contains("SQLSTATE 2D000") && report.contains("Транзакция откачена."),
        "{report}"
    );
    verify_no_writes(&mut observer);

    let running = start("write_then_wait", BTreeSet::new());
    assert_pause(&running, 3, None);
    running.command(Command::Continue);
    std::thread::sleep(Duration::from_millis(250));
    running.stop();
    let report = finish(&running);
    assert!(
        report.contains("Запрошена остановка") && report.contains("Транзакция откачена."),
        "{report}"
    );
    verify_no_writes(&mut observer);
}

fn assert_pause(session: &Session, line: i32, total: Option<&str>) {
    match session
        .events
        .recv_timeout(Duration::from_secs(12))
        .expect("pause")
    {
        Event::Paused {
            stack, variables, ..
        } => {
            assert_eq!(stack[0].line_number, line);
            if let Some(total) = total {
                assert!(
                    variables
                        .iter()
                        .any(|var| var.name == "running_total"
                            && var.value.as_deref() == Some(total))
                );
            }
        }
        Event::Finished(outcome) => panic!("{outcome:?}"),
        Event::Warning(report) => panic!("ожидалась остановка: {report}"),

        Event::BreakpointRejected { message, .. } => panic!("{message}"),
        Event::FrameSelected { .. } | Event::NestedBreakpointsChanged { .. } => {
            panic!("unexpected nested debug event")
        }
    }
}

fn finish(session: &Session) -> String {
    match session
        .events
        .recv_timeout(Duration::from_secs(12))
        .expect("result")
    {
        Event::Finished(outcome) => outcome.render(),
        Event::Warning(report) => panic!("неожиданное предупреждение: {report}"),

        Event::BreakpointRejected { message, .. } => panic!("{message}"),
        Event::FrameSelected { .. } | Event::NestedBreakpointsChanged { .. } => {
            panic!("unexpected nested debug event")
        }
        Event::Paused { position, .. } => panic!("неожиданная остановка: {}", position.line_number),
    }
}
