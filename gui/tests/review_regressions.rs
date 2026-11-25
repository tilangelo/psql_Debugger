use pg_debugger_gui::outcome::{CallOutcome, RollbackOutcome, SessionOutcome};
use pg_debugger_gui::{
    catalog::{Connection, RoutineKind},
    session::{Call, Command, Event, Session},
};
use postgres::{Client, Config, NoTls};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

fn fixture() -> (Connection, Client) {
    let port = std::env::var("PGDEBUG_GUI_TEST_PORT").unwrap();
    let password = std::env::var("PGDEBUG_GUI_TEST_PASSWORD").unwrap();
    let connection = Connection::new(
        "127.0.0.1",
        &port,
        "gui_plugin_test",
        "debugger_test",
        &password,
    )
    .unwrap();
    let client = Config::new()
        .host("127.0.0.1")
        .port(port.parse().unwrap())
        .user("debugger_test")
        .password(password)
        .dbname("gui_plugin_test")
        .connect(NoTls)
        .unwrap();
    (connection, client)
}

fn call(client: &mut Client, name: &str) -> Call {
    let row = client
        .query_one(
            "SELECT oid, pg_get_functiondef(oid) FROM pg_proc WHERE proname=$1",
            &[&name],
        )
        .unwrap();
    Call::new(
        row.get(0),
        RoutineKind::Function,
        "public",
        name,
        &[],
        vec![],
    )
    .unwrap()
    .with_definition(row.get(1))
}

fn complete_outcome(session: &Session) -> SessionOutcome {
    loop {
        match session
            .events
            .recv_timeout(Duration::from_secs(35))
            .unwrap()
        {
            Event::Paused { .. } => session.command(Command::Continue),
            Event::Finished(report) => return report,
            Event::Warning(_) => {}
            _ => panic!("unexpected event"),
        }
    }
}

fn complete(session: &Session) -> String {
    complete_outcome(session).render()
}

#[test]
#[ignore = "requires the isolated GUI PostgreSQL fixture"]
fn bounded_results_and_error_context() {
    let (connection, mut client) = fixture();
    client.batch_execute("CREATE FUNCTION public.review_many() RETURNS SETOF integer LANGUAGE plpgsql AS $$ BEGIN RETURN QUERY SELECT generate_series(1, 10000); END; $$;
    CREATE FUNCTION public.review_big() RETURNS text LANGUAGE plpgsql AS $$ BEGIN RETURN repeat('Ж', 1000000); END; $$;
    CREATE FUNCTION public.review_error() RETURNS integer LANGUAGE plpgsql AS $$ BEGIN RETURN 1 / 0; END; $$;").unwrap();
    for (name, expected) in [
        ("review_many", "100 из 10000"),
        ("review_big", "объём текста ограничен"),
        ("review_error", "Контекст:"),
    ] {
        let session =
            Session::start(connection.clone(), call(&mut client, name), BTreeSet::new()).unwrap();
        let outcome = complete_outcome(&session);
        assert_eq!(outcome.rollback, RollbackOutcome::Confirmed);
        if name == "review_many" {
            let CallOutcome::Succeeded(preview) = &outcome.call else {
                panic!("expected rows");
            };
            assert_eq!(preview.total_rows, 10000);
            assert_eq!(preview.shown_rows, 100);
            assert!(preview.truncated);
        }
        if name == "review_error" {
            let CallOutcome::Failed(error) = &outcome.call else {
                panic!("expected SQL failure");
            };
            assert_eq!(error.sqlstate.as_deref(), Some("22012"));
            assert!(error.context.is_some());
        }
        let report = outcome.render();
        assert!(report.contains(expected), "{report}");
        assert!(report.contains("Транзакция откачена."));
        assert!(report.len() < 66000);
        if name == "review_error" {
            assert!(report.contains("review_error()") && report.contains("22012"));
        }
    }
    let password = std::env::var("PGDEBUG_GUI_TEST_PASSWORD").unwrap();
    let literal: String = client
        .query_one("SELECT quote_literal($1::text)", &[&password])
        .unwrap()
        .get(0);
    client.batch_execute(&format!("CREATE FUNCTION public.review_private_error() RETURNS integer LANGUAGE plpgsql AS $body$ BEGIN RAISE EXCEPTION 'diagnostic' USING DETAIL={literal}, HINT={literal}; END; $body$;")).unwrap();
    let session = Session::start(
        connection.clone(),
        call(&mut client, "review_private_error"),
        BTreeSet::new(),
    )
    .unwrap();
    let report = complete(&session);
    assert!(!report.contains(&password));
    assert!(report.contains("Подробности: [скрыто]") && report.contains("Подсказка: [скрыто]"));

    client.batch_execute("CREATE FUNCTION public.review_notices() RETURNS integer LANGUAGE plpgsql AS $$ BEGIN FOR i IN 1..1000 LOOP RAISE NOTICE '%', repeat('x', 5000); END LOOP; RETURN 1; END; $$;").unwrap();
    let session = Session::start(
        connection,
        call(&mut client, "review_notices"),
        BTreeSet::new(),
    )
    .unwrap();
    let report = complete(&session);
    assert!(report.contains("достигнут лимит 100") && report.len() < 420000);
}

#[test]
#[ignore = "requires the isolated GUI PostgreSQL fixture"]
fn definition_change_prevents_call() {
    let (connection, mut client) = fixture();
    client.batch_execute("CREATE FUNCTION public.review_changed() RETURNS integer LANGUAGE plpgsql AS $$ BEGIN RETURN 1; END; $$;").unwrap();
    let old = call(&mut client, "review_changed");
    client.batch_execute("CREATE OR REPLACE FUNCTION public.review_changed() RETURNS integer LANGUAGE plpgsql AS $$ BEGIN INSERT INTO public.debug_writes VALUES (999); RETURN 2; END; $$;").unwrap();
    let session = Session::start(connection, old, BTreeSet::new()).unwrap();
    let outcome = complete_outcome(&session);
    assert!(matches!(outcome.call, CallOutcome::DefinitionChanged));
    assert_eq!(outcome.rollback, RollbackOutcome::NotNeeded);
    assert!(outcome.render().contains("Вызов не выполнялся"));
    let count: i64 = client
        .query_one(
            "SELECT count(*) FROM public.debug_writes WHERE value=999",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(count, 0);
}

#[test]
#[ignore = "requires the isolated GUI PostgreSQL fixture; checks a 25-second initialization"]
fn slow_initialization_is_not_cancelled() {
    let (connection, mut client) = fixture();
    client.batch_execute("CREATE FUNCTION public.review_slow_entry() RETURNS integer LANGUAGE plpgsql AS $$ DECLARE waited integer := (SELECT 1 FROM pg_sleep(25)); BEGIN RETURN waited; END; $$;").unwrap();
    let session = Session::start(
        connection,
        call(&mut client, "review_slow_entry"),
        BTreeSet::new(),
    )
    .unwrap();
    let before = Instant::now();
    assert!(matches!(
        session
            .events
            .recv_timeout(Duration::from_secs(24))
            .unwrap(),
        Event::Warning(_)
    ));
    assert!(before.elapsed() >= Duration::from_secs(19));
    assert!(matches!(
        session
            .events
            .recv_timeout(Duration::from_secs(10))
            .unwrap(),
        Event::Paused { .. }
    ));
    session.command(Command::Continue);
    assert!(complete(&session).contains("Результат:\n1\nТранзакция откачена."));
}

#[test]
#[ignore = "requires the isolated GUI PostgreSQL fixture"]
fn cancellation_waits_for_target_cleanup() {
    let (connection, mut client) = fixture();
    client.batch_execute("CREATE FUNCTION public.review_cancel() RETURNS integer LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(30); RETURN 1; EXCEPTION WHEN query_canceled THEN PERFORM pg_sleep(2); RETURN 2; END; $$;").unwrap();
    let session = Session::start(
        connection,
        call(&mut client, "review_cancel"),
        BTreeSet::new(),
    )
    .unwrap();
    assert!(matches!(
        session.events.recv_timeout(Duration::from_secs(5)).unwrap(),
        Event::Paused { .. }
    ));
    session.command(Command::Continue);
    std::thread::sleep(Duration::from_millis(250));
    let before = Instant::now();
    session.stop();
    let outcome = complete_outcome(&session);
    assert!(outcome.cancellation_requested);
    assert_eq!(outcome.rollback, RollbackOutcome::Confirmed);
    let report = outcome.render();
    assert!(before.elapsed() >= Duration::from_millis(1500));
    assert!(report.contains("Транзакция откачена."), "{report}");
}
