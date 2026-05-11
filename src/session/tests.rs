use super::*;

fn tempfile_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lazydb-session-test-{}-{}",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("session.json")
}

fn make_tab(content: &str) -> TabSnapshot {
    TabSnapshot {
        name: "Query".to_string(),
        content: content.to_string(),
        cursor_row: 0,
        cursor_col: 0,
    }
}

#[test]
fn load_returns_empty_state_when_file_missing() {
    let path = tempfile_path("missing");
    let store = SessionStore::with_path(path);
    let state = store.load();
    assert!(state.connections.is_empty());
}

#[test]
fn save_then_load_round_trip_preserves_per_connection_tabs() {
    let path = tempfile_path("round_trip");
    let store = SessionStore::with_path(path);

    let mut state = SessionState::default();
    state.set(
        "prod-db".to_string(),
        ConnectionSession {
            tabs: vec![make_tab("SELECT 1;"), make_tab("SELECT 2;")],
            active_tab: 1,
        },
    );
    state.set(
        "local".to_string(),
        ConnectionSession {
            tabs: vec![make_tab("SHOW TABLES;")],
            active_tab: 0,
        },
    );

    store.save(&state).unwrap();
    let loaded = store.load();

    let prod = loaded.get("prod-db").expect("prod-db session exists");
    assert_eq!(prod.tabs.len(), 2);
    assert_eq!(prod.tabs[1].content, "SELECT 2;");
    assert_eq!(prod.active_tab, 1);

    let local = loaded.get("local").expect("local session exists");
    assert_eq!(local.tabs.len(), 1);
    assert_eq!(local.tabs[0].content, "SHOW TABLES;");
}

#[test]
fn load_returns_empty_state_when_file_is_corrupt() {
    let path = tempfile_path("corrupt");
    std::fs::write(&path, "not valid json {{{").unwrap();
    let store = SessionStore::with_path(path);
    let state = store.load();
    assert!(state.connections.is_empty());
}

#[test]
fn load_accepts_missing_optional_fields() {
    let path = tempfile_path("partial");
    std::fs::write(
        &path,
        r#"{"connections":{"db1":{"tabs":[{"name":"Query","content":"SELECT 1"}]}}}"#,
    )
    .unwrap();
    let store = SessionStore::with_path(path);
    let loaded = store.load();
    let db1 = loaded.get("db1").expect("db1 session exists");
    assert_eq!(db1.tabs.len(), 1);
    assert_eq!(db1.tabs[0].cursor_row, 0);
    assert_eq!(db1.active_tab, 0);
}

#[test]
fn set_overwrites_existing_connection_entry() {
    let mut state = SessionState::default();
    state.set(
        "db".to_string(),
        ConnectionSession {
            tabs: vec![make_tab("old")],
            active_tab: 0,
        },
    );
    state.set(
        "db".to_string(),
        ConnectionSession {
            tabs: vec![make_tab("new1"), make_tab("new2")],
            active_tab: 1,
        },
    );
    let s = state.get("db").unwrap();
    assert_eq!(s.tabs.len(), 2);
    assert_eq!(s.tabs[0].content, "new1");
    assert_eq!(s.active_tab, 1);
}
