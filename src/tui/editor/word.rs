// 単語境界の判定と単語単位の移動操作

use super::{char_count, EditorState};

/// vim 互換の単語文字判定（英数字 + アンダースコア）
#[inline]
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl EditorState {
    /// 指定位置の「inner word」(iw) 範囲を返す。
    /// (col_start, col_end_inclusive) を返す。空行や非単語上では (col, col) を返す。
    pub fn inner_word_range_at(&self, row: usize, col: usize) -> Option<(usize, usize)> {
        if row >= self.lines.len() {
            return None;
        }
        let chars: Vec<char> = self.lines[row].chars().collect();
        if chars.is_empty() {
            return None;
        }
        let col = col.min(chars.len().saturating_sub(1));
        let cur = chars[col];
        if !is_word_char(cur) {
            // 非単語文字の連続
            let mut s = col;
            while s > 0 && !is_word_char(chars[s - 1]) && !chars[s - 1].is_whitespace() {
                s -= 1;
            }
            let mut e = col;
            while e + 1 < chars.len() && !is_word_char(chars[e + 1]) && !chars[e + 1].is_whitespace() {
                e += 1;
            }
            // 空白上の場合は単に col のみ
            if cur.is_whitespace() {
                let mut s = col;
                while s > 0 && chars[s - 1].is_whitespace() {
                    s -= 1;
                }
                let mut e = col;
                while e + 1 < chars.len() && chars[e + 1].is_whitespace() {
                    e += 1;
                }
                return Some((s, e));
            }
            return Some((s, e));
        }
        // 単語の前方・後方を伸ばす
        let mut s = col;
        while s > 0 && is_word_char(chars[s - 1]) {
            s -= 1;
        }
        let mut e = col;
        while e + 1 < chars.len() && is_word_char(chars[e + 1]) {
            e += 1;
        }
        Some((s, e))
    }

    /// dw / cw / yw 用: カーソル位置から「次の単語の先頭の手前まで」の inclusive な終了列を返す。
    /// 行末を超える場合は行末を返す。
    pub fn forward_word_end_col_at(&self, row: usize, col: usize) -> Option<usize> {
        if row >= self.lines.len() {
            return None;
        }
        let chars: Vec<char> = self.lines[row].chars().collect();
        if chars.is_empty() || col >= chars.len() {
            return None;
        }
        let cur = chars[col];
        let mut i = col;
        if cur.is_whitespace() {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
        } else if is_word_char(cur) {
            while i < chars.len() && is_word_char(chars[i]) {
                i += 1;
            }
            // dw は次単語先頭手前までだが、空白も含めるのが vim の挙動（行内の場合）
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
        } else {
            // 非単語文字: 同種を進む
            while i < chars.len() && !is_word_char(chars[i]) && !chars[i].is_whitespace() {
                i += 1;
            }
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
        }
        // i は次単語の先頭。inclusive 終端は i.saturating_sub(1)
        Some(i.saturating_sub(1).max(col))
    }

    /// cw / ce 用: 末尾空白を含めない単語末尾 inclusive 列を返す（vim の cw = ce 相当の挙動）。
    pub fn change_word_end_col_at(&self, row: usize, col: usize) -> Option<usize> {
        if row >= self.lines.len() { return None; }
        let chars: Vec<char> = self.lines[row].chars().collect();
        if chars.is_empty() || col >= chars.len() { return None; }
        let cur = chars[col];
        let mut i = col;
        if is_word_char(cur) {
            while i < chars.len() && is_word_char(chars[i]) { i += 1; }
        } else if !cur.is_whitespace() {
            while i < chars.len() && !is_word_char(chars[i]) && !chars[i].is_whitespace() { i += 1; }
        } else {
            while i < chars.len() && chars[i].is_whitespace() { i += 1; }
        }
        Some(i.saturating_sub(1).max(col))
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
            let is_word = is_word_char(chars[i]);
            while i < len && (is_word_char(chars[i]) == is_word) {
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
            let is_word = is_word_char(chars[i]);
            while i > 0 && (is_word_char(chars[i - 1]) == is_word) {
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
                let is_word = is_word_char(chars[i]);
                while i + 1 < len && (is_word_char(chars[i + 1]) == is_word) {
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
                let is_word = next_chars.get(i).is_some_and(|c| is_word_char(*c));
                while i + 1 < next_chars.len() && (is_word_char(next_chars[i + 1]) == is_word) {
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
}
