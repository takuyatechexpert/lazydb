use super::*;
use crate::db::adapter::QueryResult;

fn make_result(columns: Vec<&str>, rows: Vec<Vec<&str>>) -> QueryResult {
    QueryResult {
        columns: columns.into_iter().map(String::from).collect(),
        rows: rows
            .into_iter()
            .map(|r| r.into_iter().map(String::from).collect())
            .collect(),
        duration_ms: 0,
    }
}

// ── to_csv ──

#[test]
fn to_csv_outputs_header_and_rows() {
    let result = make_result(vec!["id", "name"], vec![vec!["1", "Alice"], vec!["2", "Bob"]]);
    let csv = to_csv(&result).unwrap();
    assert_eq!(csv, "id,name\n1,Alice\n2,Bob\n");
}

#[test]
fn to_csv_empty_result_outputs_header_only() {
    let result = make_result(vec!["id", "name"], vec![]);
    let csv = to_csv(&result).unwrap();
    assert_eq!(csv, "id,name\n");
}

#[test]
fn to_csv_escapes_comma_in_value() {
    let result = make_result(vec!["name"], vec![vec!["Alice, Bob"]]);
    let csv = to_csv(&result).unwrap();
    assert!(csv.contains("\"Alice, Bob\""));
}

// ── to_json ──

#[test]
fn to_json_outputs_array_of_objects() {
    let result = make_result(vec!["id", "name"], vec![vec!["1", "Alice"]]);
    let json = to_json(&result).unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0]["id"], serde_json::json!(1));
    assert_eq!(parsed[0]["name"], serde_json::json!("Alice"));
}

// ── parse_json_value ──

#[test]
fn parse_json_value_true() {
    assert_eq!(parse_json_value("t"), serde_json::Value::Bool(true));
    assert_eq!(parse_json_value("true"), serde_json::Value::Bool(true));
}

#[test]
fn parse_json_value_false() {
    assert_eq!(parse_json_value("f"), serde_json::Value::Bool(false));
    assert_eq!(parse_json_value("false"), serde_json::Value::Bool(false));
}

#[test]
fn parse_json_value_integer() {
    assert_eq!(parse_json_value("42"), serde_json::json!(42));
}

#[test]
fn parse_json_value_float() {
    assert_eq!(parse_json_value("3.14"), serde_json::json!(3.14));
}

#[test]
fn parse_json_value_string() {
    assert_eq!(
        parse_json_value("hello"),
        serde_json::Value::String("hello".to_string())
    );
}

#[test]
fn parse_json_value_empty_is_null() {
    assert_eq!(parse_json_value(""), serde_json::Value::Null);
}
