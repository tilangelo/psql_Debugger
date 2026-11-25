use pg_debugger_gui::catalog::{Connection, DebuggerStatus, RoutineKind, Selection, browse};

#[test]
fn rejects_nonlocal_addresses_and_invalid_ports() {
    for (host, port, field) in [
        ("192.0.2.1", "5432", "Адрес"),
        ("127.0.0.1", "0", "Порт"),
        ("::1", "65536", "Порт"),
    ] {
        match Connection::new(host, port, "postgres", "tester", "") {
            Err(error) => assert_eq!(error.field, field),
            Ok(_) => panic!("неверное поле принято"),
        }
    }
    assert!(Connection::new("::1", "5432", "postgres", "tester", "").is_ok());
}

#[test]
#[ignore = "requires the isolated PostgreSQL fixture documented in the GUI README"]
fn local_catalog_without_orm() {
    let port = std::env::var("PGDEBUG_GUI_TEST_PORT").expect("fixture port");
    let password = std::env::var("PGDEBUG_GUI_TEST_PASSWORD").expect("fixture password");
    let connect =
        |db, password| Connection::new("localhost", &port, db, "debugger_test", password).unwrap();
    let connection = connect("gui_catalog_test", &password);
    let selection = Selection {
        schema: Some("gui_checks".into()),
        routine_oid: None,
    };
    let catalog = browse(&connection, &selection).expect("catalog");
    assert_eq!(catalog.debugger, DebuggerStatus::MissingExtension);
    assert!(
        catalog
            .databases
            .iter()
            .any(|name| name == "gui_plugin_test")
    );
    assert!(catalog.schemas.iter().all(|name| name != "orm"));
    assert_eq!(catalog.schema.as_deref(), Some("gui_checks"));
    assert!(
        catalog
            .routines
            .iter()
            .all(|item| item.name != "sql_function")
    );

    let functions: Vec<_> = catalog
        .routines
        .iter()
        .filter(|item| item.name == "calculate")
        .collect();
    assert_eq!(functions.len(), 2);
    assert_ne!(functions[0].oid, functions[1].oid);
    assert_ne!(functions[0].signature, functions[1].signature);
    let procedure = catalog
        .routines
        .iter()
        .find(|item| item.name == "process")
        .expect("procedure");
    assert_eq!(procedure.kind, RoutineKind::Procedure);

    let details = browse(
        &connection,
        &Selection {
            routine_oid: Some(procedure.oid),
            ..selection.clone()
        },
    )
    .expect("procedure metadata")
    .selected
    .expect("selected procedure");
    assert!(details.source.contains("counter := counter + input_value"));
    assert_eq!(
        details
            .arguments
            .iter()
            .map(|arg| arg.mode.as_str())
            .collect::<Vec<_>>(),
        ["IN", "INOUT", "OUT"]
    );
    assert_eq!(details.arguments[0].name.as_deref(), Some("input_value"));
    assert!(details.arguments[0].value_is_valid("42"));
    assert!(!details.arguments[0].value_is_valid("2147483648"));
    assert!(!details.arguments[2].accepts_input());

    let no_args = catalog
        .routines
        .iter()
        .find(|item| item.name == "no_args")
        .expect("function");
    let details = browse(
        &connection,
        &Selection {
            routine_oid: Some(no_args.oid),
            ..selection
        },
    )
    .expect("function metadata")
    .selected
    .expect("selected function");
    assert!(details.arguments.is_empty());

    let second = browse(
        &connect("gui_plugin_test", &password),
        &Selection::default(),
    )
    .expect("second database");
    assert_eq!(second.debugger, DebuggerStatus::PluginConfigured);
    assert!(second.schemas.iter().all(|name| name != "gui_checks"));

    let read_only = |database| {
        let connection =
            Connection::new("localhost", &port, database, "gui_reader", &password).unwrap();
        browse(&connection, &Selection::default()).unwrap()
    };
    let reader = read_only("gui_plugin_test");
    assert_eq!(reader.debugger, DebuggerStatus::PluginUnverified);
    assert!(
        reader
            .routines
            .iter()
            .any(|item| item.name == "reader_owned" && item.debug_allowed)
    );
    assert!(
        reader
            .routines
            .iter()
            .any(|item| item.name == "debug_smoke" && !item.debug_allowed)
    );
    assert_eq!(
        read_only("gui_denied_test").debugger,
        DebuggerStatus::AccessDenied
    );

    for (db, pwd, expected) in [
        ("gui_catalog_test", "wrong-gui-test-password", "28P01"),
        ("does_not_exist", password.as_str(), "3D000"),
    ] {
        let result = browse(&connect(db, pwd), &Selection::default());
        let Err(error) = result else {
            panic!("expected SQLSTATE {expected}")
        };
        assert!(error.detail.contains(expected), "{error:?}");
        assert!(!error.detail.contains(pwd));
    }
}
