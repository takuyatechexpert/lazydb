use super::*;

fn editor_with(text: &str) -> EditorState {
    let mut e = EditorState::new();
    e.set_content(text);
    e
}

// ── insert_char ──

#[test]
fn insert_char_at_cursor() {
    let mut e = EditorState::new();
    e.mode = EditorMode::Insert;
    e.insert_char('A');
    e.insert_char('B');
    assert_eq!(e.lines, vec!["AB"]);
    assert_eq!(e.cursor, (0, 2));
}

// ── insert_newline ──

#[test]
fn insert_newline_splits_line() {
    let mut e = editor_with("ABCD");
    e.cursor = (0, 2);
    e.insert_newline();
    assert_eq!(e.lines, vec!["AB", "CD"]);
    assert_eq!(e.cursor, (1, 0));
}

// ── backspace ──

#[test]
fn backspace_deletes_char() {
    let mut e = editor_with("ABC");
    e.cursor = (0, 3);
    e.backspace();
    assert_eq!(e.lines, vec!["AB"]);
    assert_eq!(e.cursor, (0, 2));
}

#[test]
fn backspace_joins_lines() {
    let mut e = editor_with("AB\nCD");
    e.cursor = (1, 0);
    e.backspace();
    assert_eq!(e.lines, vec!["ABCD"]);
    assert_eq!(e.cursor, (0, 2));
}

#[test]
fn backspace_at_start_does_nothing() {
    let mut e = editor_with("ABC");
    e.cursor = (0, 0);
    e.backspace();
    assert_eq!(e.lines, vec!["ABC"]);
    assert_eq!(e.cursor, (0, 0));
}

// ── undo / redo ──

#[test]
fn undo_redo_cycle() {
    let mut e = EditorState::new();
    e.insert_char('A');
    assert_eq!(e.lines, vec!["A"]);

    e.undo();
    assert_eq!(e.lines, vec![""]);

    e.redo();
    assert_eq!(e.lines, vec!["A"]);
}

// ── append_text ──

#[test]
fn append_text_to_empty_editor() {
    let mut e = EditorState::new();
    e.append_text("SELECT 1");
    assert_eq!(e.lines, vec!["SELECT 1"]);
    assert_eq!(e.cursor, (0, 8));
}

#[test]
fn append_text_when_last_line_empty_appends_without_newline() {
    let mut e = editor_with("SELECT 1;\n");
    e.append_text("UPDATE t SET a=1;");
    assert_eq!(e.lines, vec!["SELECT 1;", "UPDATE t SET a=1;"]);
    assert_eq!(e.cursor, (1, 17));
}

#[test]
fn append_text_when_last_line_nonempty_inserts_newline() {
    let mut e = editor_with("SELECT 1;");
    e.append_text("UPDATE t SET a=1;");
    assert_eq!(e.lines, vec!["SELECT 1;", "UPDATE t SET a=1;"]);
    assert_eq!(e.cursor, (1, 17));
}

#[test]
fn append_text_with_leading_newline_does_not_insert_extra_newline() {
    let mut e = editor_with("SELECT 1;");
    e.append_text("\nUPDATE t SET a=1;");
    assert_eq!(e.lines, vec!["SELECT 1;", "UPDATE t SET a=1;"]);
    assert_eq!(e.cursor, (1, 17));
}

#[test]
fn append_text_with_embedded_newlines_creates_multiple_lines() {
    let mut e = editor_with("A");
    e.append_text("B\nC\nD");
    assert_eq!(e.lines, vec!["A", "B", "C", "D"]);
    assert_eq!(e.cursor, (3, 1));
}

#[test]
fn append_text_undo_restores_previous_state() {
    let mut e = editor_with("SELECT 1;");
    e.cursor = (0, 3);
    e.append_text("UPDATE t SET a=1;");
    assert_eq!(e.lines, vec!["SELECT 1;", "UPDATE t SET a=1;"]);
    e.undo();
    assert_eq!(e.lines, vec!["SELECT 1;"]);
    assert_eq!(e.cursor, (0, 3));
}

// ── move_word_forward (w) ──

#[test]
fn move_word_forward_jumps_to_next_word() {
    let mut e = editor_with("SELECT * FROM users");
    e.cursor = (0, 0);
    e.move_word_forward();
    assert_eq!(e.cursor.1, 7); // "*" の位置
}

// ── move_word_back (b) ──

#[test]
fn move_word_back_jumps_to_prev_word() {
    let mut e = editor_with("SELECT * FROM users");
    e.cursor = (0, 9); // "F" の位置
    e.move_word_back();
    // "SELECT_*_FROM" で * は記号なので独立した単語、b で " " を超えて "SELECT" の先頭 (6) に
    // 実装: 空白スキップ→同種文字スキップ = "*" (pos 7) の手前の空白を越えて pos 6
    assert_eq!(e.cursor.1, 6);
}

// ── delete_line (dd) ──

#[test]
fn delete_line_removes_current_line() {
    let mut e = editor_with("line1\nline2\nline3");
    e.cursor = (1, 0);
    e.delete_line();
    assert_eq!(e.lines, vec!["line1", "line3"]);
}

#[test]
fn delete_line_on_single_line_clears_it() {
    let mut e = editor_with("hello");
    e.cursor = (0, 0);
    e.delete_line();
    assert_eq!(e.lines, vec![""]);
}

// ── mode transitions ──

#[test]
fn enter_insert_changes_mode() {
    let mut e = EditorState::new();
    assert_eq!(e.mode, EditorMode::Normal);
    e.enter_insert();
    assert_eq!(e.mode, EditorMode::Insert);
}

#[test]
fn enter_normal_changes_mode() {
    let mut e = EditorState::new();
    e.enter_insert();
    e.insert_char('A');
    e.enter_normal();
    assert_eq!(e.mode, EditorMode::Normal);
}

#[test]
fn enter_insert_after_moves_cursor_right() {
    let mut e = editor_with("ABC");
    e.cursor = (0, 1);
    e.enter_insert_after();
    assert_eq!(e.cursor, (0, 2));
    assert_eq!(e.mode, EditorMode::Insert);
}

#[test]
fn enter_insert_end_moves_to_eol() {
    let mut e = editor_with("ABC");
    e.cursor = (0, 0);
    e.enter_insert_end();
    assert_eq!(e.cursor, (0, 3));
    assert_eq!(e.mode, EditorMode::Insert);
}

#[test]
fn enter_insert_below_adds_line() {
    let mut e = editor_with("line1");
    e.cursor = (0, 0);
    e.enter_insert_below();
    assert_eq!(e.lines, vec!["line1", ""]);
    assert_eq!(e.cursor, (1, 0));
    assert_eq!(e.mode, EditorMode::Insert);
}

#[test]
fn enter_insert_above_adds_line() {
    let mut e = editor_with("line1");
    e.cursor = (0, 0);
    e.enter_insert_above();
    assert_eq!(e.lines, vec!["", "line1"]);
    assert_eq!(e.cursor, (0, 0));
    assert_eq!(e.mode, EditorMode::Insert);
}

// ── set_content ──

#[test]
fn set_content_replaces_editor() {
    let mut e = EditorState::new();
    e.set_content("SELECT *\nFROM users;");
    assert_eq!(e.lines, vec!["SELECT *", "FROM users;"]);
    assert_eq!(e.cursor, (1, 11));
}

// ── get_query_at_cursor ──

#[test]
fn get_query_at_cursor_single_query() {
    let mut e = editor_with("SELECT 1");
    e.cursor = (0, 4);
    assert_eq!(e.get_query_at_cursor(), Some("SELECT 1".to_string()));
}

#[test]
fn get_query_at_cursor_with_semicolons() {
    let mut e = editor_with("SELECT 1; SELECT 2;");
    e.cursor = (0, 12); // "SELECT 2" の中
    assert_eq!(e.get_query_at_cursor(), Some("SELECT 2".to_string()));
}

#[test]
fn get_query_at_cursor_empty_editor() {
    let e = EditorState::new();
    assert_eq!(e.get_query_at_cursor(), None);
}

// ── delete_to_end (D) ──

#[test]
fn delete_to_end_truncates_line() {
    let mut e = editor_with("SELECT * FROM users");
    e.cursor = (0, 9);
    e.delete_to_end();
    assert_eq!(e.lines, vec!["SELECT * "]);
}

// ── change_to_end (C) ──

#[test]
fn change_to_end_truncates_and_enters_insert() {
    let mut e = editor_with("SELECT * FROM users");
    e.cursor = (0, 9);
    e.change_to_end();
    assert_eq!(e.lines, vec!["SELECT * "]);
    assert_eq!(e.mode, EditorMode::Insert);
}

// ── move_to_top / move_to_bottom ──

#[test]
fn move_to_top_and_bottom() {
    let mut e = editor_with("line1\nline2\nline3");
    e.cursor = (1, 3);
    e.move_to_top();
    assert_eq!(e.cursor, (0, 0));
    e.move_to_bottom();
    assert_eq!(e.cursor, (2, 0));
}

// ── move_first_non_blank (^) ──

#[test]
fn move_first_non_blank() {
    let mut e = editor_with("  SELECT *");
    e.cursor = (0, 8);
    e.move_first_non_blank();
    assert_eq!(e.cursor, (0, 2));
}

// ── format_buffer ──

#[test]
fn format_buffer_uppercases_and_indents() {
    let mut e = editor_with("select id,name from users where id=1");
    let changed = e.format_buffer();
    assert!(changed);
    // SELECT 等のキーワードが大文字化されていること
    assert!(e.lines.iter().any(|l| l.contains("SELECT")));
    assert!(e.lines.iter().any(|l| l.contains("FROM")));
    assert!(e.lines.iter().any(|l| l.contains("WHERE")));
}

#[test]
fn format_buffer_empty_returns_false() {
    let mut e = EditorState::new();
    assert!(!e.format_buffer());
}

#[test]
fn format_buffer_undoable() {
    let original = "select 1";
    let mut e = editor_with(original);
    assert!(e.format_buffer());
    // 結果が変化していること
    assert_ne!(e.lines.join("\n"), original);
    e.undo();
    assert_eq!(e.lines, vec![original.to_string()]);
}

#[test]
fn format_buffer_inserts_blank_line_between_queries() {
    let mut e = editor_with("select 1; select 2;");
    assert!(e.format_buffer());
    // 1つ目のクエリ末尾の `;` の次行が空行になっていること
    let joined = e.lines.join("\n");
    // SELECT が2回登場する
    assert_eq!(joined.matches("SELECT").count(), 2);
    // `;\n\nSELECT` のパターンを含む（クエリ間に空行）
    assert!(
        joined.contains(";\n\nSELECT"),
        "expected blank line between queries, got:\n{}",
        joined
    );
    // 末尾の `;` 後ろには余計な空行が無い
    assert!(joined.trim_end().ends_with(';'));
}

#[test]
fn insert_blank_lines_no_change_when_already_separated() {
    let input = "SELECT 1;\n\nSELECT 2;";
    let out = insert_blank_lines_between_queries(input);
    assert_eq!(out, input);
}

#[test]
fn insert_blank_lines_no_trailing_blank_after_last_semicolon() {
    let input = "SELECT 1;";
    let out = insert_blank_lines_between_queries(input);
    assert_eq!(out, "SELECT 1;");
}

#[test]
fn insert_blank_lines_three_queries() {
    let input = "SELECT 1;\nSELECT 2;\nSELECT 3;";
    let out = insert_blank_lines_between_queries(input);
    assert_eq!(out, "SELECT 1;\n\nSELECT 2;\n\nSELECT 3;");
}

// ── adjust_scroll (horizontal) ──

#[test]
fn adjust_scroll_horizontal_when_cursor_exceeds_width() {
    let mut e = editor_with("ABCDEFGHIJ");
    e.cursor = (0, 10);
    e.adjust_scroll(10, 5); // visible_width = 5
    // cursor.1 (10) >= h_scroll_offset (0) + 5 → h_scroll_offset = 10 + 1 - 5 = 6
    assert_eq!(e.h_scroll_offset, 6);
}

#[test]
fn adjust_scroll_horizontal_when_cursor_before_offset() {
    let mut e = editor_with("ABCDEFGHIJ");
    e.h_scroll_offset = 5;
    e.cursor = (0, 2);
    e.adjust_scroll(10, 5);
    // cursor.1 (2) < h_scroll_offset (5) → h_scroll_offset = 2
    assert_eq!(e.h_scroll_offset, 2);
}

#[test]
fn adjust_scroll_horizontal_no_change_when_within_view() {
    let mut e = editor_with("ABCDEFGHIJ");
    e.cursor = (0, 3);
    e.adjust_scroll(10, 10);
    assert_eq!(e.h_scroll_offset, 0);
}

// ── move_page_down ──

fn editor_with_lines(n: usize) -> EditorState {
    let text: Vec<String> = (0..n).map(|i| format!("line{}", i)).collect();
    editor_with(&text.join("\n"))
}

#[test]
fn move_page_down_advances_cursor_by_page_size() {
    let mut e = editor_with_lines(50);
    e.cursor = (0, 0);
    e.move_page_down(20);
    assert_eq!(e.cursor.0, 20);
    assert_eq!(e.cursor.1, 0);
}

#[test]
fn move_page_down_clamps_at_last_line() {
    let mut e = editor_with_lines(10);
    e.cursor = (5, 0);
    e.move_page_down(20);
    assert_eq!(e.cursor.0, 9);
}

#[test]
fn move_page_down_clamps_column_to_line_width() {
    // 移動先の行幅が短い場合、列がクランプされる
    let mut e = editor_with("aaaaaa\nbb");
    e.cursor = (0, 5);
    e.move_page_down(1);
    assert_eq!(e.cursor.0, 1);
    assert_eq!(e.cursor.1, 2); // "bb" は 2 文字
}

#[test]
fn move_page_down_at_last_line_keeps_position() {
    let mut e = editor_with_lines(5);
    e.cursor = (4, 0);
    e.move_page_down(20);
    assert_eq!(e.cursor.0, 4);
}

#[test]
fn move_page_down_with_page_size_zero_no_move() {
    let mut e = editor_with_lines(10);
    e.cursor = (3, 0);
    e.move_page_down(0);
    assert_eq!(e.cursor.0, 3);
}

// ── move_page_up ──

#[test]
fn move_page_up_retreats_cursor_by_page_size() {
    let mut e = editor_with_lines(50);
    e.cursor = (30, 0);
    e.move_page_up(20);
    assert_eq!(e.cursor.0, 10);
    assert_eq!(e.cursor.1, 0);
}

#[test]
fn move_page_up_clamps_at_top() {
    let mut e = editor_with_lines(10);
    e.cursor = (5, 0);
    e.move_page_up(20);
    assert_eq!(e.cursor.0, 0);
}

#[test]
fn move_page_up_clamps_column_to_line_width() {
    let mut e = editor_with("a\nbbbbbb");
    e.cursor = (1, 6);
    e.move_page_up(1);
    assert_eq!(e.cursor.0, 0);
    assert_eq!(e.cursor.1, 1); // "a" は 1 文字
}

#[test]
fn move_page_up_at_first_line_keeps_position() {
    let mut e = editor_with_lines(5);
    e.cursor = (0, 0);
    e.move_page_up(20);
    assert_eq!(e.cursor.0, 0);
}

// ── move_h_page_left ──

#[test]
fn move_h_page_left_retreats_column_by_40() {
    let line: String = "x".repeat(100);
    let mut e = editor_with(&line);
    e.cursor = (0, 80);
    e.move_h_page_left();
    assert_eq!(e.cursor.1, 40);
}

#[test]
fn move_h_page_left_clamps_at_zero() {
    let line: String = "x".repeat(100);
    let mut e = editor_with(&line);
    e.cursor = (0, 30);
    e.move_h_page_left();
    assert_eq!(e.cursor.1, 0);
}

#[test]
fn move_h_page_left_at_zero_keeps_zero() {
    let mut e = editor_with("hello");
    e.cursor = (0, 0);
    e.move_h_page_left();
    assert_eq!(e.cursor.1, 0);
}

// ── move_h_page_right ──

#[test]
fn move_h_page_right_advances_column_by_40() {
    let line: String = "x".repeat(100);
    let mut e = editor_with(&line);
    e.cursor = (0, 10);
    e.move_h_page_right();
    assert_eq!(e.cursor.1, 50);
}

#[test]
fn move_h_page_right_clamps_to_line_end() {
    let line: String = "x".repeat(20);
    let mut e = editor_with(&line);
    e.cursor = (0, 0);
    e.move_h_page_right();
    assert_eq!(e.cursor.1, 20); // 行幅でクランプ
}

#[test]
fn move_h_page_right_at_line_end_keeps_position() {
    let mut e = editor_with("hello");
    e.cursor = (0, 5);
    e.move_h_page_right();
    assert_eq!(e.cursor.1, 5);
}

// ── Scrollable for EditorState ──

use crate::tui::scrollable::Scrollable as ScrollableTrait;

#[test]
fn scrollable_editor_move_one_down_advances_row() {
    let mut e = editor_with_lines(5);
    e.cursor = (1, 0);
    ScrollableTrait::move_one_down(&mut e);
    assert_eq!(e.cursor, (2, 0));
}

#[test]
fn scrollable_editor_move_one_up_retreats_row() {
    let mut e = editor_with_lines(5);
    e.cursor = (3, 0);
    ScrollableTrait::move_one_up(&mut e);
    assert_eq!(e.cursor, (2, 0));
}

#[test]
fn scrollable_editor_move_one_left_retreats_column() {
    let mut e = editor_with("abcde");
    e.cursor = (0, 3);
    ScrollableTrait::move_one_left(&mut e);
    assert_eq!(e.cursor, (0, 2));
}

#[test]
fn scrollable_editor_move_one_right_advances_column() {
    let mut e = editor_with("abcde");
    e.cursor = (0, 1);
    ScrollableTrait::move_one_right(&mut e);
    assert_eq!(e.cursor, (0, 2));
}

#[test]
fn scrollable_editor_scroll_to_top_jumps_to_zero_zero() {
    let mut e = editor_with_lines(10);
    e.cursor = (5, 3);
    ScrollableTrait::scroll_to_top(&mut e);
    assert_eq!(e.cursor, (0, 0));
}

#[test]
fn scrollable_editor_scroll_to_bottom_jumps_to_last_row_col_zero() {
    let mut e = editor_with_lines(10);
    e.cursor = (0, 0);
    ScrollableTrait::scroll_to_bottom(&mut e);
    assert_eq!(e.cursor, (9, 0));
}

#[test]
fn scrollable_editor_h_scroll_home_zeros_column() {
    let mut e = editor_with("hello world");
    e.cursor = (0, 7);
    ScrollableTrait::h_scroll_home(&mut e);
    assert_eq!(e.cursor.1, 0);
}

#[test]
fn scrollable_editor_h_scroll_end_jumps_to_line_end() {
    let mut e = editor_with("hello");
    e.cursor = (0, 0);
    ScrollableTrait::h_scroll_end(&mut e);
    assert_eq!(e.cursor.1, 5);
}

#[test]
fn scrollable_editor_page_down_delegates_to_move_page_down() {
    let mut e = editor_with_lines(50);
    e.cursor = (0, 0);
    ScrollableTrait::page_down(&mut e, 20);
    assert_eq!(e.cursor.0, 20);
}

#[test]
fn scrollable_editor_page_up_delegates_to_move_page_up() {
    let mut e = editor_with_lines(50);
    e.cursor = (30, 0);
    ScrollableTrait::page_up(&mut e, 20);
    assert_eq!(e.cursor.0, 10);
}

#[test]
fn scrollable_editor_h_page_left_retreats_40() {
    let line: String = "x".repeat(100);
    let mut e = editor_with(&line);
    e.cursor = (0, 80);
    ScrollableTrait::h_page_left(&mut e);
    assert_eq!(e.cursor.1, 40);
}

#[test]
fn scrollable_editor_h_page_right_advances_40() {
    let line: String = "x".repeat(100);
    let mut e = editor_with(&line);
    e.cursor = (0, 10);
    ScrollableTrait::h_page_right(&mut e);
    assert_eq!(e.cursor.1, 50);
}

// ── Visual モード ──

#[test]
fn enter_visual_sets_anchor_and_mode() {
    let mut e = editor_with("SELECT 1");
    e.cursor = (0, 2);
    e.enter_visual();
    assert_eq!(e.mode, EditorMode::Visual);
    assert_eq!(e.visual_anchor, Some((0, 2)));
}

#[test]
fn enter_visual_line_sets_anchor_and_linewise_mode() {
    let mut e = editor_with("AB\nCD");
    e.cursor = (1, 1);
    e.enter_visual_line();
    assert_eq!(e.mode, EditorMode::VisualLine);
    assert_eq!(e.visual_anchor, Some((1, 1)));
}

#[test]
fn enter_normal_clears_visual_anchor_and_pending() {
    let mut e = editor_with("AB");
    e.cursor = (0, 0);
    e.enter_visual();
    e.pending_chord = PendingChord::Operator('d');
    e.enter_normal();
    assert_eq!(e.mode, EditorMode::Normal);
    assert_eq!(e.visual_anchor, None);
    assert_eq!(e.pending_chord, PendingChord::None);
}

#[test]
fn selection_range_normalizes_when_cursor_before_anchor() {
    let mut e = editor_with("ABCDE");
    e.cursor = (0, 4);
    e.enter_visual();
    e.cursor = (0, 1);
    let r = e.selection_range().unwrap();
    assert_eq!(r, ((0, 1), (0, 4)));
}

#[test]
fn selection_text_charwise_within_line() {
    let mut e = editor_with("ABCDE");
    e.cursor = (0, 1);
    e.enter_visual();
    e.cursor = (0, 3);
    let (text, kind) = e.selection_text().unwrap();
    assert_eq!(text, "BCD");
    assert_eq!(kind, YankKind::Char);
}

#[test]
fn selection_text_linewise_includes_trailing_newline() {
    let mut e = editor_with("AB\nCD\nEF");
    e.cursor = (0, 1);
    e.enter_visual_line();
    e.cursor = (1, 0);
    let (text, kind) = e.selection_text().unwrap();
    assert_eq!(text, "AB\nCD\n");
    assert_eq!(kind, YankKind::Line);
}

#[test]
fn delete_selection_charwise_within_line_returns_to_normal() {
    let mut e = editor_with("ABCDE");
    e.cursor = (0, 1);
    e.enter_visual();
    e.cursor = (0, 3);
    let (text, _) = e.delete_selection().unwrap();
    assert_eq!(text, "BCD");
    assert_eq!(e.lines, vec!["AE"]);
    assert_eq!(e.mode, EditorMode::Normal);
    assert_eq!(e.visual_anchor, None);
}

#[test]
fn delete_selection_linewise_removes_full_lines() {
    let mut e = editor_with("AA\nBB\nCC\nDD");
    e.cursor = (1, 0);
    e.enter_visual_line();
    e.cursor = (2, 1);
    e.delete_selection().unwrap();
    assert_eq!(e.lines, vec!["AA", "DD"]);
    assert_eq!(e.mode, EditorMode::Normal);
}

#[test]
fn swap_visual_anchor_swaps_cursor_and_anchor() {
    let mut e = editor_with("ABCDE");
    e.cursor = (0, 1);
    e.enter_visual();
    e.cursor = (0, 4);
    e.swap_visual_anchor();
    assert_eq!(e.cursor, (0, 1));
    assert_eq!(e.visual_anchor, Some((0, 4)));
}

// ── 単語境界 ──

#[test]
fn inner_word_range_on_word_extends_both_sides() {
    let e = editor_with("SELECT id FROM users");
    let (s, end) = e.inner_word_range_at(0, 8).unwrap();
    // "id" は col 7..=8
    assert_eq!((s, end), (7, 8));
}

#[test]
fn forward_word_end_col_includes_trailing_whitespace() {
    let e = editor_with("SELECT id FROM users");
    // col 0 から dw 相当: "SELECT" + 空白 を消費 → 次単語 'i' の手前 (col 6)
    let end = e.forward_word_end_col_at(0, 0).unwrap();
    assert_eq!(end, 6);
}

// ── オペレータ ──

#[test]
fn dw_deletes_word_and_yanks() {
    let mut e = editor_with("SELECT id FROM users");
    e.cursor = (0, 0);
    e.delete_word_forward();
    assert_eq!(e.lines, vec!["id FROM users"]);
    assert_eq!(e.register.as_ref().unwrap().text, "SELECT ");
}

#[test]
fn diw_deletes_inner_word_keeps_surrounding_spaces() {
    let mut e = editor_with("SELECT id FROM users");
    e.cursor = (0, 7); // "id" の上
    e.delete_inner_word();
    assert_eq!(e.lines, vec!["SELECT  FROM users"]);
    assert_eq!(e.register.as_ref().unwrap().text, "id");
}

#[test]
fn ciw_deletes_inner_word_and_enters_insert() {
    let mut e = editor_with("foo bar baz");
    e.cursor = (0, 5); // "bar"
    e.change_inner_word();
    assert_eq!(e.lines, vec!["foo  baz"]);
    assert_eq!(e.mode, EditorMode::Insert);
}

#[test]
fn yy_yanks_line_with_newline() {
    let mut e = editor_with("hello\nworld");
    e.cursor = (0, 0);
    e.yank_line();
    let r = e.register.as_ref().unwrap();
    assert_eq!(r.text, "hello\n");
    assert_eq!(r.kind, YankKind::Line);
}

#[test]
fn paste_after_linewise_inserts_below() {
    let mut e = editor_with("A\nB");
    e.cursor = (0, 0);
    e.register = Some(Register { text: "X\n".to_string(), kind: YankKind::Line });
    e.paste_after();
    assert_eq!(e.lines, vec!["A", "X", "B"]);
    assert_eq!(e.cursor, (1, 0));
}

#[test]
fn paste_before_linewise_inserts_above() {
    let mut e = editor_with("A\nB");
    e.cursor = (1, 0);
    e.register = Some(Register { text: "X\n".to_string(), kind: YankKind::Line });
    e.paste_before();
    assert_eq!(e.lines, vec!["A", "X", "B"]);
}

#[test]
fn paste_after_charwise_inserts_after_cursor() {
    let mut e = editor_with("AB");
    e.cursor = (0, 0);
    e.register = Some(Register { text: "XY".to_string(), kind: YankKind::Char });
    e.paste_after();
    assert_eq!(e.lines, vec!["AXYB"]);
}

// ── 置換系 ──

#[test]
fn replace_char_keeps_normal_mode() {
    let mut e = editor_with("ABC");
    e.cursor = (0, 1);
    e.replace_char('X');
    assert_eq!(e.lines, vec!["AXC"]);
    assert_eq!(e.mode, EditorMode::Normal);
}

#[test]
fn substitute_char_deletes_and_enters_insert() {
    let mut e = editor_with("ABC");
    e.cursor = (0, 1);
    e.substitute_char();
    assert_eq!(e.lines, vec!["AC"]);
    assert_eq!(e.mode, EditorMode::Insert);
}

#[test]
fn substitute_line_clears_and_enters_insert() {
    let mut e = editor_with("ABC\nDEF");
    e.cursor = (1, 1);
    e.substitute_line();
    assert_eq!(e.lines, vec!["ABC", ""]);
    assert_eq!(e.cursor, (1, 0));
    assert_eq!(e.mode, EditorMode::Insert);
}

#[test]
fn join_lines_joins_with_space() {
    let mut e = editor_with("hello\n  world");
    e.cursor = (0, 0);
    e.join_lines();
    assert_eq!(e.lines, vec!["hello world"]);
}

#[test]
fn toggle_case_inverts_case_and_advances() {
    let mut e = editor_with("aB");
    e.cursor = (0, 0);
    e.toggle_case();
    assert_eq!(e.lines, vec!["AB"]);
    assert_eq!(e.cursor.1, 1);
}

// ── 検索 ──

#[test]
fn search_finds_matches_case_insensitive() {
    let mut e = editor_with("SELECT id FROM users\nselect name from users");
    e.search_start();
    e.search_push_char('s');
    e.search_push_char('e');
    e.search_push_char('l');
    e.search_push_char('e');
    e.search_push_char('c');
    e.search_push_char('t');
    assert_eq!(e.search.matches.len(), 2);
    assert_eq!(e.search.matches[0], (0, 0, 6));
    assert_eq!(e.search.matches[1], (1, 0, 6));
}

#[test]
fn search_confirm_jumps_to_first_match_after_cursor() {
    let mut e = editor_with("foo bar foo\nfoo bar");
    e.cursor = (0, 5);
    e.search_start();
    for ch in "foo".chars() {
        e.search_push_char(ch);
    }
    e.search_confirm();
    // cursor (0,5) 以降の最初のマッチは (0,8)
    assert_eq!(e.cursor, (0, 8));
}

#[test]
fn search_next_and_prev_wrap() {
    let mut e = editor_with("a x a x a");
    e.cursor = (0, 0);
    e.search_start();
    e.search_push_char('a');
    e.search_confirm();
    let first = e.cursor;
    e.next_match();
    assert_ne!(e.cursor, first);
    e.next_match();
    e.next_match();
    // 3つマッチ → 元に戻る
    assert_eq!(e.cursor, first);
    e.prev_match();
    assert_ne!(e.cursor, first);
}

#[test]
fn search_cancel_clears_state() {
    let mut e = editor_with("hello");
    e.search_start();
    e.search_push_char('h');
    e.search_cancel();
    assert!(e.search.query.is_empty());
    assert!(e.search.matches.is_empty());
    assert!(!e.search.active);
}
