// オペレータ・ヤンク・ペースト・置換系の編集操作

use super::{char_count, char_to_byte_idx, EditorMode, EditorState};

/// ヤンク内容の種類（ペースト時の貼り方が変わる）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum YankKind {
    Char,
    Line,
}

#[derive(Debug, Clone)]
pub struct Register {
    pub(crate) text: String,
    pub(crate) kind: YankKind,
}

impl EditorState {
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
            for c_ref in &mut new_chars[s..e] {
                let c = *c_ref;
                *c_ref = if c.is_uppercase() {
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

    /// `(row, col_start..=col_end)` 範囲のテキストを文字単位レジスタに保存し、
    /// `delete` が true なら save_snapshot を取って削除し、カーソルを col_start に置く（行末でクランプ）。
    /// `enter_insert` が true なら最後に Insert モードへ遷移する。
    /// dw / diw / cw / ciw / yw / yiw の共通実装。
    pub(super) fn yank_or_delete_range(
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

    // ── 行末まで系 ──

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
}
