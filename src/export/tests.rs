use super::*;
use crate::db::adapter::QueryResult;

fn make_result(columns: Vec<&str>, rows: Vec<Vec<&str>>) -> QueryResult {
    QueryResult {
        columns: columns.into_iter().map(String::from).collect(),
        rows: rows
            .into_iter()
            .map(|r| r.into_iter().map(|v| Some(v.to_string())).collect())
            .collect(),
        duration_ms: 0,
    }
}

/// `Option<&str>` を直接受け付けるヘルパー（NULL を含む結果を組み立てる用）
fn make_result_opt(columns: Vec<&str>, rows: Vec<Vec<Option<&str>>>) -> QueryResult {
    QueryResult {
        columns: columns.into_iter().map(String::from).collect(),
        rows: rows
            .into_iter()
            .map(|r| r.into_iter().map(|v| v.map(String::from)).collect())
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

/// NULL 値（`None`）は JSON では `null` として出力される（空文字列とは区別される）
#[test]
fn to_json_null_value_serializes_as_null() {
    let result = make_result_opt(
        vec!["id", "created_at"],
        vec![vec![Some("1"), None]],
    );
    let json = to_json(&result).unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed[0]["id"], serde_json::json!(1));
    assert_eq!(parsed[0]["created_at"], serde_json::Value::Null);
}

/// NULL 値は CSV では空フィールドとして出力される
#[test]
fn to_csv_null_value_outputs_empty_field() {
    let result = make_result_opt(
        vec!["id", "name"],
        vec![vec![Some("1"), None]],
    );
    let csv = to_csv(&result).unwrap();
    assert_eq!(csv, "id,name\n1,\n");
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
    assert_eq!(parse_json_value("1.25"), serde_json::json!(1.25));
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

// ── to_table ──

/// テーブル出力はヘッダー行・区切り行・データ行を含み、列を ` │ ` で区切る
#[test]
fn to_table_outputs_header_separator_and_rows() {
    let result = make_result(vec!["id", "name"], vec![vec!["1", "Alice"], vec!["2", "Bob"]]);
    let table = to_table(&result);
    let lines: Vec<&str> = table.lines().collect();
    assert_eq!(lines.len(), 4, "header + sep + 2 rows: {:?}", lines);
    assert_eq!(lines[0], " id │ name ");
    assert_eq!(lines[1], "────┼───────");
    assert_eq!(lines[2], " 1  │ Alice");
    assert_eq!(lines[3], " 2  │ Bob  ");
}

/// 列幅は値側がヘッダーより広い場合、値の幅に合わせて広がる
#[test]
fn to_table_widens_columns_to_fit_values() {
    let result = make_result(vec!["a"], vec![vec!["longer_value"]]);
    let table = to_table(&result);
    let lines: Vec<&str> = table.lines().collect();
    assert_eq!(lines[0], " a           ");
    assert_eq!(lines[1], "──────────────");
    assert_eq!(lines[2], " longer_value");
}

/// NULL は `NULL` リテラルとして出力される（CSV の空フィールドとは異なる挙動）
#[test]
fn to_table_null_value_renders_as_null_literal() {
    let result = make_result_opt(vec!["id", "name"], vec![vec![Some("1"), None]]);
    let table = to_table(&result);
    let lines: Vec<&str> = table.lines().collect();
    assert_eq!(lines[2], " 1  │ NULL");
}

/// 空の結果（列が無い）では空文字列を返す
#[test]
fn to_table_no_columns_returns_empty_string() {
    let result = make_result(vec![], vec![]);
    let table = to_table(&result);
    assert_eq!(table, "");
}

/// 列はあるが行が無い場合、ヘッダーと区切り線のみが出力される
#[test]
fn to_table_no_rows_outputs_header_and_separator_only() {
    let result = make_result(vec!["id", "name"], vec![]);
    let table = to_table(&result);
    let lines: Vec<&str> = table.lines().collect();
    assert_eq!(lines.len(), 2);
    // 行が無いため `name` 列の幅はヘッダー文字列幅（4）に等しく、末尾パディングが付かない
    assert_eq!(lines[0], " id │ name");
    assert_eq!(lines[1], "────┼──────");
}

// ── ExportFormat ──

#[test]
fn export_format_clipboard_label_and_extension() {
    assert_eq!(ExportFormat::Clipboard.label(), "Clipboard");
    assert_eq!(ExportFormat::Clipboard.extension(), "txt");
}
