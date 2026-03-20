use super::*;
use std::path::PathBuf;

fn temp_store() -> HistoryStore {
    let dir = std::env::temp_dir().join(format!("lazydb_test_{}", uuid::Uuid::new_v4()));
    let path = dir.join("history.ndjson");
    HistoryStore { path }
}

#[test]
fn append_and_load_all() {
    let store = temp_store();
    store.append("SELECT 1", "local", 1, 10).unwrap();

    let entries = store.load_all().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].query, "SELECT 1");
    assert_eq!(entries[0].connection, "local");
    assert_eq!(entries[0].rows, 1);
    assert_eq!(entries[0].duration_ms, 10);
}

#[test]
fn append_multiple() {
    let store = temp_store();
    store.append("SELECT 1", "local", 1, 10).unwrap();
    store.append("SELECT 2", "local", 2, 20).unwrap();
    store.append("SELECT 3", "staging", 3, 30).unwrap();

    let entries = store.load_all().unwrap();
    assert_eq!(entries.len(), 3);
}

#[test]
fn search_filters_by_query() {
    let store = temp_store();
    store.append("SELECT * FROM users", "local", 10, 50).unwrap();
    store.append("SELECT * FROM orders", "local", 5, 30).unwrap();
    store.append("INSERT INTO logs", "local", 0, 10).unwrap();

    let results = store.search("users").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].query, "SELECT * FROM users");
}

#[test]
fn search_returns_newest_first() {
    let store = temp_store();
    store.append("SELECT 1", "local", 1, 10).unwrap();
    store.append("SELECT 2", "local", 2, 20).unwrap();

    let results = store.search("SELECT").unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].query, "SELECT 2");
    assert_eq!(results[1].query, "SELECT 1");
}

#[test]
fn load_all_returns_empty_when_file_not_exists() {
    let store = HistoryStore {
        path: PathBuf::from("/tmp/lazydb_nonexistent_dir/history.ndjson"),
    };
    let entries = store.load_all().unwrap();
    assert!(entries.is_empty());
}

#[test]
fn search_with_empty_filter_returns_all() {
    let store = temp_store();
    store.append("SELECT 1", "local", 1, 10).unwrap();
    store.append("INSERT INTO t", "local", 0, 5).unwrap();

    let results = store.search("").unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn max_entries_truncation() {
    let store = temp_store();
    // 1002件追加
    for i in 0..1002 {
        store
            .append(&format!("SELECT {}", i), "local", 1, 10)
            .unwrap();
    }

    let entries = store.load_all().unwrap();
    assert_eq!(entries.len(), 1000);
    // 最古の 0, 1 が消えている
    assert_eq!(entries[0].query, "SELECT 2");
    assert_eq!(entries[999].query, "SELECT 1001");
}
