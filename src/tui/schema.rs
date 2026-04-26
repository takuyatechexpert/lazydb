use std::cell::Cell;

use crate::tui::scrollable::Scrollable;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use super::{App, Panel};

// ── 状態 ──

#[derive(Debug, Clone)]
pub enum SchemaItem {
    Table { name: String, expanded: bool },
    Column { name: String, col_type: String },
}

#[derive(Debug, Clone)]
pub struct SchemaState {
    /// テーブル一覧（展開状態を保持）
    pub tables: Vec<TableEntry>,
    /// フラット化した表示リスト上のカーソル位置
    pub cursor: usize,
    /// 表示開始位置（描画時にカーソルが見える範囲を維持するため調整される）
    pub scroll_offset: Cell<usize>,
    /// 読み込み中フラグ
    pub loading: bool,
    /// スピナーフレーム
    pub spinner_frame: usize,
    /// `/` 検索モード入力中フラグ
    pub search_active: bool,
    /// 現在の検索クエリ（小文字 substring マッチ）
    pub search_query: String,
}

#[derive(Debug, Clone)]
pub struct TableEntry {
    pub name: String,
    pub expanded: bool,
    pub columns: Vec<ColumnEntry>,
    /// カラムが読み込み済みか
    pub columns_loaded: bool,
    /// カラム読み込み中
    pub columns_loading: bool,
}

#[derive(Debug, Clone)]
pub struct ColumnEntry {
    pub name: String,
    pub col_type: String,
    pub is_primary_key: bool,
}

impl SchemaState {
    pub fn new() -> Self {
        Self {
            tables: Vec::new(),
            cursor: 0,
            scroll_offset: Cell::new(0),
            loading: false,
            spinner_frame: 0,
            search_active: false,
            search_query: String::new(),
        }
    }

    // ── 検索モード ──

    /// `/` を押されたとき: 検索モードに入りクエリをクリアする
    pub fn enter_search(&mut self) {
        self.search_active = true;
        self.search_query.clear();
    }

    /// Esc: 検索モードを抜けてクエリも破棄する
    pub fn cancel_search(&mut self) {
        self.search_active = false;
        self.search_query.clear();
    }

    /// Enter: 検索モードを抜けてクエリは保持する（n/N で再ジャンプ可能）
    pub fn confirm_search(&mut self) {
        self.search_active = false;
    }

    /// 検索クエリに 1 文字追加し、現在カーソル位置から前方の最初のマッチへジャンプする
    pub fn push_search_char(&mut self, ch: char) {
        self.search_query.push(ch);
        if let Some(pos) = self.find_match(self.cursor, true) {
            self.cursor = pos;
        }
    }

    /// 検索クエリの末尾 1 文字を削除し、再度カーソル位置から前方検索する
    pub fn pop_search_char(&mut self) {
        self.search_query.pop();
        if !self.search_query.is_empty() {
            if let Some(pos) = self.find_match(self.cursor, true) {
                self.cursor = pos;
            }
        }
    }

    /// `n`: カーソルより後ろの次のマッチへ移動
    pub fn find_next(&mut self) -> bool {
        if self.search_query.is_empty() {
            return false;
        }
        let start = self.cursor.saturating_add(1);
        if let Some(pos) = self.find_match(start, true) {
            self.cursor = pos;
            true
        } else {
            false
        }
    }

    /// `N`: カーソルより前の前のマッチへ移動
    pub fn find_prev(&mut self) -> bool {
        if self.search_query.is_empty() {
            return false;
        }
        let start = self.cursor.saturating_sub(1);
        if let Some(pos) = self.find_match(start, false) {
            self.cursor = pos;
            true
        } else {
            false
        }
    }

    /// `start` を起点に flat_items 内のテーブル名マッチ位置を探す。
    /// - `forward=true`: start から末尾→先頭へラップ
    /// - `forward=false`: start から先頭→末尾へラップ
    ///
    /// 大文字小文字無視の substring マッチ。
    fn find_match(&self, start: usize, forward: bool) -> Option<usize> {
        let items = self.flat_items();
        if items.is_empty() || self.search_query.is_empty() {
            return None;
        }
        let needle = self.search_query.to_lowercase();
        let len = items.len();

        // 末尾を超えた場合は wrap させる: forward なら 0 から、backward なら末尾から走査
        let order: Vec<usize> = if forward {
            if start >= len {
                (0..len).collect()
            } else {
                (start..len).chain(0..start).collect()
            }
        } else {
            let s = start.min(len - 1);
            let mut v: Vec<usize> = (0..=s).rev().collect();
            v.extend((s + 1..len).rev());
            v
        };

        for i in order {
            if let SchemaItem::Table { name, .. } = &items[i] {
                if name.to_lowercase().contains(&needle) {
                    return Some(i);
                }
            }
        }
        None
    }

    /// フラット化した表示アイテムリストを生成
    pub fn flat_items(&self) -> Vec<SchemaItem> {
        let mut items = Vec::new();
        for table in &self.tables {
            items.push(SchemaItem::Table {
                name: table.name.clone(),
                expanded: table.expanded,
            });
            if table.expanded {
                for col in &table.columns {
                    items.push(SchemaItem::Column {
                        name: col.name.clone(),
                        col_type: col.col_type.clone(),
                    });
                }
            }
        }
        items
    }

    /// カーソルが指しているテーブル名を返す（テーブル行またはカラム行の親テーブル）
    pub fn current_table_name(&self) -> Option<String> {
        let items = self.flat_items();
        if items.is_empty() {
            return None;
        }
        let idx = self.cursor.min(items.len().saturating_sub(1));
        match &items[idx] {
            SchemaItem::Table { name, .. } => Some(name.clone()),
            SchemaItem::Column { .. } => {
                // カーソルより上にある最も近い Table を探す
                for i in (0..idx).rev() {
                    if let SchemaItem::Table { name, .. } = &items[i] {
                        return Some(name.clone());
                    }
                }
                None
            }
        }
    }

    /// カーソルを1つ下に移動
    pub fn move_down(&mut self) {
        let len = self.flat_items().len();
        if len > 0 {
            self.cursor = (self.cursor + 1).min(len - 1);
        }
    }

    /// カーソルを1つ上に移動
    pub fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// テーブルの展開/折りたたみをトグル
    pub fn toggle_expand(&mut self) -> Option<ToggleResult> {
        let items = self.flat_items();
        if items.is_empty() {
            return None;
        }
        let idx = self.cursor.min(items.len().saturating_sub(1));
        match &items[idx] {
            SchemaItem::Table { name, expanded } => {
                let table_idx = self.tables.iter().position(|t| t.name == *name)?;
                let table = &mut self.tables[table_idx];
                if *expanded {
                    table.expanded = false;
                    None
                } else {
                    table.expanded = true;
                    if !table.columns_loaded && !table.columns_loading {
                        table.columns_loading = true;
                        Some(ToggleResult::NeedFetchColumns(name.clone()))
                    } else {
                        None
                    }
                }
            }
            _ => None,
        }
    }

    pub fn tick(&mut self) {
        if self.loading || self.tables.iter().any(|t| t.columns_loading) {
            self.spinner_frame = (self.spinner_frame + 1) % 4;
        }
    }

    /// 指定テーブルの PK カラム名リストを返す。
    /// - テーブル名の照合は `eq_ignore_ascii_case`
    /// - 該当テーブルが未読込または存在しない場合 → None
    /// - 展開済みで PK が無ければ Some(vec![])
    pub fn primary_keys_for(&self, table: &str) -> Option<Vec<String>> {
        let entry = self
            .tables
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(table))?;
        if !entry.columns_loaded {
            return None;
        }
        let pks: Vec<String> = entry
            .columns
            .iter()
            .filter(|c| c.is_primary_key)
            .map(|c| c.name.clone())
            .collect();
        Some(pks)
    }

    /// 指定テーブルのカラムがロード済みか（照合は `eq_ignore_ascii_case`）
    pub fn columns_loaded(&self, table: &str) -> bool {
        self.tables
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(table))
            .map(|t| t.columns_loaded)
            .unwrap_or(false)
    }
}

pub enum ToggleResult {
    NeedFetchColumns(String),
}

impl Scrollable for SchemaState {
    fn move_one_down(&mut self) {
        self.move_down();
    }

    fn move_one_up(&mut self) {
        self.move_up();
    }

    fn move_one_left(&mut self) {
        // Schema は横スクロール状態を持たないため no-op
    }

    fn move_one_right(&mut self) {
        // Schema は横スクロール状態を持たないため no-op
    }

    fn scroll_to_top(&mut self) {
        self.cursor = 0;
    }

    fn scroll_to_bottom(&mut self) {
        self.cursor = self.flat_items().len().saturating_sub(1);
    }

    fn h_scroll_home(&mut self) {
        // no-op
    }

    fn h_scroll_end(&mut self) {
        // no-op
    }

    fn page_down(&mut self, page_size: usize) {
        let len = self.flat_items().len();
        if len == 0 {
            return;
        }
        self.cursor = (self.cursor + page_size).min(len - 1);
    }

    fn page_up(&mut self, page_size: usize) {
        self.cursor = self.cursor.saturating_sub(page_size);
    }

    fn h_page_left(&mut self) {
        // no-op
    }

    fn h_page_right(&mut self) {
        // no-op
    }

    fn center_on_cursor(&mut self, page_size: usize) {
        // カーソル行を画面中央に: view top = cursor - page_size/2
        let half = page_size / 2;
        self.scroll_offset.set(self.cursor.saturating_sub(half));
    }
}

// ── 描画 ──

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸"];

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.active_panel == Panel::Schema;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Schema ")
        .borders(Borders::ALL)
        .border_style(border_style);

    if app.active_connection.is_none() {
        let p = Paragraph::new("  接続を選択してください").block(block);
        f.render_widget(p, area);
        return;
    }

    if app.schema.loading {
        let spinner = SPINNER[app.schema.spinner_frame % SPINNER.len()];
        let p = Paragraph::new(format!("  {} 読み込み中...", spinner)).block(block);
        f.render_widget(p, area);
        return;
    }

    if app.schema.tables.is_empty() {
        let p = Paragraph::new("  テーブルがありません").block(block);
        f.render_widget(p, area);
        return;
    }

    // 検索バーを描画する場合は inner の最下行を 1 行検索バー用に確保する
    let show_search_bar = app.schema.search_active || !app.schema.search_query.is_empty();
    let search_bar_area = if show_search_bar && area.height >= 4 {
        Some(Rect {
            x: area.x + 1,
            y: area.y + area.height - 2,
            width: area.width.saturating_sub(2),
            height: 1,
        })
    } else {
        None
    };
    let list_area = area;

    let items = app.schema.flat_items();
    let needle_lower = app.schema.search_query.to_lowercase();
    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == app.schema.cursor;
            match item {
                SchemaItem::Table { name, expanded } => {
                    let prefix = if *expanded { "▼ " } else { "▶ " };
                    let style = if is_selected && is_focused {
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    };
                    // テーブル行にカラム読み込み中スピナーを表示
                    let table = app.schema.tables.iter().find(|t| t.name == *name);
                    let suffix = if table.is_some_and(|t| t.columns_loading) {
                        let s = SPINNER[app.schema.spinner_frame % SPINNER.len()];
                        format!(" {}", s)
                    } else {
                        String::new()
                    };
                    let cursor_mark = Span::raw(if is_selected && is_focused { ">" } else { " " });
                    let prefix_span = Span::styled(prefix.to_string(), style);

                    // 検索クエリにマッチする部分をハイライト
                    let name_spans = if !needle_lower.is_empty() {
                        highlight_match(name, &needle_lower, style)
                    } else {
                        vec![Span::styled(name.clone(), style)]
                    };

                    let mut spans = vec![cursor_mark, prefix_span];
                    spans.extend(name_spans);
                    if !suffix.is_empty() {
                        spans.push(Span::styled(suffix, style));
                    }
                    ListItem::new(Line::from(spans))
                }
                SchemaItem::Column { name, col_type } => {
                    let style = if is_selected && is_focused {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    ListItem::new(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(format!("{} ", name), style),
                        Span::styled(
                            col_type.clone(),
                            Style::default().fg(Color::Yellow),
                        ),
                    ]))
                }
            }
        })
        .collect();

    // スクロール: カーソルが画面外に出たときだけオフセットを動かす
    // 検索バーが出ているときは List 下端 1 行分を予約
    let inner_height = (list_area.height.saturating_sub(2) as usize)
        .saturating_sub(if search_bar_area.is_some() { 1 } else { 0 });
    let total = list_items.len();
    let cursor = app.schema.cursor;
    let mut offset = app.schema.scroll_offset.get();

    if inner_height == 0 {
        offset = 0;
    } else {
        // 全体が収まるならオフセット 0
        if total <= inner_height {
            offset = 0;
        } else {
            // 末尾以降にはみ出していたら縮める
            let max_offset = total - inner_height;
            if offset > max_offset {
                offset = max_offset;
            }
            // カーソルが画面より上 → 上にスクロール
            if cursor < offset {
                offset = cursor;
            }
            // カーソルが画面より下 → 下にスクロール
            if cursor >= offset + inner_height {
                offset = cursor + 1 - inner_height;
            }
        }
    }
    app.schema.scroll_offset.set(offset);

    let visible: Vec<ListItem> = list_items
        .into_iter()
        .skip(offset)
        .take(inner_height.max(1))
        .collect();
    let list = List::new(visible).block(block);
    f.render_widget(list, list_area);

    // 検索バー（border 内の最下部に上書き描画）
    if let Some(bar) = search_bar_area {
        let cursor_glyph = if app.schema.search_active { "█" } else { "" };
        let prompt_color = if app.schema.search_active {
            Color::Yellow
        } else {
            Color::DarkGray
        };
        let line = Line::from(vec![
            Span::styled("/", Style::default().fg(prompt_color)),
            Span::raw(app.schema.search_query.clone()),
            Span::styled(cursor_glyph, Style::default().fg(Color::Gray)),
        ]);
        f.render_widget(Paragraph::new(line), bar);
    }
}

/// 検索クエリにマッチする部分文字列を Yellow で強調した Span 列を返す。
/// `needle` は事前に小文字化しておくこと。
fn highlight_match<'a>(haystack: &'a str, needle: &str, base: Style) -> Vec<Span<'a>> {
    let lower = haystack.to_lowercase();
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut last = 0usize;
    let mut search_from = 0usize;
    while let Some(rel) = lower[search_from..].find(needle) {
        let start = search_from + rel;
        let end = start + needle.len();
        if start > last {
            spans.push(Span::styled(haystack[last..start].to_string(), base));
        }
        spans.push(Span::styled(
            haystack[start..end].to_string(),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        last = end;
        search_from = end;
    }
    if last < haystack.len() {
        spans.push(Span::styled(haystack[last..].to_string(), base));
    }
    if spans.is_empty() {
        spans.push(Span::styled(haystack.to_string(), base));
    }
    spans
}
#[cfg(test)]
mod tests {
    use super::*;

    fn schema_with(table_specs: &[(&str, bool, &[&str])]) -> SchemaState {
        // (table_name, expanded, columns)
        let mut s = SchemaState::new();
        for (name, expanded, cols) in table_specs {
            s.tables.push(TableEntry {
                name: (*name).to_string(),
                expanded: *expanded,
                columns_loaded: !cols.is_empty(),
                columns_loading: false,
                columns: cols
                    .iter()
                    .map(|c| ColumnEntry {
                        name: (*c).to_string(),
                        col_type: "text".to_string(),
                        is_primary_key: false,
                    })
                    .collect(),
            });
        }
        s
    }

    fn schema_with_n_tables(n: usize) -> SchemaState {
        let mut s = SchemaState::new();
        for i in 0..n {
            s.tables.push(TableEntry {
                name: format!("t{}", i),
                expanded: false,
                columns_loaded: false,
                columns_loading: false,
                columns: Vec::new(),
            });
        }
        s
    }

    // ── move_one_down / move_one_up ──

    #[test]
    fn scrollable_schema_move_one_down_advances_cursor() {
        let mut s = schema_with_n_tables(5);
        s.cursor = 1;
        s.move_one_down();
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn scrollable_schema_move_one_down_clamps_at_last() {
        let mut s = schema_with_n_tables(3);
        s.cursor = 2;
        s.move_one_down();
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn scrollable_schema_move_one_up_retreats_cursor() {
        let mut s = schema_with_n_tables(5);
        s.cursor = 3;
        s.move_one_up();
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn scrollable_schema_move_one_up_clamps_at_zero() {
        let mut s = schema_with_n_tables(3);
        s.cursor = 0;
        s.move_one_up();
        assert_eq!(s.cursor, 0);
    }

    // ── 横操作は no-op ──

    #[test]
    fn scrollable_schema_move_one_left_is_noop() {
        let mut s = schema_with_n_tables(5);
        s.cursor = 2;
        s.move_one_left();
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn scrollable_schema_move_one_right_is_noop() {
        let mut s = schema_with_n_tables(5);
        s.cursor = 2;
        s.move_one_right();
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn scrollable_schema_h_scroll_home_is_noop() {
        let mut s = schema_with_n_tables(5);
        s.cursor = 3;
        s.h_scroll_home();
        assert_eq!(s.cursor, 3);
    }

    #[test]
    fn scrollable_schema_h_scroll_end_is_noop() {
        let mut s = schema_with_n_tables(5);
        s.cursor = 3;
        s.h_scroll_end();
        assert_eq!(s.cursor, 3);
    }

    #[test]
    fn scrollable_schema_h_page_left_is_noop() {
        let mut s = schema_with_n_tables(5);
        s.cursor = 3;
        s.h_page_left();
        assert_eq!(s.cursor, 3);
    }

    #[test]
    fn scrollable_schema_h_page_right_is_noop() {
        let mut s = schema_with_n_tables(5);
        s.cursor = 3;
        s.h_page_right();
        assert_eq!(s.cursor, 3);
    }

    // ── scroll_to_top / scroll_to_bottom ──

    #[test]
    fn scrollable_schema_scroll_to_top_zeros_cursor() {
        let mut s = schema_with_n_tables(5);
        s.cursor = 4;
        s.scroll_to_top();
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn scrollable_schema_scroll_to_bottom_lands_on_last_item() {
        let mut s = schema_with_n_tables(5);
        s.cursor = 0;
        s.scroll_to_bottom();
        assert_eq!(s.cursor, 4); // 0..=4 の末尾
    }

    #[test]
    fn scrollable_schema_scroll_to_bottom_with_expanded_table_uses_flat_len() {
        // テーブルが expanded=true でカラムも flat_items に含まれる
        let mut s = schema_with(&[("t1", true, &["c1", "c2", "c3"]), ("t2", false, &[])]);
        // flat_items: [t1, c1, c2, c3, t2] = 5 件
        assert_eq!(s.flat_items().len(), 5);
        s.cursor = 0;
        s.scroll_to_bottom();
        assert_eq!(s.cursor, 4);
    }

    #[test]
    fn scrollable_schema_scroll_to_bottom_with_no_items_clamps_to_zero() {
        let mut s = SchemaState::new();
        s.scroll_to_bottom();
        // saturating_sub(1) で 0
        assert_eq!(s.cursor, 0);
    }

    // ── page_down / page_up ──

    #[test]
    fn scrollable_schema_page_down_advances_by_page_size() {
        let mut s = schema_with_n_tables(50);
        s.cursor = 0;
        s.page_down(20);
        assert_eq!(s.cursor, 20);
    }

    #[test]
    fn scrollable_schema_page_down_clamps_at_last() {
        let mut s = schema_with_n_tables(10);
        s.cursor = 5;
        s.page_down(20);
        assert_eq!(s.cursor, 9);
    }

    #[test]
    fn scrollable_schema_page_down_with_empty_items_does_nothing() {
        let mut s = SchemaState::new();
        s.cursor = 0;
        s.page_down(20);
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn scrollable_schema_page_up_retreats_by_page_size() {
        let mut s = schema_with_n_tables(50);
        s.cursor = 30;
        s.page_up(20);
        assert_eq!(s.cursor, 10);
    }

    #[test]
    fn scrollable_schema_page_up_clamps_at_zero() {
        let mut s = schema_with_n_tables(10);
        s.cursor = 5;
        s.page_up(20);
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn scrollable_schema_page_up_with_empty_items_keeps_zero() {
        let mut s = SchemaState::new();
        s.cursor = 0;
        s.page_up(20);
        assert_eq!(s.cursor, 0);
    }

    // ── 検索モード ──

    /// 検索テスト用: マッチがちょうど 2 件（idx 2, 4）に来るようにした名前セット。
    /// "xx" を含むのは "ccc_xx" と "eee_xx" のみ。
    fn schema_for_search() -> SchemaState {
        let mut s = SchemaState::new();
        for n in &["aaa", "bbb", "ccc_xx", "ddd", "eee_xx"] {
            s.tables.push(TableEntry {
                name: n.to_string(),
                expanded: false,
                columns_loaded: false,
                columns_loading: false,
                columns: Vec::new(),
            });
        }
        s
    }

    #[test]
    fn search_enter_clears_query_and_activates() {
        let mut s = schema_for_search();
        s.search_query = "old".to_string();
        s.enter_search();
        assert!(s.search_active);
        assert_eq!(s.search_query, "");
    }

    #[test]
    fn search_push_char_jumps_cursor_to_first_match() {
        let mut s = schema_for_search();
        s.cursor = 0;
        s.enter_search();
        s.push_search_char('x');
        // "x" を含むのは "ccc_xx"(2) と "eee_xx"(4)。前方検索で 2 にジャンプ
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn search_is_case_insensitive() {
        let mut s = schema_for_search();
        s.cursor = 0;
        s.enter_search();
        s.push_search_char('X'); // 大文字でも小文字 'xx' にマッチ
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn search_pop_char_keeps_query_navigation_consistent() {
        let mut s = schema_for_search();
        s.cursor = 0;
        s.enter_search();
        s.push_search_char('x');
        s.push_search_char('x');
        assert_eq!(s.cursor, 2); // ccc_xx
        s.pop_search_char();
        // クエリは "x" のまま。現在 cursor=2 から前方検索 → 自身がマッチするので 2 のまま
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn find_next_advances_to_next_match_and_wraps() {
        let mut s = schema_for_search();
        s.search_query = "xx".to_string();
        s.cursor = 0;
        assert!(s.find_next()); // 0 → 2
        assert_eq!(s.cursor, 2);
        assert!(s.find_next()); // 2 → 4
        assert_eq!(s.cursor, 4);
        assert!(s.find_next()); // 4 → ラップして 2
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn find_prev_retreats_to_previous_match_and_wraps() {
        let mut s = schema_for_search();
        s.search_query = "xx".to_string();
        s.cursor = 4; // eee_xx
        assert!(s.find_prev()); // 4 → 2
        assert_eq!(s.cursor, 2);
        assert!(s.find_prev()); // 2 → ラップして 4
        assert_eq!(s.cursor, 4);
    }

    #[test]
    fn find_next_with_empty_query_returns_false() {
        let mut s = schema_for_search();
        s.search_query.clear();
        assert!(!s.find_next());
    }

    #[test]
    fn find_next_no_match_returns_false_without_moving() {
        let mut s = schema_for_search();
        s.cursor = 2;
        s.search_query = "xyz_no_match".to_string();
        assert!(!s.find_next());
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn cancel_search_clears_query() {
        let mut s = schema_for_search();
        s.enter_search();
        s.push_search_char('u');
        s.cancel_search();
        assert!(!s.search_active);
        assert_eq!(s.search_query, "");
    }

    #[test]
    fn confirm_search_keeps_query_for_n_navigation() {
        let mut s = schema_for_search();
        s.enter_search();
        s.push_search_char('u');
        s.confirm_search();
        assert!(!s.search_active);
        assert_eq!(s.search_query, "u");
    }

    #[test]
    fn search_skips_column_items_and_only_matches_tables() {
        let mut s = schema_with(&[
            ("users", true, &["name", "email"]),
            ("orders", false, &[]),
        ]);
        // flat_items = [users(0), name(1), email(2), orders(3)]
        s.cursor = 0;
        s.search_query = "name".to_string();
        // "name" はカラム名のみで、テーブル名にはマッチしない
        assert!(!s.find_next());
    }
}
