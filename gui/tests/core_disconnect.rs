#![cfg(target_os = "linux")]
use pg_debugger::demo_linux::DemoServer;
use pg_debugger_gui::{
    catalog::{Connection, RoutineKind},
    outcome::{CallOutcome, RollbackOutcome},
    session::{Call, Event, Session},
};
use postgres::{Client, NoTls};
use std::{collections::BTreeSet, path::PathBuf, process::Command, time::Duration};

#[test]
#[ignore = "starts and immediately stops its own disposable server; requires a non-root runner"]
fn actual_connection_loss_finishes_paused_session() {
    let (mut server, password) = DemoServer::start_isolated().unwrap();
    let mut client = Client::connect(server.connection_string(), NoTls).unwrap();
    let data: String = client.query_one("SHOW data_directory", &[]).unwrap().get(0);
    let root = PathBuf::from(&data).parent().unwrap().to_owned();
    client.batch_execute("CREATE FUNCTION public.core_lost() RETURNS integer LANGUAGE plpgsql AS $$ BEGIN RETURN 1; END; $$;").unwrap();
    let oid: u32 = client
        .query_one("SELECT 'public.core_lost()'::regprocedure::oid", &[])
        .unwrap()
        .get(0);
    let connection = Connection::new(
        "127.0.0.1",
        &server.port().to_string(),
        "postgres",
        "debugger_test",
        &password,
    )
    .unwrap();
    let call = Call::new(
        oid,
        RoutineKind::Function,
        "public",
        "core_lost",
        &[],
        vec![],
    )
    .unwrap();
    let session = Session::start(connection, call, BTreeSet::new()).unwrap();
    assert!(matches!(
        session.events.recv_timeout(Duration::from_secs(8)).unwrap(),
        Event::Paused { .. }
    ));
    drop(client);
    let ctl = std::env::var_os("PGDEBUG_PG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/lib/postgresql/17"))
        .join("bin/pg_ctl");
    // Только собственный временный сервер: разрываем реальные соединения,
    // не принимая отправленный SIGTERM за доказательство выхода backend.
    assert!(
        Command::new(ctl)
            .args(["-D", &data, "-m", "immediate", "-w", "stop"])
            .status()
            .unwrap()
            .success()
    );
    match session.events.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::Finished(outcome) => {
            assert!(matches!(outcome.call, CallOutcome::Failed(_)));
            assert!(matches!(outcome.rollback, RollbackOutcome::Unconfirmed(_)));
            assert!(!outcome.cancellation_requested);
        }
        _ => panic!("connection loss must finish without a user command"),
    }
    server.shutdown().unwrap();
    assert!(!root.exists());
}
