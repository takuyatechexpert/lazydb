// EditorState の基本操作（カーソル移動・編集・undo/redo・補完・スクロール）と Scrollable trait 実装

use crate::tui::scrollable::Scrollable;

use super::search::Search;
use super::{
    char_count, char_to_byte_idx, Completion, EditorMode, EditorSnapshot, EditorState,
    PendingChord, SQL_KEYWORDS,
};

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

    pub(super) fn save_snapshot(&mut self) {
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

    /// 複数行を含む文字列を一括挿入する（ペースト用）。
    /// `\r\n` / `\r` / `\n` を改行として扱い、それ以外は通常文字として挿入する。
    /// snapshot は冒頭で 1 回だけ取り、undo を 1 アクションにまとめる。
    pub fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.save_snapshot();
        let normalized = s.replace("\r\n", "\n").replace('\r', "\n");
        let mut first = true;
        for segment in normalized.split('\n') {
            if !first {
                let (row, col) = self.cursor;
                if row < self.lines.len() {
                    let byte_idx = char_to_byte_idx(&self.lines[row], col);
                    let rest = self.lines[row][byte_idx..].to_string();
                    self.lines[row].truncate(byte_idx);
                    self.lines.insert(row + 1, rest);
                    self.cursor = (row + 1, 0);
                }
            }
            for ch in segment.chars() {
                let (row, col) = self.cursor;
                if row < self.lines.len() {
                    let line = &mut self.lines[row];
                    let byte_idx = char_to_byte_idx(line, col);
                    line.insert(byte_idx, ch);
                    self.cursor.1 += 1;
                }
            }
            first = false;
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
        self.cursor.1 = self.cursor.1.min(line_chars.saturating_sub(1));
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
        let formatted = insert_blank_lines_between_queries(&formatted);
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

/// 複数クエリを見やすくするため、`;` で終わる行の直後に空行を挿入する。
/// - 既に空行が続く場合は何もしない
/// - 末尾の `;` の後ろには追加しない
pub(crate) fn insert_blank_lines_between_queries(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        out.push((*line).to_string());
        if !line.trim_end().ends_with(';') {
            continue;
        }
        // 後続に非空行が存在するか確認（末尾の`;`の後ろには追加しない）
        let mut has_following_content = false;
        let mut next_is_blank = false;
        if let Some(next) = lines.get(i + 1) {
            next_is_blank = next.trim().is_empty();
            for l in &lines[i + 1..] {
                if !l.trim().is_empty() {
                    has_following_content = true;
                    break;
                }
            }
        }
        if has_following_content && !next_is_blank {
            out.push(String::new());
        }
    }
    out.join("\n")
}
