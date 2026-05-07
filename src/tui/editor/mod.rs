use crate::tui::scrollable::Scrollable;

mod render;
mod word;

pub use render::render;

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

/// ヤンク内容の種類（ペースト時の貼り方が変わる）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum YankKind {
    Char,
    Line,
}

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

#[derive(Debug, Clone)]
pub struct Register {
    pub(crate) text: String,
    pub(crate) kind: YankKind,
}

#[derive(Debug, Clone)]
pub struct Search {
    /// 検索バーで入力中
    pub(crate) active: bool,
    /// 確定済みの検索クエリ
    pub(crate) query: String,
    /// マッチ箇所 (行, 列開始, 長さ)
    pub(crate) matches: Vec<(usize, usize, usize)>,
    /// 現在の matches インデックス
    pub(crate) current: usize,
}

impl Search {
    pub fn new() -> Self {
        Self {
            active: false,
            query: String::new(),
            matches: Vec::new(),
            current: 0,
        }
    }

    pub fn clear(&mut self) {
        self.active = false;
        self.query.clear();
        self.matches.clear();
        self.current = 0;
    }
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
    /// Visual モード開始位置 (行, 列)
    pub visual_anchor: Option<(usize, usize)>,
    /// Normal モード中のチョード待機状態（r/gg/d系/y系/c系）
    pub pending_chord: PendingChord,
    /// 内部レジスタ（y / d で更新）
    pub register: Option<Register>,
    /// 検索状態
    pub search: Search,
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
            visual_anchor: None,
            pending_chord: PendingChord::None,
            register: None,
            search: Search::new(),
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
        let was_insert = self.mode == EditorMode::Insert;
        self.mode = EditorMode::Normal;
        self.visual_anchor = None;
        self.pending_chord = PendingChord::None;
        // vim の挙動: Insert → Normal でカーソルが1つ左に戻る
        if was_insert {
            let line_chars = char_count(&self.lines[self.cursor.0]);
            if self.cursor.1 > 0 && self.cursor.1 >= line_chars && line_chars > 0 {
                self.cursor.1 = line_chars - 1;
            }
        }
    }

    // ── Visual モード ──

    /// Visual (charwise) を開始
    pub fn enter_visual(&mut self) {
        self.mode = EditorMode::Visual;
        self.visual_anchor = Some(self.cursor);
        self.pending_chord = PendingChord::None;
    }

    /// VisualLine を開始
    pub fn enter_visual_line(&mut self) {
        self.mode = EditorMode::VisualLine;
        self.visual_anchor = Some(self.cursor);
        self.pending_chord = PendingChord::None;
    }

    /// Visual 中、anchor とカーソルを入れ替える (o)
    pub fn swap_visual_anchor(&mut self) {
        if let Some(anchor) = self.visual_anchor {
            self.visual_anchor = Some(self.cursor);
            self.cursor = anchor;
        }
    }

    /// 選択範囲を正規化された (start, end) で返す。
    /// charwise は両端を含む文字インデックス、linewise は (start.0, _) と (end.0, end_line_char_count) のように行末まで。
    pub fn selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        let anchor = self.visual_anchor?;
        let cursor = self.cursor;
        let (start, end) = if anchor <= cursor {
            (anchor, cursor)
        } else {
            (cursor, anchor)
        };
        match self.mode {
            EditorMode::Visual => Some((start, end)),
            EditorMode::VisualLine => {
                let s = (start.0, 0);
                let last_col = char_count(&self.lines[end.0]);
                let e = (end.0, last_col);
                Some((s, e))
            }
            _ => None,
        }
    }

    /// 選択範囲のテキストを返す。VisualLine の場合は末尾改行付き。
    pub fn selection_text(&self) -> Option<(String, YankKind)> {
        let ((sr, sc), (er, ec)) = self.selection_range()?;
        let kind = match self.mode {
            EditorMode::VisualLine => YankKind::Line,
            _ => YankKind::Char,
        };
        let text = if sr == er {
            let line = &self.lines[sr];
            let bs = char_to_byte_idx(line, sc);
            // charwise の end は inclusive、linewise はそのまま行末
            let be = match self.mode {
                EditorMode::Visual => char_to_byte_idx(line, (ec + 1).min(char_count(line))),
                _ => char_to_byte_idx(line, ec),
            };
            let mut s = line[bs..be].to_string();
            if matches!(self.mode, EditorMode::VisualLine) {
                s.push('\n');
            }
            s
        } else {
            let mut buf = String::new();
            // 先頭行
            let first_line = &self.lines[sr];
            let bs = char_to_byte_idx(first_line, sc);
            buf.push_str(&first_line[bs..]);
            buf.push('\n');
            // 中間行
            for r in (sr + 1)..er {
                buf.push_str(&self.lines[r]);
                buf.push('\n');
            }
            // 終端行
            let last_line = &self.lines[er];
            let be = match self.mode {
                EditorMode::Visual => char_to_byte_idx(last_line, (ec + 1).min(char_count(last_line))),
                _ => char_to_byte_idx(last_line, ec),
            };
            buf.push_str(&last_line[..be]);
            if matches!(self.mode, EditorMode::VisualLine) {
                buf.push('\n');
            }
            buf
        };
        Some((text, kind))
    }

    /// 選択範囲を削除し、削除した内容（YankKind 付き）を返す。
    /// Normal モードに戻る。
    pub fn delete_selection(&mut self) -> Option<(String, YankKind)> {
        let captured = self.selection_text()?;
        let ((sr, sc), (er, ec)) = self.selection_range()?;
        self.save_snapshot();

        match self.mode {
            EditorMode::VisualLine => {
                // 行ごと削除
                let drain_start = sr;
                let drain_end = er + 1;
                self.lines.drain(drain_start..drain_end);
                if self.lines.is_empty() {
                    self.lines.push(String::new());
                }
                let new_row = sr.min(self.lines.len() - 1);
                self.cursor = (new_row, 0);
            }
            EditorMode::Visual => {
                if sr == er {
                    let line = &mut self.lines[sr];
                    let bs = char_to_byte_idx(line, sc);
                    let be = char_to_byte_idx(line, (ec + 1).min(char_count(line)));
                    line.replace_range(bs..be, "");
                    self.cursor = (sr, sc);
                } else {
                    let first_byte = char_to_byte_idx(&self.lines[sr], sc);
                    let mut head = self.lines[sr][..first_byte].to_string();
                    let last_line = &self.lines[er];
                    let last_byte = char_to_byte_idx(last_line, (ec + 1).min(char_count(last_line)));
                    let tail = last_line[last_byte..].to_string();
                    head.push_str(&tail);
                    self.lines[sr] = head;
                    // sr+1..=er を削除
                    self.lines.drain(sr + 1..=er);
                    self.cursor = (sr, sc);
                }
                // カーソル列を行末でクランプ
                let line_chars = char_count(&self.lines[self.cursor.0]);
                if self.cursor.1 > 0 && self.cursor.1 >= line_chars {
                    self.cursor.1 = line_chars.saturating_sub(1);
                }
            }
            _ => return None,
        }
        self.mode = EditorMode::Normal;
        self.visual_anchor = None;
        Some(captured)
    }

    /// Visual / VisualLine 選択範囲の大小反転（~）。
    /// 行ごとに対象列を判定し、各 char を upper⇔lower 切り替え。
    /// VisualLine では行全体、Visual では (sr,sc)〜(er,ec) inclusive を対象。
    pub fn toggle_case_selection(&mut self) {
        let Some(((sr, sc), (er, ec))) = self.selection_range() else { return };
        let mode = self.mode;
        self.save_snapshot();
        for r in sr..=er {
            let chars: Vec<char> = self.lines[r].chars().collect();
            let s = if r == sr && mode == EditorMode::Visual { sc } else { 0 };
            let e = if r == er && mode == EditorMode::Visual {
                (ec + 1).min(chars.len())
            } else {
                chars.len()
            };
            let mut new_chars = chars.clone();
            for k in s..e {
                let c = new_chars[k];
                new_chars[k] = if c.is_uppercase() {
                    c.to_lowercase().next().unwrap_or(c)
                } else if c.is_lowercase() {
                    c.to_uppercase().next().unwrap_or(c)
                } else {
                    c
                };
            }
            self.lines[r] = new_chars.iter().collect();
        }
    }

    // ── 単語境界系は word.rs に分離 ──
    // inner_word_range_at / forward_word_end_col_at / change_word_end_col_at

    // ── オペレータ + モーション ──

    /// `(row, col_start..=col_end)` 範囲のテキストを文字単位レジスタに保存し、
    /// `delete` が true なら save_snapshot を取って削除し、カーソルを col_start に置く（行末でクランプ）。
    /// `enter_insert` が true なら最後に Insert モードへ遷移する。
    /// dw / diw / cw / ciw / yw / yiw の共通実装。
    fn yank_or_delete_range(
        &mut self,
        row: usize,
        col_start: usize,
        col_end: usize,
        delete: bool,
        enter_insert: bool,
    ) {
        let line = &self.lines[row];
        let bs = char_to_byte_idx(line, col_start);
        let be = char_to_byte_idx(line, (col_end + 1).min(char_count(line)));
        let yanked = line[bs..be].to_string();
        self.register = Some(Register { text: yanked, kind: YankKind::Char });
        if delete {
            self.save_snapshot();
            self.lines[row].replace_range(bs..be, "");
            self.cursor.1 = col_start;
            let line_chars = char_count(&self.lines[row]);
            if self.cursor.1 > 0 && self.cursor.1 >= line_chars {
                self.cursor.1 = line_chars.saturating_sub(1);
            }
        }
        if enter_insert {
            self.mode = EditorMode::Insert;
        }
    }

    // change_word_end_col_at は word.rs に分離

    /// 内部レジスタを差し替える。外部から register 直接代入する代わりに使う。
    pub fn set_register(&mut self, text: String, kind: YankKind) {
        self.register = Some(Register { text, kind });
    }

    /// dd: 行削除 + ヤンク（既存の delete_line をレジスタ対応に拡張）
    pub fn delete_line_yank(&mut self) {
        let row = self.cursor.0;
        let yanked = format!("{}\n", self.lines[row]);
        self.register = Some(Register { text: yanked, kind: YankKind::Line });
        self.delete_line();
    }

    /// dw: カーソルから次単語先頭の手前まで削除 + ヤンク
    pub fn delete_word_forward(&mut self) {
        let (row, col) = self.cursor;
        let Some(end) = self.forward_word_end_col_at(row, col) else { return };
        if end < col { return; }
        self.yank_or_delete_range(row, col, end, true, false);
    }

    /// diw: 単語まるごと削除 + ヤンク
    pub fn delete_inner_word(&mut self) {
        let (row, col) = self.cursor;
        let Some((s, e)) = self.inner_word_range_at(row, col) else { return };
        self.yank_or_delete_range(row, s, e, true, false);
    }

    /// cw: dw + Insert（末尾空白は含めない = ce 相当）
    pub fn change_word_forward(&mut self) {
        let (row, col) = self.cursor;
        let Some(end) = self.change_word_end_col_at(row, col) else {
            self.mode = EditorMode::Insert;
            return;
        };
        self.yank_or_delete_range(row, col, end, true, true);
    }

    /// ciw: diw + Insert
    pub fn change_inner_word(&mut self) {
        let (row, col) = self.cursor;
        let Some((s, e)) = self.inner_word_range_at(row, col) else {
            self.mode = EditorMode::Insert;
            return;
        };
        self.yank_or_delete_range(row, s, e, true, true);
    }

    /// yy: 行をヤンク
    pub fn yank_line(&mut self) {
        let row = self.cursor.0;
        let text = format!("{}\n", self.lines[row]);
        self.register = Some(Register { text, kind: YankKind::Line });
    }

    /// yw: 次単語先頭手前までヤンク
    pub fn yank_word_forward(&mut self) {
        let (row, col) = self.cursor;
        let Some(end) = self.forward_word_end_col_at(row, col) else { return };
        if end < col { return; }
        self.yank_or_delete_range(row, col, end, false, false);
    }

    /// yiw: 単語まるごとヤンク
    pub fn yank_inner_word(&mut self) {
        let (row, col) = self.cursor;
        let Some((s, e)) = self.inner_word_range_at(row, col) else { return };
        self.yank_or_delete_range(row, s, e, false, false);
    }

    /// p: カーソル後ろにペースト（Char）/ 下行にペースト（Line）
    pub fn paste_after(&mut self) {
        let Some(reg) = self.register.clone() else { return };
        self.save_snapshot();
        match reg.kind {
            YankKind::Line => {
                let mut text = reg.text;
                if text.ends_with('\n') {
                    text.pop();
                }
                let row = self.cursor.0;
                let new_lines: Vec<String> = text.split('\n').map(String::from).collect();
                let insert_at = row + 1;
                for (i, l) in new_lines.iter().enumerate() {
                    self.lines.insert(insert_at + i, l.clone());
                }
                self.cursor = (insert_at, 0);
            }
            YankKind::Char => {
                let (row, col) = self.cursor;
                let line_chars = char_count(&self.lines[row]);
                let insert_col = if line_chars == 0 { 0 } else { (col + 1).min(line_chars) };
                // 空 register を p するときは cursor を insert_col に揃える（旧来挙動）
                if reg.text.is_empty() {
                    self.cursor = (row, insert_col);
                }
                self.paste_chars_at(&reg.text, row, insert_col);
            }
        }
    }

    /// P: カーソル前にペースト（Char）/ 上行にペースト（Line）
    pub fn paste_before(&mut self) {
        let Some(reg) = self.register.clone() else { return };
        self.save_snapshot();
        match reg.kind {
            YankKind::Line => {
                let mut text = reg.text;
                if text.ends_with('\n') { text.pop(); }
                let row = self.cursor.0;
                let new_lines: Vec<String> = text.split('\n').map(String::from).collect();
                for (i, l) in new_lines.iter().enumerate() {
                    self.lines.insert(row + i, l.clone());
                }
                self.cursor = (row, 0);
            }
            YankKind::Char => {
                let (row, col) = self.cursor;
                self.paste_chars_at(&reg.text, row, col);
            }
        }
    }

    /// charwise paste の共通実装。
    /// `text` を `(row, insert_col)` から挿入し、カーソルを末尾に置く。
    /// 改行を含む場合は途中行を新規行として挿入し、最終行末-1 にカーソルを合わせる。
    fn paste_chars_at(&mut self, text: &str, row: usize, insert_col: usize) {
        let parts: Vec<&str> = text.split('\n').collect();
        if parts.len() == 1 {
            let bi = char_to_byte_idx(&self.lines[row], insert_col);
            self.lines[row].insert_str(bi, parts[0]);
            if !parts[0].is_empty() {
                self.cursor = (row, insert_col + parts[0].chars().count() - 1);
            }
        } else {
            let bi = char_to_byte_idx(&self.lines[row], insert_col);
            let tail = self.lines[row][bi..].to_string();
            self.lines[row].truncate(bi);
            self.lines[row].push_str(parts[0]);
            for (i, p) in parts[1..].iter().enumerate() {
                let line_text = if i == parts.len() - 2 {
                    let mut s = p.to_string();
                    s.push_str(&tail);
                    s
                } else {
                    p.to_string()
                };
                self.lines.insert(row + 1 + i, line_text);
            }
            let last_row = row + parts.len() - 1;
            let last_inserted_chars = parts.last().unwrap().chars().count();
            self.cursor = (last_row, last_inserted_chars.saturating_sub(1));
        }
    }

    // ── 置換系 ──

    /// r{ch}: カーソル位置の1文字を置換（モード遷移なし）
    pub fn replace_char(&mut self, ch: char) {
        let (row, col) = self.cursor;
        if row >= self.lines.len() { return; }
        let line_chars = char_count(&self.lines[row]);
        if col >= line_chars { return; }
        self.save_snapshot();
        let line = &mut self.lines[row];
        let bs = char_to_byte_idx(line, col);
        let be = char_to_byte_idx(line, col + 1);
        let mut buf = String::new();
        buf.push(ch);
        line.replace_range(bs..be, &buf);
    }

    /// s: 1文字削除して Insert
    pub fn substitute_char(&mut self) {
        let (row, col) = self.cursor;
        let line_chars = char_count(&self.lines[row]);
        if col < line_chars {
            self.save_snapshot();
            let line = &mut self.lines[row];
            let bs = char_to_byte_idx(line, col);
            let be = char_to_byte_idx(line, col + 1);
            line.replace_range(bs..be, "");
        }
        self.mode = EditorMode::Insert;
    }

    /// S: 行を空にして Insert（インデント保持はしない簡易版）
    pub fn substitute_line(&mut self) {
        self.save_snapshot();
        self.lines[self.cursor.0].clear();
        self.cursor.1 = 0;
        self.mode = EditorMode::Insert;
    }

    /// J: 次行を現在行末に結合（間に半角スペース）
    pub fn join_lines(&mut self) {
        if self.cursor.0 + 1 >= self.lines.len() { return; }
        self.save_snapshot();
        let row = self.cursor.0;
        let next = self.lines.remove(row + 1);
        let cur_chars = char_count(&self.lines[row]);
        let need_space = !self.lines[row].is_empty() && !next.trim_start().is_empty();
        let trimmed = next.trim_start();
        if need_space {
            self.lines[row].push(' ');
        }
        self.lines[row].push_str(trimmed);
        self.cursor = (row, cur_chars + if need_space { 1 } else { 0 });
    }

    /// ~: カーソル下の文字の大小を反転し、カーソルを1つ右へ
    pub fn toggle_case(&mut self) {
        let (row, col) = self.cursor;
        let line_chars = char_count(&self.lines[row]);
        if col >= line_chars { return; }
        self.save_snapshot();
        let chars: Vec<char> = self.lines[row].chars().collect();
        let ch = chars[col];
        let toggled: String = if ch.is_uppercase() {
            ch.to_lowercase().collect()
        } else if ch.is_lowercase() {
            ch.to_uppercase().collect()
        } else {
            ch.to_string()
        };
        let bs = char_to_byte_idx(&self.lines[row], col);
        let be = char_to_byte_idx(&self.lines[row], col + 1);
        self.lines[row].replace_range(bs..be, &toggled);
        if col + 1 < char_count(&self.lines[row]) {
            self.cursor.1 = col + 1;
        }
    }

    // ── 検索 ──

    pub fn search_start(&mut self) {
        self.search.active = true;
        self.search.query.clear();
        self.search.matches.clear();
        self.search.current = 0;
    }

    pub fn search_cancel(&mut self) {
        self.search.clear();
    }

    pub fn search_confirm(&mut self) {
        self.search.active = false;
        self.recompute_matches();
        self.jump_to_match(self.search.current);
    }

    pub fn search_push_char(&mut self, ch: char) {
        self.search.query.push(ch);
        self.recompute_matches();
    }

    pub fn search_pop_char(&mut self) {
        self.search.query.pop();
        self.recompute_matches();
    }

    /// 現在のクエリでマッチを再計算（小文字での部分一致）
    pub fn recompute_matches(&mut self) {
        self.search.matches.clear();
        self.search.current = 0;
        if self.search.query.is_empty() {
            return;
        }
        let needle = self.search.query.to_lowercase();
        let needle_chars = needle.chars().count();
        for (row, line) in self.lines.iter().enumerate() {
            let lower = line.to_lowercase();
            let mut from = 0usize;
            while let Some(rel) = lower[from..].find(&needle) {
                let byte_start = from + rel;
                // バイト位置を文字位置に変換
                let col_start = lower[..byte_start].chars().count();
                self.search.matches.push((row, col_start, needle_chars));
                from = byte_start + needle.len();
                if from > lower.len() { break; }
            }
        }
        // カーソル位置以降の最初のマッチに current を合わせる
        let cursor = self.cursor;
        if let Some((idx, _)) = self
            .search
            .matches
            .iter()
            .enumerate()
            .find(|(_, (r, c, _))| (*r, *c) >= cursor)
        {
            self.search.current = idx;
        }
    }

    pub fn next_match(&mut self) {
        if self.search.matches.is_empty() { return; }
        self.search.current = (self.search.current + 1) % self.search.matches.len();
        self.jump_to_match(self.search.current);
    }

    pub fn prev_match(&mut self) {
        if self.search.matches.is_empty() { return; }
        self.search.current = if self.search.current == 0 {
            self.search.matches.len() - 1
        } else {
            self.search.current - 1
        };
        self.jump_to_match(self.search.current);
    }

    fn jump_to_match(&mut self, idx: usize) {
        if let Some(&(row, col, _)) = self.search.matches.get(idx) {
            self.cursor = (row, col);
        }
    }

    // move_word_forward / move_word_back / move_word_end / move_first_non_blank は word.rs に分離

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

    fn center_on_cursor(&mut self, page_size: usize) {
        // カーソル行を画面中央に持ってくる: scroll_offset = cursor_row - page_size/2
        let half = page_size / 2;
        self.scroll_offset = self.cursor.0.saturating_sub(half);
    }
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
}
