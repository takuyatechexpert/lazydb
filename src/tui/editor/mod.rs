
mod operators;
mod render;
mod search;
mod visual;
mod word;
mod state;

pub use operators::{Register, YankKind};
pub use render::render;
use search::Search;

// App / Panel は不要: render は EditorState を直接受け取る

// ── SQL キーワード ──

pub(super) const SQL_KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "AND", "OR", "NOT", "IN", "IS", "NULL", "AS",
    "JOIN", "LEFT", "RIGHT", "INNER", "OUTER", "CROSS", "ON", "USING",
    "ORDER", "BY", "ASC", "DESC", "GROUP", "HAVING", "LIMIT", "OFFSET",
    "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE", "TRUNCATE",
    "CREATE", "ALTER", "DROP", "TABLE", "INDEX", "VIEW", "SCHEMA",
    "WITH", "RECURSIVE", "UNION", "ALL", "EXCEPT", "INTERSECT",
    "CASE", "WHEN", "THEN", "ELSE", "END", "CAST", "EXISTS",
    "BETWEEN", "LIKE", "ILIKE", "DISTINCT", "COUNT", "SUM", "AVG", "MIN", "MAX",
    "TRUE", "FALSE", "FETCH", "FIRST", "NEXT", "ROWS", "ONLY",
    "BEGIN", "COMMIT", "ROLLBACK", "RETURNING",
];

// 単語判定ヘルパ・単語境界系・単語移動系は word.rs に分離

// ── 状態 ──

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EditorMode {
    Normal,
    Insert,
    Visual,
    VisualLine,
}

// YankKind は operators.rs に分離

/// Normal モード中のチョード待機状態。
/// vim の `r{ch}` / `gg` / `dd dw d{i}w` 系を表現する。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PendingChord {
    /// チョード待機なし
    None,
    /// `r` 押下後、置換文字を待っている
    Replace,
    /// `g` 押下後、`gg` の 2 文字目を待っている
    GotoG,
    /// `d` / `y` / `c` 押下後、二段目（dd/dw/di...）を待っている
    Operator(char),
    /// `di` / `yi` / `ci` 押下後、テキストオブジェクト指定を待っている
    OperatorInner(char),
}

// Register は operators.rs に分離

// Search 構造体と impl は search.rs に分離

#[derive(Debug, Clone)]
struct EditorSnapshot {
    lines: Vec<String>,
    cursor: (usize, usize),
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub candidates: Vec<String>,
    pub cursor: usize,
    pub prefix: String,
    pub active: bool,
}

impl Completion {
    pub fn new() -> Self {
        Self {
            candidates: Vec::new(),
            cursor: 0,
            prefix: String::new(),
            active: false,
        }
    }

    pub fn close(&mut self) {
        self.active = false;
        self.candidates.clear();
        self.cursor = 0;
        self.prefix.clear();
    }

    pub fn next(&mut self) {
        if !self.candidates.is_empty() {
            self.cursor = (self.cursor + 1) % self.candidates.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.candidates.is_empty() {
            self.cursor = if self.cursor == 0 {
                self.candidates.len() - 1
            } else {
                self.cursor - 1
            };
        }
    }

    pub fn selected(&self) -> Option<&str> {
        self.candidates.get(self.cursor).map(|s| s.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct EditorState {
    pub lines: Vec<String>,
    /// (行, 列)
    pub cursor: (usize, usize),
    pub scroll_offset: usize,
    /// 横スクロール（カーソル列が画面幅を超えた場合のオフセット）
    pub h_scroll_offset: usize,
    pub mode: EditorMode,
    undo_stack: Vec<EditorSnapshot>,
    redo_stack: Vec<EditorSnapshot>,
    /// クエリ実行中
    pub executing: bool,
    /// オートコンプリート
    pub completion: Completion,
    /// Visual モード開始位置 (行, 列)
    pub visual_anchor: Option<(usize, usize)>,
    /// Normal モード中のチョード待機状態（r/gg/d系/y系/c系）
    pub pending_chord: PendingChord,
    /// 内部レジスタ（y / d で更新）
    pub register: Option<Register>,
    /// 検索状態
    pub search: Search,
}

// ── ヘルパー ──

pub(super) fn char_count(s: &str) -> usize {
    s.chars().count()
}

pub(super) fn char_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}


#[cfg(test)]
mod tests;
