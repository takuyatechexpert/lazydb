use crate::db::adapter::QueryResult;
use crate::tui::cc_edit::CcEligibility;
use crate::tui::scrollable::Scrollable;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// NULL セルの表示文字列
pub const NULL_DISPLAY: &str = "NULL";

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
    /// 行データ。`Option<String>` の `None` は SQL の NULL を表す。
    pub rows: Vec<Vec<Option<String>>>,
    pub col_widths: Vec<usize>,
    pub scroll_offset: usize,
    pub h_scroll: usize,
    pub status: ResultStatus,
    pub duration_ms: u64,
    pub total_rows: usize,
    pub auto_limited: bool,
    pub result: Option<QueryResult>,
    pub visible_width: usize,
    /// 表示中の結果に対応するクエリ（cc 用）
    pub last_query: Option<String>,
    /// cc 機能の利用可否（描画・cc 押下時判定に使用）
    pub cc_eligibility: CcEligibility,
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
            last_query: None,
            cc_eligibility: CcEligibility::NotSelect,
        }
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }

    pub fn set_result(&mut self, result: QueryResult, auto_limited: bool, query: String) {
        self.total_rows = result.rows.len();
        self.duration_ms = result.duration_ms;
        self.columns = result.columns.clone();
        self.rows = result.rows.clone();
        self.result = Some(result);
        self.auto_limited = auto_limited;
        self.scroll_offset = 0;
        self.h_scroll = 0;
        self.status = ResultStatus::Success;
        self.last_query = Some(query);
        self.calculate_widths();
    }

    pub fn set_cc_eligibility(&mut self, eligibility: CcEligibility) {
        self.cc_eligibility = eligibility;
    }

    pub fn set_error(&mut self, msg: String) {
        self.columns.clear();
        self.rows.clear();
        self.col_widths.clear();
        self.scroll_offset = 0;
        self.status = ResultStatus::Error(msg);
        // エラー時は cc 対象にならないのでラベルもリセット
        self.cc_eligibility = CcEligibility::NotSelect;
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
                    let s = cell.as_deref().unwrap_or(NULL_DISPLAY);
                    self.col_widths[i] = self.col_widths[i].max(UnicodeWidthStr::width(s));
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
        self.rows.get(self.scroll_offset).map(|row| {
            row.iter()
                .map(|c| c.as_deref().unwrap_or(NULL_DISPLAY))
                .collect::<Vec<&str>>()
                .join(",")
        })
    }
}

impl Scrollable for ResultsState {
    fn move_one_down(&mut self) {
        self.scroll_down();
    }

    fn move_one_up(&mut self) {
        self.scroll_up();
    }

    fn move_one_left(&mut self) {
        self.scroll_left(4);
    }

    fn move_one_right(&mut self) {
        self.scroll_right(4);
    }

    fn scroll_to_top(&mut self) {
        ResultsState::scroll_to_top(self);
    }

    fn scroll_to_bottom(&mut self) {
        ResultsState::scroll_to_bottom(self);
    }

    fn h_scroll_home(&mut self) {
        ResultsState::h_scroll_home(self);
    }

    fn h_scroll_end(&mut self) {
        ResultsState::h_scroll_end(self);
    }

    fn page_down(&mut self, page_size: usize) {
        ResultsState::page_down(self, page_size);
    }

    fn page_up(&mut self, page_size: usize) {
        ResultsState::page_up(self, page_size);
    }

    fn h_page_left(&mut self) {
        ResultsState::h_page_left(self);
    }

    fn h_page_right(&mut self) {
        ResultsState::h_page_right(self);
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
        let row_focused = i == results.scroll_offset && is_focused;
        let base_style = if row_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        let null_style = if row_focused {
            // フォーカス行でも NULL は控えめに見せる（Cyan より一段暗い色）
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC)
        } else {
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC)
        };

        // セグメント単位（スタイル付き文字列）に分解して Span を組み立てる
        let mut segments: Vec<(String, Style)> = Vec::with_capacity(row.len() * 2 + 2);
        // 先頭スペース
        segments.push((" ".to_string(), base_style));
        for (j, cell) in row.iter().enumerate() {
            if j > 0 {
                segments.push((" │ ".to_string(), base_style));
            }
            let width = results.col_widths.get(j).copied().unwrap_or(0);
            match cell.as_deref() {
                Some(s) => segments.push((pad_right(s, width), base_style)),
                None => segments.push((pad_right(NULL_DISPLAY, width), null_style)),
            }
        }
        // 末尾スペース
        segments.push((" ".to_string(), base_style));

        let spans = slice_segments_by_width(&segments, h_scroll, visible_width);
        lines.push(Line::from(spans));
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
    lines.push(Line::from(vec![
        Span::styled(footer, Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled(
            results.cc_eligibility.label().to_string(),
            Style::default().fg(results.cc_eligibility.label_color()),
        ),
    ]));

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

/// Unicode 幅を考慮して文字列を横スクロールし、表示幅分を切り出す
fn slice_by_width(s: &str, skip: usize, max_width: usize) -> String {
    let mut result = String::new();
    let mut current_width = 0;
    let mut skipped = 0;

    for ch in s.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
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

/// スタイル付きセグメント列を、横スクロール量と最大幅に従って切り出して `Span` 列を返す。
///
/// 各セグメントごとに `slice_by_width` 相当の処理を行いつつ、消費した幅を共有することで
/// セル間の境界（`│`）でスタイルが切り替わっても一貫した位置決めを保つ。
fn slice_segments_by_width(
    segments: &[(String, Style)],
    skip: usize,
    max_width: usize,
) -> Vec<Span<'static>> {
    let mut result: Vec<Span<'static>> = Vec::new();
    let mut skipped = 0;
    let mut consumed = 0;

    for (text, style) in segments {
        if consumed >= max_width {
            break;
        }
        let mut buf = String::new();
        for ch in text.chars() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            if skipped < skip {
                skipped += w;
                continue;
            }
            if consumed + w > max_width {
                break;
            }
            buf.push(ch);
            consumed += w;
        }
        if !buf.is_empty() {
            result.push(Span::styled(buf, *style));
        }
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
#[cfg(test)]
mod tests {
    use super::*;

    fn results_with(rows: usize, cols: usize) -> ResultsState {
        let mut r = ResultsState::new();
        r.columns = (0..cols).map(|i| format!("c{}", i)).collect();
        r.rows = (0..rows)
            .map(|i| (0..cols).map(|j| Some(format!("r{}_{}", i, j))).collect())
            .collect();
        r.col_widths = vec![10; cols];
        r.status = ResultStatus::Success;
        r.total_rows = rows;
        r.visible_width = 30;
        r
    }

    // ── move_one_down / move_one_up ──

    #[test]
    fn scrollable_results_move_one_down_advances_offset() {
        let mut r = results_with(10, 2);
        r.scroll_offset = 0;
        r.move_one_down();
        assert_eq!(r.scroll_offset, 1);
    }

    #[test]
    fn scrollable_results_move_one_down_clamps_at_last_row() {
        let mut r = results_with(5, 2);
        r.scroll_offset = 4;
        r.move_one_down();
        // 既存 scroll_down は scroll_offset + 1 < rows.len() のときだけ進む
        assert_eq!(r.scroll_offset, 4);
    }

    #[test]
    fn scrollable_results_move_one_up_retreats_offset() {
        let mut r = results_with(10, 2);
        r.scroll_offset = 5;
        r.move_one_up();
        assert_eq!(r.scroll_offset, 4);
    }

    #[test]
    fn scrollable_results_move_one_up_clamps_at_zero() {
        let mut r = results_with(10, 2);
        r.scroll_offset = 0;
        r.move_one_up();
        assert_eq!(r.scroll_offset, 0);
    }

    // ── move_one_left / move_one_right (4 セル単位) ──

    #[test]
    fn scrollable_results_move_one_right_advances_h_scroll_by_4() {
        let mut r = results_with(10, 5);
        // total_content_width >> visible_width になるように col_widths を膨らませる
        r.col_widths = vec![50; 5];
        r.visible_width = 10;
        r.h_scroll = 0;
        r.move_one_right();
        assert_eq!(r.h_scroll, 4);
    }

    #[test]
    fn scrollable_results_move_one_left_retreats_h_scroll_by_4() {
        let mut r = results_with(10, 5);
        r.h_scroll = 10;
        r.move_one_left();
        assert_eq!(r.h_scroll, 6);
    }

    #[test]
    fn scrollable_results_move_one_left_clamps_at_zero() {
        let mut r = results_with(10, 5);
        r.h_scroll = 2;
        r.move_one_left();
        assert_eq!(r.h_scroll, 0);
    }

    // ── scroll_to_top / scroll_to_bottom ──

    #[test]
    fn scrollable_results_scroll_to_top_zeros_offset() {
        let mut r = results_with(10, 2);
        r.scroll_offset = 5;
        Scrollable::scroll_to_top(&mut r);
        assert_eq!(r.scroll_offset, 0);
    }

    #[test]
    fn scrollable_results_scroll_to_bottom_lands_on_last() {
        let mut r = results_with(10, 2);
        r.scroll_offset = 0;
        Scrollable::scroll_to_bottom(&mut r);
        assert_eq!(r.scroll_offset, 9);
    }

    #[test]
    fn scrollable_results_scroll_to_bottom_with_no_rows_is_zero() {
        let mut r = ResultsState::new();
        r.scroll_offset = 0;
        Scrollable::scroll_to_bottom(&mut r);
        assert_eq!(r.scroll_offset, 0);
    }

    // ── h_scroll_home / h_scroll_end ──

    #[test]
    fn scrollable_results_h_scroll_home_zeros_h_scroll() {
        let mut r = results_with(10, 5);
        r.h_scroll = 50;
        Scrollable::h_scroll_home(&mut r);
        assert_eq!(r.h_scroll, 0);
    }

    #[test]
    fn scrollable_results_h_scroll_end_lands_at_max() {
        let mut r = results_with(10, 3);
        r.col_widths = vec![20; 3]; // total = 20*3 + 3*2 + 2 = 68
        r.visible_width = 10;
        Scrollable::h_scroll_end(&mut r);
        // total_content_width(68) - visible_width(10) = 58
        assert_eq!(r.h_scroll, 58);
    }

    // ── page_down / page_up ──

    #[test]
    fn scrollable_results_page_down_advances_by_page_size() {
        let mut r = results_with(50, 2);
        r.scroll_offset = 0;
        Scrollable::page_down(&mut r, 20);
        assert_eq!(r.scroll_offset, 20);
    }

    #[test]
    fn scrollable_results_page_down_clamps_at_last_row() {
        let mut r = results_with(10, 2);
        r.scroll_offset = 5;
        Scrollable::page_down(&mut r, 20);
        assert_eq!(r.scroll_offset, 9);
    }

    #[test]
    fn scrollable_results_page_up_retreats_by_page_size() {
        let mut r = results_with(50, 2);
        r.scroll_offset = 30;
        Scrollable::page_up(&mut r, 20);
        assert_eq!(r.scroll_offset, 10);
    }

    #[test]
    fn scrollable_results_page_up_clamps_at_zero() {
        let mut r = results_with(10, 2);
        r.scroll_offset = 5;
        Scrollable::page_up(&mut r, 20);
        assert_eq!(r.scroll_offset, 0);
    }

    // ── h_page_left / h_page_right ──

    #[test]
    fn scrollable_results_h_page_right_advances_h_scroll_by_40() {
        let mut r = results_with(10, 5);
        r.col_widths = vec![50; 5]; // total >>>
        r.visible_width = 10;
        r.h_scroll = 0;
        Scrollable::h_page_right(&mut r);
        assert_eq!(r.h_scroll, 40);
    }

    #[test]
    fn scrollable_results_h_page_left_retreats_h_scroll_by_40() {
        let mut r = results_with(10, 5);
        r.h_scroll = 100;
        Scrollable::h_page_left(&mut r);
        assert_eq!(r.h_scroll, 60);
    }

    #[test]
    fn scrollable_results_h_page_left_clamps_at_zero() {
        let mut r = results_with(10, 5);
        r.h_scroll = 20;
        Scrollable::h_page_left(&mut r);
        assert_eq!(r.h_scroll, 0);
    }
}
