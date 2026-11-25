use pg_debugger_gui::catalog::{Connection, Selection, browse};
use pg_debugger_gui::session::{Call, Command, Event, Session};
use postgres::{Config, NoTls};
use std::collections::BTreeSet;
use std::time::Duration;

#[test]
#[ignore = "requires examples/shop in the isolated gui_shop_test database"]
fn shop_metadata_and_transactional_calls() {
    let port = std::env::var("PGDEBUG_GUI_TEST_PORT").unwrap();
    let password = std::env::var("PGDEBUG_GUI_TEST_PASSWORD").unwrap();
    let connection = Connection::new(
        "127.0.0.1",
        &port,
        "gui_shop_test",
        "debugger_test",
        &password,
    )
    .unwrap();
    let mut observer = Config::new()
        .host("127.0.0.1")
        .port(port.parse().unwrap())
        .user("debugger_test")
        .password(password)
        .dbname("gui_shop_test")
        .connect(NoTls)
        .unwrap();
    let load = |schema: &str| {
        browse(
            &connection,
            &Selection {
                schema: Some(schema.into()),
                routine_oid: None,
            },
        )
        .unwrap()
    };
    for schema in ["sales", "billing"] {
        let catalog = load(schema);
        let rows = observer
            .query(
                "SELECT oid, routine_type::text, prosrc FROM orm.functions_view
             WHERE routine_schema::text = $1 AND lanname = 'plpgsql'",
                &[&schema],
            )
            .unwrap();
        assert_eq!(rows.len(), catalog.routines.len());
        for routine in catalog.routines {
            let row = rows
                .iter()
                .find(|row| row.get::<_, u32>(0) == routine.oid)
                .unwrap();
            assert_eq!(
                row.get::<_, String>(1),
                if schema == "billing" {
                    "PROCEDURE"
                } else {
                    "FUNCTION"
                }
            );
            let detail = browse(
                &connection,
                &Selection {
                    schema: Some(schema.into()),
                    routine_oid: Some(routine.oid),
                },
            )
            .unwrap()
            .selected
            .unwrap();
            assert_eq!(row.get::<_, String>(2), detail.source);
        }
    }
    let baseline = snapshot(&mut observer);
    let call = |schema: &str,
                name: &str,
                input_type: u32,
                values: Vec<Option<String>>,
                stop_after_update: bool| {
        let catalog = load(schema);
        for routine in catalog.routines.iter().filter(|item| item.name == name) {
            let detail = browse(
                &connection,
                &Selection {
                    schema: Some(schema.into()),
                    routine_oid: Some(routine.oid),
                },
            )
            .unwrap()
            .selected
            .unwrap();
            if detail.arguments[0].type_oid != input_type {
                continue;
            }
            let mut points = BTreeSet::new();
            if stop_after_update {
                let line = detail
                    .source
                    .lines()
                    .position(|line| line.contains("INSERT INTO billing.payments"))
                    .unwrap() as i32
                    + 1;
                points.insert(line);
            }
            let call = Call::new(
                routine.oid,
                routine.kind,
                schema,
                name,
                &detail.arguments,
                values,
            )
            .unwrap();
            return Session::start(connection.clone(), call, points).unwrap();
        }
        panic!("fixture routine not found");
    };
    for (ty, value) in [(23, "1001"), (25, "ORDER-1001")] {
        let session = call("sales", "order_total", ty, vec![Some(value.into())], false);
        pause(&session);
        session.command(Command::Continue);
        assert!(finish(&session).contains("Результат:\n5990\nТранзакция откачена."));
    }
    for (amount, reference, expected) in [
        ("5990", "NEW-PAYMENT", "receipt = PAY-1001"),
        ("1", "NEW-PAYMENT", "SQLSTATE 22023"),
        ("5990", "EXISTING-PAYMENT", "SQLSTATE 23505"),
    ] {
        let values = vec![
            Some("1001".into()),
            Some(amount.into()),
            Some(reference.into()),
            None,
        ];
        let session = call("billing", "pay_order", 23, values, false);
        pause(&session);
        session.command(Command::Continue);
        let report = finish(&session);
        assert!(
            report.contains(expected) && report.contains("Транзакция откачена."),
            "{report}"
        );
        assert_eq!(snapshot(&mut observer), baseline);
    }
    let values = vec![
        Some("1001".into()),
        Some("5990".into()),
        Some("CANCELLED".into()),
        None,
    ];
    let session = call("billing", "pay_order", 23, values, true);
    pause(&session);
    session.stop();
    assert!(finish(&session).contains("Транзакция откачена."));
    assert_eq!(snapshot(&mut observer), baseline);
}

fn snapshot(client: &mut postgres::Client) -> String {
    client
        .query_one(
            "SELECT jsonb_build_object(
            'orders', (SELECT jsonb_agg(to_jsonb(o) ORDER BY o.id) FROM sales.orders o),
            'payments', (SELECT jsonb_agg(to_jsonb(p) ORDER BY p.id) FROM billing.payments p),
            'audit', (SELECT jsonb_agg(to_jsonb(a) ORDER BY a.id) FROM audit.events a)
         )::text",
            &[],
        )
        .unwrap()
        .get(0)
}

#[test]
#[ignore = "requires the isolated gui_shop_test database"]
fn shop_breakpoints_follow_statements_and_branches() {
    let connection = Connection::new(
        "127.0.0.1",
        &std::env::var("PGDEBUG_GUI_TEST_PORT").unwrap(),
        "gui_shop_test",
        "debugger_test",
        &std::env::var("PGDEBUG_GUI_TEST_PASSWORD").unwrap(),
    )
    .unwrap();
    let catalog = browse(
        &connection,
        &Selection {
            schema: Some("sales".into()),
            routine_oid: None,
        },
    )
    .unwrap();
    let (routine, detail) = catalog
        .routines
        .iter()
        .filter(|r| r.name == "order_total")
        .find_map(|routine| {
            let detail = browse(
                &connection,
                &Selection {
                    schema: Some("sales".into()),
                    routine_oid: Some(routine.oid),
                },
            )
            .unwrap()
            .selected
            .unwrap();
            (detail.arguments[0].type_oid == 23).then_some((routine, detail))
        })
        .unwrap();
    let start = |value: &str| {
        let call = Call::new(
            routine.oid,
            routine.kind,
            "sales",
            &routine.name,
            &detail.arguments,
            vec![Some(value.into())],
        )
        .unwrap();
        Session::start(connection.clone(), call, BTreeSet::from([6, 9, 11])).unwrap()
    };
    let assert_line = |session: &Session, expected| match session
        .events
        .recv_timeout(Duration::from_secs(12))
        .unwrap()
    {
        Event::Paused { stack, .. } => assert_eq!(stack[0].line_number, expected),
        Event::Finished(outcome) => panic!("{outcome:?}"),
        Event::Warning(message) => panic!("{message}"),

        Event::BreakpointRejected { message, .. } => panic!("{message}"),
        Event::FrameSelected { .. } | Event::NestedBreakpointsChanged { .. } => {
            panic!("unexpected nested debug event")
        }
    };
    let session = start("1001");
    assert_line(&session, 5);
    session.command(Command::Continue);
    assert_line(&session, 9);
    session.command(Command::Continue);
    assert!(finish(&session).contains("Результат:\n5990"));

    // Ветка RAISE достижима только для отсутствующего заказа.
    let session = start("9999");
    for expected in [5, 9, 10] {
        assert_line(&session, expected);
        session.command(Command::Continue);
    }
    assert!(finish(&session).contains("SQLSTATE P0002"));

    // Изменение и удаление перенесённых точек во время паузы.
    let session = start("1001");
    assert_line(&session, 5);
    session.command(Command::Breakpoint {
        line: 6,
        enabled: false,
    });
    session.command(Command::Breakpoint {
        line: 9,
        enabled: false,
    });
    session.command(Command::Breakpoint {
        line: 13,
        enabled: true,
    });
    session.command(Command::Continue);
    assert_line(&session, 13);
    session.command(Command::Continue);
    assert!(finish(&session).contains("Результат:\n5990"));
}

fn pause(session: &Session) {
    match session
        .events
        .recv_timeout(Duration::from_secs(12))
        .unwrap()
    {
        Event::Paused { .. } => {}
        Event::Finished(outcome) => panic!("{outcome:?}"),
        Event::Warning(text) => panic!("{text}"),

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
        .unwrap()
    {
        Event::Finished(outcome) => outcome.render(),
        Event::Warning(text) => panic!("{text}"),

        Event::BreakpointRejected { message, .. } => panic!("{message}"),
        Event::FrameSelected { .. } | Event::NestedBreakpointsChanged { .. } => {
            panic!("unexpected nested debug event")
        }
        Event::Paused { .. } => panic!("unexpected pause"),
    }
}
