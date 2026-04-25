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
        }
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

    let items = app.schema.flat_items();
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
                    ListItem::new(Line::from(vec![
                        Span::raw(if is_selected && is_focused { ">" } else { " " }),
                        Span::styled(format!("{}{}{}", prefix, name, suffix), style),
                    ]))
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
    let inner_height = area.height.saturating_sub(2) as usize; // borders
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

    let visible: Vec<ListItem> = list_items.into_iter().skip(offset).collect();
    let list = List::new(visible).block(block);
    f.render_widget(list, area);
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
}
