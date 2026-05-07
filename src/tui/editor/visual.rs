// Visual / VisualLine モードの選択範囲操作

use super::{char_count, char_to_byte_idx, EditorMode, EditorState, PendingChord, YankKind};

impl EditorState {
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
    /// charwise は両端を含む文字インデックス、linewise は (start.0, 0) と (end.0, end_line_char_count) のように行末まで。
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
}
