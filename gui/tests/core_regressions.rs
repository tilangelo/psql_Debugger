use pg_debugger_gui::{
    catalog::{Argument, Connection, RoutineKind},
    outcome::RollbackOutcome,
    session::{Call, Event, Session},
};
use postgres::{Client, Config, NoTls};
use std::{collections::BTreeSet, time::Duration};
#[path = "../../src/preflight.rs"]
mod preflight;

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

#[test]
#[ignore = "requires the isolated GUI PostgreSQL fixture"]
fn variable_and_stack_previews_are_bounded() {
    let (connection, mut client) = fixture();
    client.batch_execute("CREATE FUNCTION public.core_limits(input_text text) RETURNS integer LANGUAGE plpgsql AS $$ DECLARE payload text := repeat('Ж', 1000000); BEGIN RETURN length(payload); END; $$;").unwrap();
    let oid: u32 = client
        .query_one("SELECT 'public.core_limits(text)'::regprocedure::oid", &[])
        .unwrap()
        .get(0);
    let arg = Argument {
        name: Some("input_text".into()),
        type_name: "text".into(),
        mode: "IN".into(),
        type_oid: 25,
        custom_type: None,
        has_default: false,
    };
    let call = Call::new(
        oid,
        RoutineKind::Function,
        "public",
        "core_limits",
        &[arg],
        vec![Some("Ж".repeat(500000))],
    )
    .unwrap();
    let session = Session::start(connection, call, BTreeSet::new()).unwrap();
    match session
        .events
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
    {
        Event::Paused {
            variables, stack, ..
        } => {
            for name in ["payload", "input_text"] {
                let variable = variables.iter().find(|v| v.name == name).unwrap();
                assert!(variable.value_truncated);
                let text = variable.value.as_ref().unwrap();
                assert_eq!(text.len(), pg_debugger::pldbg::INSPECTION_TEXT_BYTES);
                assert!(text.chars().all(|c| c == 'Ж'));
            }
            assert!(stack[0].arguments_truncated);
            assert!(
                stack[0].arguments.as_ref().unwrap().len()
                    <= pg_debugger::pldbg::INSPECTION_TEXT_BYTES
            );
        }
        _ => panic!("expected pause"),
    }
    session.stop();
    match session.events.recv_timeout(Duration::from_secs(8)).unwrap() {
        Event::Finished(outcome) => assert_eq!(outcome.rollback, RollbackOutcome::Confirmed),
        _ => panic!("expected completion"),
    }
}

#[test]
#[ignore = "requires the isolated GUI PostgreSQL fixture"]
fn validator_checks_body_even_if_role_disables_checking() {
    let (_, mut client) = fixture();
    let password = std::env::var("PGDEBUG_GUI_TEST_PASSWORD").unwrap();
    let literal: String = client
        .query_one("SELECT quote_literal($1::text)", &[&password])
        .unwrap()
        .get(0);
    client.batch_execute(&format!("CREATE ROLE core_validator LOGIN SUPERUSER PASSWORD {literal}; ALTER ROLE core_validator SET check_function_bodies=off; SET check_function_bodies=off; CREATE FUNCTION public.core_invalid() RETURNS integer LANGUAGE plpgsql AS $$ BEGIN RETURN ; BROKEN SYNTAX HERE; END; $$; CREATE FUNCTION public.core_valid() RETURNS integer LANGUAGE plpgsql AS $$ BEGIN RETURN 42; END; $$;")).unwrap();
    let url = format!(
        "host=127.0.0.1 port={} user=core_validator dbname=gui_plugin_test password={password}",
        std::env::var("PGDEBUG_GUI_TEST_PORT").unwrap()
    );
    assert!(preflight::check(&url, "public.core_invalid()").is_err());
    let report = preflight::check(&url, "public.core_valid()").unwrap();
    assert_eq!(report.kind, "f");
    assert!(report.source.contains("RETURN 42"));
    let mut role = Client::connect(&url, NoTls).unwrap();
    assert_eq!(
        role.query_one("SHOW check_function_bodies", &[])
            .unwrap()
            .get::<_, String>(0),
        "off"
    );
}
