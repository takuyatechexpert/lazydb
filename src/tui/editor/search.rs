// インクリメンタル検索（小文字での部分一致）

use super::EditorState;

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

impl EditorState {
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

    pub(super) fn jump_to_match(&mut self, idx: usize) {
        if let Some(&(row, col, _)) = self.search.matches.get(idx) {
            self.cursor = (row, col);
        }
    }
}
