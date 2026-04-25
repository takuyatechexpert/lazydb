use crate::tui::scrollable::Scrollable;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

// App / Panel は不要: render は EditorState を直接受け取る

// ── SQL キーワード ──

const SQL_KEYWORDS: &[&str] = &[
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

// ── 状態 ──

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EditorMode {
    Normal,
    Insert,
}

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
}

impl EditorState {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: (0, 0),
            scroll_offset: 0,
            h_scroll_offset: 0,
            mode: EditorMode::Normal,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            executing: false,
            completion: Completion::new(),
        }
    }

    pub fn set_content(&mut self, content: &str) {
        self.save_snapshot();
        self.lines = content.lines().map(String::from).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor = (self.lines.len() - 1, self.lines.last().map_or(0, |l| l.len()));
        self.redo_stack.clear();
    }

    /// カーソル位置に関係なく、バッファ末尾に text を追記する。
    /// text が改行で始まっていない場合、末尾行が非空なら改行を挟む。
    /// undo スナップショットを保存する。
    pub fn append_text(&mut self, text: &str) {
        self.save_snapshot();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        // text が改行で始まらず、末尾行が非空なら改行（新しい空行）を挟む
        let starts_with_newline = text.starts_with('\n');
        let last_line_nonempty = self.lines.last().map(|l| !l.is_empty()).unwrap_or(false);
        if !starts_with_newline && last_line_nonempty {
            self.lines.push(String::new());
        }
        // text を行に分割（split('\n') は先頭/末尾の改行を空要素として保持する）
        // 先頭は末尾行へ追記、残りは新しい行として追加
        let mut parts = text.split('\n');
        if let Some(first) = parts.next() {
            let last = self.lines.last_mut().expect("lines is non-empty");
            last.push_str(first);
        }
        for part in parts {
            self.lines.push(part.to_string());
        }
        // カーソルを末尾に移動
        let last_row = self.lines.len() - 1;
        let last_col = char_count(&self.lines[last_row]);
        self.cursor = (last_row, last_col);
    }

    fn save_snapshot(&mut self) {
        self.undo_stack.push(EditorSnapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
        });
        if self.undo_stack.len() > 50 {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) {
        if let Some(snap) = self.undo_stack.pop() {
            self.redo_stack.push(EditorSnapshot {
                lines: self.lines.clone(),
                cursor: self.cursor,
            });
            self.lines = snap.lines;
            self.cursor = snap.cursor;
        }
    }

    pub fn redo(&mut self) {
        if let Some(snap) = self.redo_stack.pop() {
            self.undo_stack.push(EditorSnapshot {
                lines: self.lines.clone(),
                cursor: self.cursor,
            });
            self.lines = snap.lines;
            self.cursor = snap.cursor;
        }
    }

    pub fn insert_char(&mut self, ch: char) {
        self.save_snapshot();
        let (row, col) = self.cursor;
        if row < self.lines.len() {
            let line = &mut self.lines[row];
            let byte_idx = char_to_byte_idx(line, col);
            line.insert(byte_idx, ch);
            self.cursor.1 += 1;
        }
    }

    pub fn insert_newline(&mut self) {
        self.save_snapshot();
        let (row, col) = self.cursor;
        if row < self.lines.len() {
            let byte_idx = char_to_byte_idx(&self.lines[row], col);
            let rest = self.lines[row][byte_idx..].to_string();
            self.lines[row].truncate(byte_idx);
            self.lines.insert(row + 1, rest);
            self.cursor = (row + 1, 0);
        }
    }

    pub fn backspace(&mut self) {
        let (row, col) = self.cursor;
        if col > 0 {
            self.save_snapshot();
            let byte_idx = char_to_byte_idx(&self.lines[row], col);
            let prev_byte_idx = char_to_byte_idx(&self.lines[row], col - 1);
            self.lines[row].replace_range(prev_byte_idx..byte_idx, "");
            self.cursor.1 -= 1;
        } else if row > 0 {
            self.save_snapshot();
            let current_line = self.lines.remove(row);
            let prev_len = char_count(&self.lines[row - 1]);
            self.lines[row - 1].push_str(&current_line);
            self.cursor = (row - 1, prev_len);
        }
    }

    pub fn delete(&mut self) {
        let (row, col) = self.cursor;
        let line_chars = char_count(&self.lines[row]);
        if col < line_chars {
            self.save_snapshot();
            let byte_idx = char_to_byte_idx(&self.lines[row], col);
            let next_byte_idx = char_to_byte_idx(&self.lines[row], col + 1);
            self.lines[row].replace_range(byte_idx..next_byte_idx, "");
        } else if row + 1 < self.lines.len() {
            self.save_snapshot();
            let next_line = self.lines.remove(row + 1);
            self.lines[row].push_str(&next_line);
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor.1 > 0 {
            self.cursor.1 -= 1;
        } else if self.cursor.0 > 0 {
            self.cursor.0 -= 1;
            self.cursor.1 = char_count(&self.lines[self.cursor.0]);
        }
    }

    pub fn move_right(&mut self) {
        let line_chars = char_count(&self.lines[self.cursor.0]);
        if self.cursor.1 < line_chars {
            self.cursor.1 += 1;
        } else if self.cursor.0 + 1 < self.lines.len() {
            self.cursor.0 += 1;
            self.cursor.1 = 0;
        }
    }

    pub fn move_up(&mut self) {
        if self.cursor.0 > 0 {
            self.cursor.0 -= 1;
            let line_chars = char_count(&self.lines[self.cursor.0]);
            self.cursor.1 = self.cursor.1.min(line_chars);
        }
    }

    pub fn move_down(&mut self) {
        if self.cursor.0 + 1 < self.lines.len() {
            self.cursor.0 += 1;
            let line_chars = char_count(&self.lines[self.cursor.0]);
            self.cursor.1 = self.cursor.1.min(line_chars);
        }
    }

    pub fn move_home(&mut self) {
        self.cursor.1 = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor.1 = char_count(&self.lines[self.cursor.0]);
    }

    // ── vim Normal モード操作 ──

    /// Insert モードに遷移
    pub fn enter_insert(&mut self) {
        self.mode = EditorMode::Insert;
    }

    /// Insert モード: カーソルの右（append）
    pub fn enter_insert_after(&mut self) {
        let line_chars = char_count(&self.lines[self.cursor.0]);
        if self.cursor.1 < line_chars {
            self.cursor.1 += 1;
        }
        self.mode = EditorMode::Insert;
    }

    /// Insert モード: 行末（Append）
    pub fn enter_insert_end(&mut self) {
        self.cursor.1 = char_count(&self.lines[self.cursor.0]);
        self.mode = EditorMode::Insert;
    }

    /// Insert モード: 下に空行挿入
    pub fn enter_insert_below(&mut self) {
        self.save_snapshot();
        let row = self.cursor.0;
        self.lines.insert(row + 1, String::new());
        self.cursor = (row + 1, 0);
        self.mode = EditorMode::Insert;
    }

    /// Insert モード: 上に空行挿入
    pub fn enter_insert_above(&mut self) {
        self.save_snapshot();
        let row = self.cursor.0;
        self.lines.insert(row, String::new());
        self.cursor = (row, 0);
        self.mode = EditorMode::Insert;
    }

    /// Normal モードに戻る
    pub fn enter_normal(&mut self) {
        self.mode = EditorMode::Normal;
        // vim の挙動: Insert → Normal でカーソルが1つ左に戻る
        let line_chars = char_count(&self.lines[self.cursor.0]);
        if self.cursor.1 > 0 && self.cursor.1 >= line_chars && line_chars > 0 {
            self.cursor.1 = line_chars - 1;
        }
    }

    /// 次の単語先頭へ (w)
    pub fn move_word_forward(&mut self) {
        let (row, col) = self.cursor;
        let line = &self.lines[row];
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();

        if col < len {
            // 現在の文字種をスキップ
            let mut i = col;
            let is_word = chars[i].is_alphanumeric() || chars[i] == '_';
            while i < len && ((chars[i].is_alphanumeric() || chars[i] == '_') == is_word) {
                i += 1;
            }
            // 空白スキップ
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            if i < len {
                self.cursor.1 = i;
                return;
            }
        }
        // 次の行の先頭
        if row + 1 < self.lines.len() {
            self.cursor.0 = row + 1;
            self.cursor.1 = 0;
            // 空白スキップ
            let next_chars: Vec<char> = self.lines[self.cursor.0].chars().collect();
            let mut i = 0;
            while i < next_chars.len() && next_chars[i].is_whitespace() {
                i += 1;
            }
            self.cursor.1 = i;
        }
    }

    /// 前の単語先頭へ (b)
    pub fn move_word_back(&mut self) {
        let (row, col) = self.cursor;

        if col > 0 {
            let chars: Vec<char> = self.lines[row].chars().collect();
            let mut i = col - 1;
            // 空白スキップ
            while i > 0 && chars[i].is_whitespace() {
                i -= 1;
            }
            // 同種の文字をスキップ
            let is_word = chars[i].is_alphanumeric() || chars[i] == '_';
            while i > 0 && ((chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') == is_word) {
                i -= 1;
            }
            self.cursor.1 = i;
        } else if row > 0 {
            self.cursor.0 = row - 1;
            self.cursor.1 = char_count(&self.lines[self.cursor.0]);
        }
    }

    /// 単語末尾へ (e)
    pub fn move_word_end(&mut self) {
        let (row, col) = self.cursor;
        let line = &self.lines[row];
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();

        if col + 1 < len {
            let mut i = col + 1;
            // 空白スキップ
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            // 同種の文字の末尾へ
            if i < len {
                let is_word = chars[i].is_alphanumeric() || chars[i] == '_';
                while i + 1 < len
                    && ((chars[i + 1].is_alphanumeric() || chars[i + 1] == '_') == is_word)
                {
                    i += 1;
                }
                self.cursor.1 = i;
                return;
            }
        }
        // 次の行
        if row + 1 < self.lines.len() {
            self.cursor.0 = row + 1;
            let next_chars: Vec<char> = self.lines[self.cursor.0].chars().collect();
            let mut i = 0;
            while i < next_chars.len() && next_chars[i].is_whitespace() {
                i += 1;
            }
            if !next_chars.is_empty() {
                let is_word = next_chars.get(i).is_some_and(|c| c.is_alphanumeric() || *c == '_');
                while i + 1 < next_chars.len()
                    && ((next_chars[i + 1].is_alphanumeric() || next_chars[i + 1] == '_') == is_word)
                {
                    i += 1;
                }
            }
            self.cursor.1 = i;
        }
    }

    /// 行の最初の非空白文字へ (^)
    pub fn move_first_non_blank(&mut self) {
        let chars: Vec<char> = self.lines[self.cursor.0].chars().collect();
        let mut i = 0;
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        self.cursor.1 = i;
    }

    /// ファイル先頭へ (gg)
    pub fn move_to_top(&mut self) {
        self.cursor = (0, 0);
    }

    /// ファイル末尾へ (G)
    pub fn move_to_bottom(&mut self) {
        self.cursor.0 = self.lines.len().saturating_sub(1);
        self.cursor.1 = 0;
    }

    /// 縦ページダウン: cursor 行を `page_size` 単位下げ、列を行幅にクランプ
    pub fn move_page_down(&mut self, page_size: usize) {
        self.cursor.0 = (self.cursor.0 + page_size).min(self.lines.len().saturating_sub(1));
        let line_chars = char_count(&self.lines[self.cursor.0]);
        self.cursor.1 = self.cursor.1.min(line_chars);
    }

    /// 縦ページアップ: cursor 行を `page_size` 単位上げ、列を行幅にクランプ
    pub fn move_page_up(&mut self, page_size: usize) {
        self.cursor.0 = self.cursor.0.saturating_sub(page_size);
        let line_chars = char_count(&self.lines[self.cursor.0]);
        self.cursor.1 = self.cursor.1.min(line_chars);
    }

    /// 横ページ左: cursor 列を 40 単位戻す
    pub fn move_h_page_left(&mut self) {
        self.cursor.1 = self.cursor.1.saturating_sub(40);
    }

    /// 横ページ右: cursor 列を 40 単位進める（行幅にクランプ）
    pub fn move_h_page_right(&mut self) {
        let line_chars = char_count(&self.lines[self.cursor.0]);
        self.cursor.1 = (self.cursor.1 + 40).min(line_chars);
    }

    /// カーソル下の1文字を削除 (x)
    pub fn delete_char_at_cursor(&mut self) {
        self.delete();
    }

    /// 行を削除 (dd)
    pub fn delete_line(&mut self) {
        self.save_snapshot();
        if self.lines.len() > 1 {
            self.lines.remove(self.cursor.0);
            if self.cursor.0 >= self.lines.len() {
                self.cursor.0 = self.lines.len() - 1;
            }
        } else {
            self.lines[0].clear();
            self.cursor.1 = 0;
        }
        let line_chars = char_count(&self.lines[self.cursor.0]);
        self.cursor.1 = self.cursor.1.min(line_chars.saturating_sub(1).max(0));
    }

    /// カーソルから行末まで削除 (D)
    pub fn delete_to_end(&mut self) {
        self.save_snapshot();
        let byte_idx = char_to_byte_idx(&self.lines[self.cursor.0], self.cursor.1);
        self.lines[self.cursor.0].truncate(byte_idx);
        let line_chars = char_count(&self.lines[self.cursor.0]);
        if self.cursor.1 > 0 && self.cursor.1 >= line_chars {
            self.cursor.1 = line_chars.saturating_sub(1);
        }
    }

    /// カーソルから行末まで削除して Insert モード (C)
    pub fn change_to_end(&mut self) {
        self.save_snapshot();
        let byte_idx = char_to_byte_idx(&self.lines[self.cursor.0], self.cursor.1);
        self.lines[self.cursor.0].truncate(byte_idx);
        self.mode = EditorMode::Insert;
    }

    // ── オートコンプリート ──

    /// カーソル位置の入力中の単語 prefix を取得
    pub fn get_word_prefix(&self) -> String {
        let (row, col) = self.cursor;
        if row >= self.lines.len() {
            return String::new();
        }
        let line = &self.lines[row];
        let chars: Vec<char> = line.chars().collect();
        let col = col.min(chars.len());
        // カーソルから左に辿って単語の先頭を探す
        let mut start = col;
        while start > 0 {
            let ch = chars[start - 1];
            if ch.is_alphanumeric() || ch == '_' || ch == '.' {
                start -= 1;
            } else {
                break;
            }
        }
        chars[start..col].iter().collect()
    }

    /// カーソルの前にある直前の SQL キーワードを取得（FROM, JOIN 等の判定用）
    pub fn get_preceding_keyword(&self) -> Option<String> {
        let full_text = self.lines.join("\n");
        let mut byte_offset = 0;
        for (i, line) in self.lines.iter().enumerate() {
            if i == self.cursor.0 {
                byte_offset += char_to_byte_idx(line, self.cursor.1.min(char_count(line)));
                break;
            }
            byte_offset += line.len() + 1;
        }
        let before = &full_text[..byte_offset];
        // 空白で区切って直前のトークンを探す
        before
            .split_whitespace()
            .rev()
            .find(|w| {
                let upper = w.to_uppercase();
                matches!(
                    upper.as_str(),
                    "FROM" | "JOIN" | "INTO" | "TABLE" | "UPDATE"
                )
            })
            .map(|w| w.to_uppercase())
    }

    /// 補完候補の確定: prefix を候補で置換
    pub fn accept_completion(&mut self) {
        if let Some(selected) = self.completion.selected().map(String::from) {
            let prefix_len = self.completion.prefix.chars().count();
            // prefix 分だけ左に戻って削除し、候補を挿入
            for _ in 0..prefix_len {
                self.backspace();
            }
            for ch in selected.chars() {
                // snapshot は最初の1回だけ
                let (row, col) = self.cursor;
                if row < self.lines.len() {
                    let line = &mut self.lines[row];
                    let byte_idx = char_to_byte_idx(line, col);
                    line.insert(byte_idx, ch);
                    self.cursor.1 += 1;
                }
            }
            self.completion.close();
        }
    }

    /// サジェスト候補を更新
    pub fn update_completion(&mut self, table_names: &[String], table_columns: &[(String, Vec<String>)]) {
        if self.mode != EditorMode::Insert {
            self.completion.close();
            return;
        }

        let prefix = self.get_word_prefix();
        if prefix.is_empty() {
            self.completion.close();
            return;
        }

        // テーブル.カラム のパターン
        if prefix.contains('.') {
            let parts: Vec<&str> = prefix.splitn(2, '.').collect();
            let table = parts[0];
            let col_prefix = parts.get(1).copied().unwrap_or("");
            let col_prefix_lower = col_prefix.to_lowercase();

            if let Some((_, columns)) = table_columns
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(table))
            {
                let candidates: Vec<String> = columns
                    .iter()
                    .filter(|c| c.to_lowercase().starts_with(&col_prefix_lower))
                    .take(8)
                    .map(|c| format!("{}.{}", table, c))
                    .collect();

                if candidates.is_empty() {
                    self.completion.close();
                } else {
                    self.completion.prefix = prefix;
                    self.completion.candidates = candidates;
                    self.completion.cursor = 0;
                    self.completion.active = true;
                }
            } else {
                self.completion.close();
            }
            return;
        }

        let prefix_lower = prefix.to_lowercase();

        // 2文字未満はキーワード補完しない（ノイズ防止）
        if prefix.len() < 2 {
            self.completion.close();
            return;
        }

        let mut candidates: Vec<String> = Vec::new();

        // テーブル名（FROM/JOIN/INTO/TABLE/UPDATE の後）
        let preceding = self.get_preceding_keyword();
        let context_is_table = matches!(
            preceding.as_deref(),
            Some("FROM") | Some("JOIN") | Some("INTO") | Some("TABLE") | Some("UPDATE")
        );

        if context_is_table {
            candidates.extend(
                table_names
                    .iter()
                    .filter(|t| t.to_lowercase().starts_with(&prefix_lower))
                    .take(8)
                    .cloned(),
            );
        }

        // SQL キーワード
        if candidates.len() < 8 {
            let remaining = 8 - candidates.len();
            candidates.extend(
                SQL_KEYWORDS
                    .iter()
                    .filter(|kw| kw.to_lowercase().starts_with(&prefix_lower))
                    .take(remaining)
                    .map(|kw| kw.to_string()),
            );
        }

        // テーブル名もキーワード以外のコンテキストで補完（優先度低め）
        if !context_is_table && candidates.len() < 8 {
            let remaining = 8 - candidates.len();
            let extra: Vec<String> = table_names
                .iter()
                .filter(|t| {
                    t.to_lowercase().starts_with(&prefix_lower)
                        && !candidates.iter().any(|c| c == *t)
                })
                .take(remaining)
                .cloned()
                .collect();
            candidates.extend(extra);
        }

        if candidates.is_empty() {
            self.completion.close();
        } else {
            self.completion.prefix = prefix;
            self.completion.candidates = candidates;
            self.completion.cursor = 0;
            self.completion.active = true;
        }
    }

    /// カーソル位置のクエリを抽出（セミコロン区切り）
    pub fn get_query_at_cursor(&self) -> Option<String> {
        let full_text = self.lines.join("\n");
        if full_text.trim().is_empty() {
            return None;
        }

        // カーソル位置をテキスト全体でのバイトオフセットに変換
        let mut byte_offset = 0;
        for (i, line) in self.lines.iter().enumerate() {
            if i == self.cursor.0 {
                byte_offset += char_to_byte_idx(line, self.cursor.1.min(char_count(line)));
                break;
            }
            byte_offset += line.len() + 1; // +1 for \n
        }

        // セミコロンで分割し、カーソルがどのセグメントにいるか判定
        let mut start = 0;
        let mut last_query: Option<String> = None;
        for (i, _) in full_text.match_indices(';') {
            let query = full_text[start..i].trim();
            if !query.is_empty() {
                last_query = Some(query.to_string());
            }
            if byte_offset <= i {
                // カーソルがこのセミコロン以前にある → このセグメントを返す
                return last_query;
            }
            // カーソルがセミコロンの直後にある場合も、直前のクエリを記憶しておく
            start = i + 1;
        }

        // 最後のセグメント（セミコロンの後 or セミコロンなし）
        let query = full_text[start..].trim();
        if !query.is_empty() {
            Some(query.to_string())
        } else {
            // セミコロン直後で後続が空 → 直前のクエリを返す
            last_query
        }
    }

    /// バッファ全体を SQL フォーマッタで整形する。
    /// 中身が空、または整形結果が変わらない場合は何もしない。
    pub fn format_buffer(&mut self) -> bool {
        let original = self.lines.join("\n");
        if original.trim().is_empty() {
            return false;
        }
        let opts = sqlformat::FormatOptions {
            indent: sqlformat::Indent::Spaces(2),
            uppercase: Some(true),
            ..sqlformat::FormatOptions::default()
        };
        let formatted = sqlformat::format(&original, &sqlformat::QueryParams::None, &opts);
        if formatted == original {
            return false;
        }
        self.save_snapshot();
        self.lines = formatted.lines().map(String::from).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        // カーソルを安全な位置にクランプ
        if self.cursor.0 >= self.lines.len() {
            self.cursor.0 = self.lines.len() - 1;
        }
        let line_chars = char_count(&self.lines[self.cursor.0]);
        if self.cursor.1 > line_chars {
            self.cursor.1 = line_chars;
        }
        self.h_scroll_offset = 0;
        true
    }

    /// スクロールオフセットを調整してカーソルを表示範囲内に保つ
    pub fn adjust_scroll(&mut self, visible_height: usize, visible_width: usize) {
        // 縦スクロール
        if visible_height > 0 {
            if self.cursor.0 < self.scroll_offset {
                self.scroll_offset = self.cursor.0;
            } else if self.cursor.0 >= self.scroll_offset + visible_height {
                self.scroll_offset = self.cursor.0 - visible_height + 1;
            }
        }
        // 横スクロール
        if visible_width > 0 {
            if self.cursor.1 < self.h_scroll_offset {
                self.h_scroll_offset = self.cursor.1;
            } else if self.cursor.1 >= self.h_scroll_offset + visible_width {
                self.h_scroll_offset = self.cursor.1 + 1 - visible_width;
            }
        } else {
            self.h_scroll_offset = 0;
        }
    }
}

impl Scrollable for EditorState {
    fn move_one_down(&mut self) {
        self.move_down();
    }

    fn move_one_up(&mut self) {
        self.move_up();
    }

    fn move_one_left(&mut self) {
        self.move_left();
    }

    fn move_one_right(&mut self) {
        self.move_right();
    }

    fn scroll_to_top(&mut self) {
        self.move_to_top();
    }

    fn scroll_to_bottom(&mut self) {
        self.move_to_bottom();
    }

    fn h_scroll_home(&mut self) {
        self.move_home();
    }

    fn h_scroll_end(&mut self) {
        self.move_end();
    }

    fn page_down(&mut self, page_size: usize) {
        self.move_page_down(page_size);
    }

    fn page_up(&mut self, page_size: usize) {
        self.move_page_up(page_size);
    }

    fn h_page_left(&mut self) {
        self.move_h_page_left();
    }

    fn h_page_right(&mut self) {
        self.move_h_page_right();
    }
}

// ── ヘルパー ──

fn char_count(s: &str) -> usize {
    s.chars().count()
}

fn char_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

// ── 描画 ──

pub fn render(f: &mut Frame, editor: &EditorState, is_focused: bool, area: Rect) {
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if editor.executing {
        " Query Editor [実行中...] "
    } else {
        " Query Editor "
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 4 || inner.height < 1 {
        return;
    }

    let line_num_width = format!("{}", editor.lines.len()).len().max(2);
    let editor_width = (inner.width as usize).saturating_sub(line_num_width + 1); // 1 for "│"

    let visible_height = inner.height as usize;

    // 表示する行
    let start = editor.scroll_offset;
    let end = (start + visible_height).min(editor.lines.len());

    let mut display_lines: Vec<Line<'static>> = Vec::new();

    for i in start..end {
        let line = &editor.lines[i];
        let line_num = format!("{:>width$}", i + 1, width = line_num_width);

        let mut spans: Vec<Span<'static>> = vec![
            Span::styled(line_num, Style::default().fg(Color::DarkGray)),
            Span::styled("│".to_string(), Style::default().fg(Color::DarkGray)),
        ];

        // h_scroll を考慮した可視部分
        let visible_str: String = line
            .chars()
            .skip(editor.h_scroll_offset)
            .take(editor_width)
            .collect();

        // シンタックスハイライト（owned Span に変換）
        for s in highlight_sql(&visible_str) {
            spans.push(Span::styled(s.content.into_owned(), s.style));
        }

        display_lines.push(Line::from(spans));
    }

    // 空行で埋める
    for _i in end..start + visible_height {
        let line_num = " ".repeat(line_num_width);
        display_lines.push(Line::from(vec![
            Span::styled(line_num, Style::default().fg(Color::DarkGray)),
            Span::styled("│".to_string(), Style::default().fg(Color::DarkGray)),
            Span::styled("~".to_string(), Style::default().fg(Color::DarkGray)),
        ]));
    }

    let paragraph = Paragraph::new(display_lines);
    f.render_widget(paragraph, inner);

    // カーソル表示（Normal / Insert 両モード）
    if is_focused && editor.cursor.1 >= editor.h_scroll_offset {
        let cursor_x = inner.x
            + line_num_width as u16
            + 1
            + (editor.cursor.1 - editor.h_scroll_offset) as u16;
        let cursor_y = inner.y + editor.cursor.0.saturating_sub(editor.scroll_offset) as u16;
        if cursor_x < inner.x + inner.width && cursor_y < inner.y + inner.height {
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }

    // オートコンプリート プルダウン
    if is_focused
        && editor.completion.active
        && !editor.completion.candidates.is_empty()
        && editor.cursor.1 >= editor.h_scroll_offset
    {
        let cursor_x = inner.x
            + line_num_width as u16
            + 1
            + (editor.cursor.1 - editor.h_scroll_offset) as u16;
        let cursor_y = inner.y + editor.cursor.0.saturating_sub(editor.scroll_offset) as u16;

        let max_items = editor.completion.candidates.len().min(8);
        let popup_width = editor
            .completion
            .candidates
            .iter()
            .map(|c| c.len())
            .max()
            .unwrap_or(10)
            .max(10) as u16
            + 4;

        // prefix 分だけ左にオフセット
        let prefix_len = editor.completion.prefix.len() as u16;
        let popup_x = cursor_x.saturating_sub(prefix_len);
        let popup_y = cursor_y + 1;
        let popup_height = max_items as u16 + 2; // ボーダー分

        // 画面内に収まるか確認
        let frame_area = f.area();
        if popup_y + popup_height <= frame_area.height
            && popup_x + popup_width <= frame_area.width
        {
            let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);
            f.render_widget(Clear, popup_area);

            let items: Vec<ListItem> = editor
                .completion
                .candidates
                .iter()
                .enumerate()
                .take(max_items)
                .map(|(i, candidate)| {
                    let style = if i == editor.completion.cursor {
                        Style::default().bg(Color::Cyan).fg(Color::Black)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    ListItem::new(Span::styled(format!(" {} ", candidate), style))
                })
                .collect();

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray));
            let list = List::new(items).block(block);
            f.render_widget(list, popup_area);
        }
    }
}

/// SQL シンタックスハイライト（簡易版）
fn highlight_sql(line: &str) -> Vec<Span<'_>> {
    let mut spans = Vec::new();
    let mut chars = line.char_indices().peekable();
    let mut current_start = 0;

    while let Some(&(i, ch)) = chars.peek() {
        if ch == '-' {
            // コメント "--"
            let rest = &line[i..];
            if rest.starts_with("--") {
                if i > current_start {
                    spans.push(Span::raw(&line[current_start..i]));
                }
                spans.push(Span::styled(&line[i..], Style::default().fg(Color::DarkGray)));
                return spans;
            }
            chars.next();
        } else if ch == '\'' {
            // 文字列リテラル
            if i > current_start {
                spans.push(Span::raw(&line[current_start..i]));
            }
            chars.next();
            let str_start = i;
            while let Some(&(_j, c)) = chars.peek() {
                chars.next();
                if c == '\'' {
                    break;
                }
            }
            let end = chars.peek().map(|&(j, _)| j).unwrap_or(line.len());
            let str_end = end.min(line.len());
            spans.push(Span::styled(
                &line[str_start..str_end],
                Style::default().fg(Color::Green),
            ));
            current_start = str_end;
        } else if ch.is_alphabetic() || ch == '_' {
            // ワード抽出
            let word_start = i;
            while let Some(&(_, c)) = chars.peek() {
                if c.is_alphanumeric() || c == '_' {
                    chars.next();
                } else {
                    break;
                }
            }
            let word_end = chars.peek().map(|&(j, _)| j).unwrap_or(line.len());
            let word = &line[word_start..word_end];

            if i > current_start {
                spans.push(Span::raw(&line[current_start..i]));
            }

            if SQL_KEYWORDS.iter().any(|kw| kw.eq_ignore_ascii_case(word)) {
                spans.push(Span::styled(
                    word,
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::raw(word));
            }
            current_start = word_end;
        } else if ch.is_ascii_digit() {
            // 数値
            let num_start = i;
            while let Some(&(_, c)) = chars.peek() {
                if c.is_ascii_digit() || c == '.' {
                    chars.next();
                } else {
                    break;
                }
            }
            let num_end = chars.peek().map(|&(j, _)| j).unwrap_or(line.len());

            if i > current_start {
                spans.push(Span::raw(&line[current_start..i]));
            }
            spans.push(Span::styled(
                &line[num_start..num_end],
                Style::default().fg(Color::Yellow),
            ));
            current_start = num_end;
        } else {
            chars.next();
        }
    }

    if current_start < line.len() {
        spans.push(Span::raw(&line[current_start..]));
    }

    if spans.is_empty() {
        spans.push(Span::raw(""));
    }

    spans
}

#[cfg(test)]
mod tests {
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
}
