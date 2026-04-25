use super::*;
use crate::tui::editor::EditorMode;
use crate::tui::scrollable::{dispatch_scroll_key, Scrollable};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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

// ── 統合テスト: dispatch_scroll_key を経由した各ペインの状態遷移 ──

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl_key(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
}

// ── EditorState 経由の統合 ──

fn editor_with_lines(n: usize) -> editor::EditorState {
    let mut e = editor::EditorState::new();
    let text: Vec<String> = (0..n).map(|i| format!("line{}", i)).collect();
    e.set_content(&text.join("\n"));
    e
}

#[test]
fn dispatch_editor_j_advances_cursor_row() {
    let mut e = editor_with_lines(10);
    e.cursor = (0, 0);
    let handled = dispatch_scroll_key(&mut e, &key(KeyCode::Char('j')), 20);
    assert!(handled);
    assert_eq!(e.cursor, (1, 0));
}

#[test]
fn dispatch_editor_g_jumps_to_top() {
    let mut e = editor_with_lines(10);
    e.cursor = (5, 2);
    let handled = dispatch_scroll_key(&mut e, &key(KeyCode::Char('g')), 20);
    assert!(handled);
    assert_eq!(e.cursor, (0, 0));
}

#[test]
fn dispatch_editor_capital_g_jumps_to_bottom() {
    let mut e = editor_with_lines(10);
    e.cursor = (0, 0);
    let handled = dispatch_scroll_key(&mut e, &key(KeyCode::Char('G')), 20);
    assert!(handled);
    assert_eq!(e.cursor, (9, 0));
}

#[test]
fn dispatch_editor_ctrl_d_pages_down() {
    let mut e = editor_with_lines(50);
    e.cursor = (0, 0);
    let handled = dispatch_scroll_key(&mut e, &ctrl_key('d'), 20);
    assert!(handled);
    assert_eq!(e.cursor.0, 20);
}

#[test]
fn dispatch_editor_ctrl_u_pages_up() {
    let mut e = editor_with_lines(50);
    e.cursor = (30, 0);
    let handled = dispatch_scroll_key(&mut e, &ctrl_key('u'), 20);
    assert!(handled);
    assert_eq!(e.cursor.0, 10);
}

#[test]
fn dispatch_editor_capital_h_moves_40_left() {
    let line: String = "x".repeat(100);
    let mut e = editor::EditorState::new();
    e.set_content(&line);
    e.cursor = (0, 80);
    let handled = dispatch_scroll_key(&mut e, &key(KeyCode::Char('H')), 20);
    assert!(handled);
    assert_eq!(e.cursor.1, 40);
}

#[test]
fn dispatch_editor_capital_l_moves_40_right() {
    let line: String = "x".repeat(100);
    let mut e = editor::EditorState::new();
    e.set_content(&line);
    e.cursor = (0, 10);
    let handled = dispatch_scroll_key(&mut e, &key(KeyCode::Char('L')), 20);
    assert!(handled);
    assert_eq!(e.cursor.1, 50);
}

#[test]
fn dispatch_editor_plain_d_returns_false_for_delete_line_fallthrough() {
    // ctrl=false の 'd' は dispatch を素通りし、呼び出し元で delete_line に到達する設計
    let mut e = editor_with_lines(5);
    e.cursor = (2, 0);
    let handled = dispatch_scroll_key(&mut e, &key(KeyCode::Char('d')), 20);
    assert!(!handled);
    // dispatch 自体は何も変化させない
    assert_eq!(e.cursor, (2, 0));
}

#[test]
fn dispatch_editor_plain_u_returns_false_for_undo_fallthrough() {
    let mut e = editor_with_lines(5);
    e.cursor = (2, 0);
    let handled = dispatch_scroll_key(&mut e, &key(KeyCode::Char('u')), 20);
    assert!(!handled);
    assert_eq!(e.cursor, (2, 0));
}

// ── ResultsState 経由の統合 ──

fn make_results(rows: usize) -> results::ResultsState {
    let mut r = results::ResultsState::new();
    r.columns = vec!["a".to_string(), "b".to_string()];
    r.rows = (0..rows)
        .map(|i| vec![format!("r{}_a", i), format!("r{}_b", i)])
        .collect();
    r.col_widths = vec![10, 10];
    r.status = results::ResultStatus::Success;
    r.total_rows = rows;
    r.visible_width = 10;
    r
}

#[test]
fn dispatch_results_j_advances_scroll_offset() {
    let mut r = make_results(10);
    r.scroll_offset = 0;
    let handled = dispatch_scroll_key(&mut r, &key(KeyCode::Char('j')), 20);
    assert!(handled);
    assert_eq!(r.scroll_offset, 1);
}

#[test]
fn dispatch_results_capital_g_jumps_to_last_row() {
    let mut r = make_results(10);
    r.scroll_offset = 0;
    let handled = dispatch_scroll_key(&mut r, &key(KeyCode::Char('G')), 20);
    assert!(handled);
    assert_eq!(r.scroll_offset, 9);
}

#[test]
fn dispatch_results_ctrl_d_pages_down() {
    let mut r = make_results(50);
    r.scroll_offset = 0;
    let handled = dispatch_scroll_key(&mut r, &ctrl_key('d'), 20);
    assert!(handled);
    assert_eq!(r.scroll_offset, 20);
}

#[test]
fn dispatch_results_h_retreats_h_scroll_by_4() {
    let mut r = make_results(10);
    r.h_scroll = 10;
    let handled = dispatch_scroll_key(&mut r, &key(KeyCode::Char('h')), 20);
    assert!(handled);
    assert_eq!(r.h_scroll, 6);
}

#[test]
fn dispatch_results_l_advances_h_scroll_by_4() {
    let mut r = make_results(10);
    r.col_widths = vec![50, 50];
    r.visible_width = 10;
    r.h_scroll = 0;
    let handled = dispatch_scroll_key(&mut r, &key(KeyCode::Char('l')), 20);
    assert!(handled);
    assert_eq!(r.h_scroll, 4);
}

#[test]
fn dispatch_results_y_returns_false() {
    // 'y' は dispatch では捌かれず、呼び出し元の copy_current_row へ fallthrough する
    let mut r = make_results(10);
    let handled = dispatch_scroll_key(&mut r, &key(KeyCode::Char('y')), 20);
    assert!(!handled);
}

#[test]
fn dispatch_results_c_returns_false() {
    let mut r = make_results(10);
    let handled = dispatch_scroll_key(&mut r, &key(KeyCode::Char('c')), 20);
    assert!(!handled);
}

// ── SchemaState 経由の統合 ──

fn make_schema(n: usize) -> schema::SchemaState {
    let mut s = schema::SchemaState::new();
    for i in 0..n {
        s.tables.push(schema::TableEntry {
            name: format!("t{}", i),
            expanded: false,
            columns_loaded: false,
            columns_loading: false,
            columns: Vec::new(),
        });
    }
    s
}

#[test]
fn dispatch_schema_j_advances_cursor() {
    let mut s = make_schema(5);
    s.cursor = 1;
    let handled = dispatch_scroll_key(&mut s, &key(KeyCode::Char('j')), 20);
    assert!(handled);
    assert_eq!(s.cursor, 2);
}

#[test]
fn dispatch_schema_h_is_handled_but_noop() {
    // 'h' は dispatch される（handled=true）が、Schema は横スクロールが no-op
    let mut s = make_schema(5);
    s.cursor = 2;
    let handled = dispatch_scroll_key(&mut s, &key(KeyCode::Char('h')), 20);
    assert!(handled);
    assert_eq!(s.cursor, 2); // 変化なし
}

#[test]
fn dispatch_schema_capital_g_jumps_to_last_item() {
    let mut s = make_schema(8);
    s.cursor = 0;
    let handled = dispatch_scroll_key(&mut s, &key(KeyCode::Char('G')), 20);
    assert!(handled);
    assert_eq!(s.cursor, 7);
}

#[test]
fn dispatch_schema_ctrl_d_pages_down() {
    let mut s = make_schema(50);
    s.cursor = 0;
    let handled = dispatch_scroll_key(&mut s, &ctrl_key('d'), 20);
    assert!(handled);
    assert_eq!(s.cursor, 20);
}

// ── Editor Insert モード保護 ──
//
// handle_editor_insert_key は dispatch_scroll_key を呼ばないため、
// Char('H') / Char('G') などは insert_char に流れて文字入力になるべき。
// ここでは EditorState 単体で「Insert モードで insert_char を叩いた場合の挙動」を確認する。

#[test]
fn editor_insert_mode_capital_g_inserts_literal_g() {
    let mut e = editor::EditorState::new();
    e.mode = EditorMode::Insert;
    e.insert_char('G');
    assert_eq!(e.lines, vec!["G".to_string()]);
    assert_eq!(e.cursor, (0, 1));
}

#[test]
fn editor_insert_mode_capital_h_inserts_literal_h() {
    let mut e = editor::EditorState::new();
    e.mode = EditorMode::Insert;
    e.insert_char('H');
    assert_eq!(e.lines, vec!["H".to_string()]);
    assert_eq!(e.cursor, (0, 1));
}

// ── ペイン共通: 同じ KeyCode で「同じ概念の操作」が起きる ──

#[test]
fn capital_g_jumps_to_bottom_in_all_three_panes() {
    // Editor: 末尾行へ
    let mut e = editor_with_lines(10);
    e.cursor = (0, 0);
    assert!(dispatch_scroll_key(&mut e, &key(KeyCode::Char('G')), 20));
    assert_eq!(e.cursor.0, 9);

    // Results: 末尾行へ
    let mut r = make_results(10);
    r.scroll_offset = 0;
    assert!(dispatch_scroll_key(&mut r, &key(KeyCode::Char('G')), 20));
    assert_eq!(r.scroll_offset, 9);

    // Schema: 末尾アイテムへ
    let mut s = make_schema(10);
    s.cursor = 0;
    assert!(dispatch_scroll_key(&mut s, &key(KeyCode::Char('G')), 20));
    assert_eq!(s.cursor, 9);
}

#[test]
fn page_down_advances_by_20_in_all_three_panes() {
    let mut e = editor_with_lines(50);
    e.cursor = (0, 0);
    assert!(dispatch_scroll_key(&mut e, &key(KeyCode::PageDown), 20));
    assert_eq!(e.cursor.0, 20);

    let mut r = make_results(50);
    r.scroll_offset = 0;
    assert!(dispatch_scroll_key(&mut r, &key(KeyCode::PageDown), 20));
    assert_eq!(r.scroll_offset, 20);

    let mut s = make_schema(50);
    s.cursor = 0;
    assert!(dispatch_scroll_key(&mut s, &key(KeyCode::PageDown), 20));
    assert_eq!(s.cursor, 20);
}

// ── trait オブジェクト経由 (?Sized 制約の確認) ──

#[test]
fn dispatch_scroll_key_works_via_trait_object() {
    let mut e = editor_with_lines(10);
    e.cursor = (5, 0);
    {
        let dyn_ref: &mut dyn Scrollable = &mut e;
        let handled = dispatch_scroll_key(dyn_ref, &key(KeyCode::Char('g')), 20);
        assert!(handled);
    }
    assert_eq!(e.cursor, (0, 0));
}
