use pg_debugger_core::project::Project;
use serde_json::{Value, json};
fn sample() -> Value {
    json!({"schemaVersion":1,"kind":"function","schema":"public","name":"visual_smoke","returnType":"integer","parameters":[{"name":"input_value","type":"integer"}],"variables":[{"name":"running_total","type":"integer","initial":"input_value"}],"blocks":[{"id":"a","type":"assign","target":"running_total","expression":"running_total + 1"},{"id":"b","type":"assign","target":"running_total","expression":"running_total * 2"},{"id":"r","type":"return","expression":"running_total"}]})
}
fn parse(v: Value) -> Project {
    serde_json::from_value(v).unwrap()
}
#[test]
fn select_settings_are_typed_and_do_not_replace_query() {
    let mut value = sample();
    value["schemaVersion"] = json!(3);
    value["model"] = json!({"name":"Model","tables":[]});
    value["blocks"][0] = json!({"id":"s","type":"select","target":"running_total","query":"SELECT 123","assistant":{"schema":"sales","table":"orders","mode":"value","column":{"side":"source","name":"id"},"filter":null,"join":null}});
    assert!(
        parse(value.clone())
            .generate()
            .unwrap()
            .sql
            .contains("SELECT 123")
    );
    let encoded = serde_json::to_value(parse(value.clone())).unwrap();
    assert_eq!(encoded["blocks"][0]["assistant"]["table"], "orders");
    value["schemaVersion"] = json!(2);
    assert!(parse(value.clone()).generate().is_err());
    value["schemaVersion"] = json!(3);
    value["blocks"][0]["assistant"]["oid"] = json!(123);
    assert!(serde_json::from_value::<Project>(value).is_err());
}
#[test]
fn legacy_and_embedded_hints_generate_identical_sql() {
    let old = parse(sample()).generate().unwrap();
    let mut next = sample();
    next["schemaVersion"] = json!(2);
    next["model"] = json!({"name":"Заказы","tables":[{"schema":"sales","name":"orders","label":"DROP TABLE important;","description":"Подсказка","columns":[{"name":"status","values":[{"value":"new","label":"Новый"}]}]}]});
    let generated = parse(next.clone()).generate().unwrap();
    assert_eq!(old.sql, generated.sql);
    assert_eq!(old.body, generated.body);
    let encoded = serde_json::to_value(parse(next.clone())).unwrap();
    assert_eq!(
        encoded["model"]["tables"][0]["columns"][0]["values"][0]["value"],
        "new"
    );
    next["model"]["tables"][0]["columns"][0]["sql"] = json!("DROP TABLE data");
    assert!(serde_json::from_value::<Project>(next).is_err());
}
#[test]
fn hints_reject_duplicates_and_unsupported_versions() {
    let mut next = sample();
    next["schemaVersion"] = json!(2);
    assert!(parse(next.clone()).generate().is_err());
    next["model"] = json!({"name":"Model","tables":[{"schema":"sales","name":"orders","columns":[{"name":"status","values":[{"value":"new","label":"A"},{"value":"new","label":"B"}]}]}]});
    assert!(parse(next.clone()).generate().is_err());
    next["model"] = json!({"name":"Model","tables":[]});
    next["schemaVersion"] = json!(1);
    assert!(parse(next.clone()).generate().is_err());
    next["schemaVersion"] = json!(4);
    assert!(parse(next).generate().is_err());
}
#[test]
fn definition_and_source_mapping() {
    let p = parse(sample());
    let g = p.generate().unwrap();
    assert!(g.sql.contains("RETURNS pg_catalog.int4"));
    assert!(g.signature.ends_with("(pg_catalog.int4)"));
    let range = &g.source_map["a"];
    assert_eq!(range.body_start, 5);
    assert!(
        g.sql
            .lines()
            .nth(range.start - 1)
            .unwrap()
            .contains("running_total + 1")
    );
}
#[test]
fn procedure_without_return_value() {
    let mut p = sample();
    p["kind"] = json!("procedure");
    let g = parse(p).generate().unwrap();
    assert!(!g.sql.contains("RETURNS"));
    assert!(g.sql.contains("RETURN;"));
}
#[test]
fn malformed_structure_and_duplicate_ids() {
    let mut p = sample();
    p["blocks"][1]["id"] = json!("a");
    assert!(parse(p).generate().is_err());
    let mut p = sample();
    p["blocks"][0]["target"] = json!("missing");
    assert!(parse(p).generate().is_err());
    let mut p = sample();
    p["blocks"] = json!([]);
    assert!(parse(p).generate().is_err());
}
#[test]
fn unknown_fields_and_oversized_identifier() {
    let mut p = sample();
    p["sql"] = json!("DROP TABLE data");
    assert!(serde_json::from_value::<Project>(p).is_err());
    let mut p = sample();
    p["name"] = json!("x".repeat(64));
    assert!(parse(p).generate().is_err());
}
#[test]
fn both_branches_and_safe_body_delimiter() {
    let mut p = sample();
    p["blocks"] = json!([{"id":"if","type":"if","condition":"input_value > 0","then":[{"id":"r1","type":"return","expression":"1"}],"else":[{"id":"r2","type":"return","expression":"2"}]}]);
    assert!(parse(p).generate().is_ok());
    let mut p = sample();
    p["blocks"][0] = json!({"id":"sql","type":"sql","sql":"RAISE NOTICE '$body$';"});
    assert!(parse(p).generate().unwrap().sql.contains("AS $body_$"));
}
