use crate::db::adapter::QueryResult;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

// App / Panel は不要: render は ResultsState を直接受け取る

// ── 状態 ──

#[derive(Debug, Clone)]
pub enum ResultStatus {
    Empty,
    Success,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct ResultsState {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub col_widths: Vec<usize>,
    pub scroll_offset: usize,
    pub h_scroll: usize,
    pub status: ResultStatus,
    pub duration_ms: u64,
    pub total_rows: usize,
    pub auto_limited: bool,
    pub result: Option<QueryResult>,
    pub visible_width: usize,
}

impl ResultsState {
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            col_widths: Vec::new(),
            scroll_offset: 0,
            h_scroll: 0,
            status: ResultStatus::Empty,
            duration_ms: 0,
            total_rows: 0,
            auto_limited: false,
            result: None,
            visible_width: 0,
        }
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }

    pub fn set_result(&mut self, result: QueryResult, auto_limited: bool) {
        self.total_rows = result.rows.len();
        self.duration_ms = result.duration_ms;
        self.columns = result.columns.clone();
        self.rows = result.rows.clone();
        self.result = Some(result);
        self.auto_limited = auto_limited;
        self.scroll_offset = 0;
        self.h_scroll = 0;
        self.status = ResultStatus::Success;
        self.calculate_widths();
    }

    pub fn set_error(&mut self, msg: String) {
        self.columns.clear();
        self.rows.clear();
        self.col_widths.clear();
        self.scroll_offset = 0;
        self.status = ResultStatus::Error(msg);
    }

    fn calculate_widths(&mut self) {
        self.col_widths = self
            .columns
            .iter()
            .map(|c| UnicodeWidthStr::width(c.as_str()))
            .collect();

        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                if i < self.col_widths.len() {
                    self.col_widths[i] =
                        self.col_widths[i].max(UnicodeWidthStr::width(cell.as_str()));
                }
            }
        }
    }

    pub fn scroll_down(&mut self) {
        if self.scroll_offset + 1 < self.rows.len() {
            self.scroll_offset += 1;
        }
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    pub fn page_down(&mut self, page_size: usize) {
        self.scroll_offset = (self.scroll_offset + page_size).min(self.rows.len().saturating_sub(1));
    }

    pub fn page_up(&mut self, page_size: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(page_size);
    }

    pub fn scroll_right(&mut self, amount: usize) {
        let max = self.total_content_width().saturating_sub(self.visible_width);
        self.h_scroll = (self.h_scroll + amount).min(max);
    }

    pub fn scroll_left(&mut self, amount: usize) {
        self.h_scroll = self.h_scroll.saturating_sub(amount);
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.rows.len().saturating_sub(1);
    }

    pub fn h_scroll_home(&mut self) {
        self.h_scroll = 0;
    }

    pub fn h_scroll_end(&mut self) {
        let total = self.total_content_width();
        self.h_scroll = total.saturating_sub(self.visible_width);
    }

    pub fn h_page_right(&mut self) {
        self.scroll_right(40);
    }

    pub fn h_page_left(&mut self) {
        self.scroll_left(40);
    }

    fn total_content_width(&self) -> usize {
        if self.col_widths.is_empty() {
            return 0;
        }
        // 各カラム幅 + セパレータ " │ " (3文字) + 左右パディング
        self.col_widths.iter().sum::<usize>() + (self.col_widths.len() - 1) * 3 + 2
    }

    pub fn copy_current_row(&self) -> Option<String> {
        self.rows.get(self.scroll_offset).map(|row| row.join(","))
    }
}

// ── 描画 ──

pub fn render(f: &mut Frame, results: &ResultsState, is_focused: bool, area: Rect) {
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Results ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 2 {
        return;
    }

    match &results.status {
        ResultStatus::Empty => {
            let p = Paragraph::new("  クエリを実行すると結果が表示されます");
            f.render_widget(p, inner);
        }
        ResultStatus::Error(msg) => {
            let p = Paragraph::new(vec![
                Line::from(Span::styled(
                    " ERROR",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!(" {}", msg),
                    Style::default().fg(Color::Red),
                )),
            ]);
            f.render_widget(p, inner);
        }
        ResultStatus::Success => {
            render_table(f, results, is_focused, inner);
        }
    }
}

fn render_table(f: &mut Frame, results: &ResultsState, is_focused: bool, area: Rect) {
    if results.columns.is_empty() {
        let p = Paragraph::new("  (0 rows)");
        f.render_widget(p, area);
        return;
    }

    let visible_width = area.width as usize;
    let h_scroll = results.h_scroll;
    let mut lines: Vec<Line> = Vec::new();

    // ヘッダー行
    let header_cells: Vec<String> = results
        .columns
        .iter()
        .enumerate()
        .map(|(i, col)| pad_right(col, results.col_widths.get(i).copied().unwrap_or(0)))
        .collect();
    let header_full = format!(" {} ", header_cells.join(" │ "));
    lines.push(Line::from(Span::styled(
        slice_by_width(&header_full, h_scroll, visible_width),
        Style::default().add_modifier(Modifier::BOLD),
    )));

    // 区切り線
    let sep: Vec<String> = results.col_widths.iter().map(|w| "─".repeat(*w + 2)).collect();
    let sep_full = sep.join("┼");
    lines.push(Line::from(Span::styled(
        slice_by_width(&sep_full, h_scroll, visible_width),
        Style::default().fg(Color::DarkGray),
    )));

    // データ行（表示可能な分だけ）
    let data_height = area.height as usize - 3; // header + sep + footer
    let start = results.scroll_offset;
    let end = (start + data_height).min(results.rows.len());

    for i in start..end {
        let row = &results.rows[i];
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(j, cell)| pad_right(cell, results.col_widths.get(j).copied().unwrap_or(0)))
            .collect();
        let style = if i == results.scroll_offset && is_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        let row_full = format!(" {} ", cells.join(" │ "));
        lines.push(Line::from(Span::styled(
            slice_by_width(&row_full, h_scroll, visible_width),
            style,
        )));
    }

    // フッター
    let auto_limit_label = if results.auto_limited {
        "  [auto LIMIT]"
    } else {
        ""
    };
    let footer = format!(
        " {} rows  ({:.3}s){}",
        results.total_rows,
        results.duration_ms as f64 / 1000.0,
        auto_limit_label,
    );
    // 空行で埋めてフッターを最下部に
    while lines.len() < area.height as usize - 1 {
        lines.push(Line::raw(""));
    }
    lines.push(Line::from(Span::styled(
        footer,
        Style::default().fg(Color::DarkGray),
    )));

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

/// Unicode 幅を考慮して文字列を横スクロールし、表示幅分を切り出す
fn slice_by_width(s: &str, skip: usize, max_width: usize) -> String {
    let mut result = String::new();
    let mut current_width = 0;
    let mut skipped = 0;

    for ch in s.chars() {
        let w = UnicodeWidthStr::width(ch.to_string().as_str());
        if skipped < skip {
            skipped += w;
            continue;
        }
        if current_width + w > max_width {
            break;
        }
        result.push(ch);
        current_width += w;
    }

    result
}

fn pad_right(s: &str, width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}
