use pg_debugger_core::{catalog::Argument, project::Project};
use serde_json::{Value, json};

#[test]
fn shared_scalar_cases_and_project_round_trip() {
    let cases: Value = serde_json::from_str(include_str!("fixtures/scalar-values.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let arg = Argument {
            name: Some("input_value".into()),
            type_name: case["type"].as_str().unwrap().into(),
            type_oid: case["oid"].as_u64().unwrap() as u32,
            mode: "IN".into(),
            custom_type: None,
            has_default: false,
        };
        assert!(arg.accepts_input());
        for value in case["valid"].as_array().unwrap() {
            assert!(arg.value_is_valid(value.as_str().unwrap()), "{case}");
        }
        for value in case["invalid"].as_array().unwrap() {
            assert!(!arg.value_is_valid(value.as_str().unwrap()), "{case}");
        }
        let draft = json!({"schemaVersion":3,"model":{"name":"","tables":[]},"kind":"function","schema":"public","name":"typed_echo","returnType":case["type"],"parameters":[{"name":"input_value","type":case["type"]}],"variables":[{"name":"result_value","type":case["type"],"initial":"input_value"}],"blocks":[{"id":"r","type":"return","expression":"result_value"}]});
        let project: Project = serde_json::from_value(draft.clone()).unwrap();
        assert_eq!(serde_json::to_value(&project).unwrap(), draft);
        let generated = project.generate().unwrap();
        let pg_type = postgres::types::Type::from_oid(arg.type_oid).unwrap();
        let canonical = pg_type.name();
        assert_eq!(
            generated.signature,
            format!("\"public\".\"typed_echo\"(pg_catalog.{canonical})")
        );
        assert!(generated.body.contains("pg_catalog."));
    }
}

#[test]
fn multiline_initial_expression_maps_return_line() {
    let draft = json!({"schemaVersion":3,"model":{"name":"","tables":[]},"kind":"function","schema":"public","name":"typed_echo","returnType":"numeric","parameters":[{"name":"input_value","type":"numeric"}],"variables":[{"name":"result_value","type":"numeric","initial":"(\ninput_value\n)"}],"blocks":[{"id":"r","type":"return","expression":"result_value"}]});
    let project: Project = serde_json::from_value(draft).unwrap();
    let generated = project.generate().unwrap();
    let range = &generated.source_map["r"];
    assert!(
        generated
            .body
            .lines()
            .nth(range.body_start - 1)
            .unwrap()
            .contains("RETURN")
    );
}

#[test]
fn multiline_block_expressions_map_following_statements() {
    let draft = json!({"schemaVersion":3,"model":{"name":"","tables":[]},"kind":"function","schema":"public","name":"multiline","returnType":"integer","parameters":[],"variables":[{"name":"x","type":"integer","initial":"0"}],"blocks":[{"id":"a","type":"assign","target":"x","expression":"(\n1 + 2\n)"},{"id":"r","type":"return","expression":"(\nx\n)"}]});
    let project: Project = serde_json::from_value(draft).unwrap();
    let generated = project.generate().unwrap();
    for (id, needle) in [("a", "\"x\" :="), ("r", "RETURN")] {
        let range = &generated.source_map[id];
        assert!(
            generated
                .body
                .lines()
                .nth(range.body_start - 1)
                .unwrap()
                .contains(needle)
        );
        assert_eq!(range.body_end - range.body_start, 2);
    }
}
