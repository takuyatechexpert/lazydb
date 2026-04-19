use super::*;
use tokio::sync::mpsc;

// ── ヘルパー ──

fn test_app() -> App {
    let (tx, _rx) = mpsc::channel(10);
    App::new(vec![], AppConfig::default(), tx)
}

// ── Tab::new() ──

#[test]
fn tab_new_has_correct_id() {
    let tab = Tab::new(42);
    assert_eq!(tab.id, 42);
}

#[test]
fn tab_new_has_default_name() {
    let tab = Tab::new(1);
    assert_eq!(tab.name, "Query");
}

// ── add_tab() ──

#[test]
fn add_tab_increases_tab_count() {
    // Arrange
    let mut app = test_app();
    let initial_count = app.tabs.len();

    // Act
    app.add_tab();

    // Assert
    assert_eq!(app.tabs.len(), initial_count + 1);
}

#[test]
fn add_tab_activates_new_tab() {
    // Arrange
    let mut app = test_app();

    // Act
    app.add_tab();

    // Assert — 新しいタブがアクティブになっている
    let active_tab = &app.tabs[app.active_tab];
    assert_eq!(active_tab.name, "Query");
    // 初期タブ(id=1)の次なので id=2
    assert_eq!(active_tab.id, 2);
}

#[test]
fn add_tab_inserts_after_active_tab() {
    // Arrange
    let mut app = test_app();
    // 初期状態: [Tab(id=1)]、active_tab=0
    app.add_tab(); // [Tab(id=1), Tab(id=2)]、active_tab=1
    app.add_tab(); // [Tab(id=1), Tab(id=2), Tab(id=3)]、active_tab=2

    // active_tab を先頭に戻す
    app.active_tab = 0;

    // Act — 先頭タブがアクティブな状態で追加
    app.add_tab();

    // Assert — アクティブタブ(0)の直後(1)に挿入される
    assert_eq!(app.active_tab, 1);
    assert_eq!(app.tabs[1].id, 4);
}

#[test]
fn add_tab_at_max_sets_status_message() {
    // Arrange
    let mut app = test_app();
    // 初期1タブ + 9タブ追加 = 10タブ（上限）
    for _ in 0..9 {
        app.add_tab();
    }
    assert_eq!(app.tabs.len(), MAX_TABS);

    // Act — 上限で追加を試みる
    app.add_tab();

    // Assert — タブは増えず、ステータスメッセージが設定される
    assert_eq!(app.tabs.len(), MAX_TABS);
    assert!(app.status_message.is_some());
}

// ── close_tab() ──

#[test]
fn close_tab_removes_active_tab() {
    // Arrange
    let mut app = test_app();
    app.add_tab(); // 2タブになる
    assert_eq!(app.tabs.len(), 2);

    // Act
    app.close_tab();

    // Assert
    assert_eq!(app.tabs.len(), 1);
}

#[test]
fn close_tab_activates_right_neighbor() {
    // Arrange
    let mut app = test_app();
    app.add_tab();
    app.add_tab();
    // 3タブ: [Tab0, Tab1, Tab2]
    // active_tab=2（最後の add_tab で最新がアクティブ）

    // 中間のタブをアクティブにする
    app.active_tab = 1;
    let right_tab_id = app.tabs[2].id;

    // Act
    app.close_tab();

    // Assert — 右隣がアクティブになる
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.tabs[app.active_tab].id, right_tab_id);
}

#[test]
fn close_tab_at_right_end_activates_left_neighbor() {
    // Arrange
    let mut app = test_app();
    app.add_tab();
    // 2タブ: [Tab0, Tab1]、active_tab=1（右端）
    assert_eq!(app.active_tab, 1);
    let left_tab_id = app.tabs[0].id;

    // Act
    app.close_tab();

    // Assert — 左隣がアクティブになる
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[app.active_tab].id, left_tab_id);
}

#[test]
fn close_tab_does_nothing_when_only_one_tab() {
    // Arrange
    let mut app = test_app();
    assert_eq!(app.tabs.len(), 1);

    // Act
    app.close_tab();

    // Assert — タブは閉じられない
    assert_eq!(app.tabs.len(), 1);
}

// ── next_tab() ──

#[test]
fn next_tab_moves_to_next() {
    // Arrange
    let mut app = test_app();
    app.add_tab();
    app.add_tab();
    // 3タブ、active_tab を先頭に
    app.active_tab = 0;

    // Act
    app.next_tab();

    // Assert
    assert_eq!(app.active_tab, 1);
}

#[test]
fn next_tab_wraps_around_from_last_to_first() {
    // Arrange
    let mut app = test_app();
    app.add_tab();
    // 2タブ、active_tab=1（末尾）
    assert_eq!(app.active_tab, 1);

    // Act
    app.next_tab();

    // Assert — ラップアラウンドで先頭に戻る
    assert_eq!(app.active_tab, 0);
}

// ── prev_tab() ──

#[test]
fn prev_tab_moves_to_previous() {
    // Arrange
    let mut app = test_app();
    app.add_tab();
    // 2タブ、active_tab=1
    assert_eq!(app.active_tab, 1);

    // Act
    app.prev_tab();

    // Assert
    assert_eq!(app.active_tab, 0);
}

#[test]
fn prev_tab_wraps_around_from_first_to_last() {
    // Arrange
    let mut app = test_app();
    app.add_tab();
    // 2タブ、active_tab を先頭に
    app.active_tab = 0;

    // Act
    app.prev_tab();

    // Assert — ラップアラウンドで末尾に移動
    assert_eq!(app.active_tab, 1);
}
