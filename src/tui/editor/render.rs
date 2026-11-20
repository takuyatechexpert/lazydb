//! Query Editor の描画担当。
//!
//! 本文行・検索バー・補完ポップアップのレンダリングを集約する。
//! `super::EditorState` を読み取り専用で受け取り、ratatui の `Frame` に書き出す。

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use unicode_width::UnicodeWidthStr;

use super::{char_count, char_width, width_between, EditorMode, EditorState, SQL_KEYWORDS};

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

    // 検索バーを描画する場合は inner の最下行を 1 行確保する
    let show_search_bar = editor.search.active || !editor.search.query.is_empty();
    let (content_inner, search_bar_area) = if show_search_bar && inner.height >= 2 {
        let bar = Rect {
            x: inner.x,
            y: inner.y + inner.height - 1,
            width: inner.width,
            height: 1,
        };
        let content = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: inner.height - 1,
        };
        (content, Some(bar))
    } else {
        (inner, None)
    };

    let line_num_width = format!("{}", editor.lines.len()).len().max(2);
    let editor_width = (content_inner.width as usize).saturating_sub(line_num_width + 1); // 1 for "│"
    let visible_height = content_inner.height as usize;

    // 本文行の組み立てとレンダリング
    let display_lines = render_content_lines(editor, line_num_width, editor_width, visible_height);
    f.render_widget(Paragraph::new(display_lines), content_inner);

    // 検索バー
    if let Some(bar) = search_bar_area {
        render_search_bar(f, editor, bar);
    }

    if !is_focused || editor.cursor.1 < editor.h_scroll_offset {
        return;
    }
    // カーソルの画面座標。列は横スクロール位置からカーソルまでの表示幅で決まる
    let inner = content_inner;
    let line = editor.lines.get(editor.cursor.0).map_or("", String::as_str);
    let cursor_x = inner.x
        + line_num_width as u16
        + 1
        + width_between(line, editor.h_scroll_offset, editor.cursor.1) as u16;
    let cursor_y = inner.y + editor.cursor.0.saturating_sub(editor.scroll_offset) as u16;

    // カーソル表示（Normal / Insert 両モード）
    // 検索バー入力中はバー側にカーソルを出すので、本文側のカーソルは出さない
    if !editor.search.active
        && cursor_x < inner.x + inner.width
        && cursor_y < inner.y + inner.height
    {
        f.set_cursor_position((cursor_x, cursor_y));
    }

    // オートコンプリート プルダウン
    if editor.completion.active && !editor.completion.candidates.is_empty() {
        let frame_area = f.area();
        render_completion_popup(f, editor, (cursor_x, cursor_y), frame_area);
    }
}

/// 本文行（行番号 + シンタックスハイライト + 選択 / 検索ハイライト）を組み立てて返す。
/// 表示範囲に満たない分は `~` 行で埋める。
fn render_content_lines(
    editor: &EditorState,
    line_num_width: usize,
    editor_width: usize,
    visible_height: usize,
) -> Vec<Line<'static>> {
    let start = editor.scroll_offset;
    let end = (start + visible_height).min(editor.lines.len());
    let selection = editor.selection_range();

    let mut display_lines: Vec<Line<'static>> = Vec::new();

    for i in start..end {
        let line = &editor.lines[i];
        let line_num = format!("{:>width$}", i + 1, width = line_num_width);

        let mut spans: Vec<Span<'static>> = vec![
            Span::styled(line_num, Style::default().fg(Color::DarkGray)),
            Span::styled("│".to_string(), Style::default().fg(Color::DarkGray)),
        ];

        // h_scroll を考慮した可視部分（表示幅が editor_width に収まるところまで）
        let mut used_width = 0usize;
        let visible_str: String = line
            .chars()
            .skip(editor.h_scroll_offset)
            .take_while(|&ch| {
                used_width += char_width(ch);
                used_width <= editor_width
            })
            .collect();

        // 文字位置 -> 背景色のマップ（可視範囲）
        let mut bg: Vec<Option<Color>> = vec![None; visible_str.chars().count()];

        // 選択範囲のハイライト
        if let Some(((sr, sc), (er, ec))) = selection {
            if i >= sr && i <= er {
                let (sel_start_col, sel_end_col) = match editor.mode {
                    EditorMode::VisualLine => (0usize, char_count(line)),
                    _ => {
                        let s_col = if i == sr { sc } else { 0 };
                        let e_col = if i == er { ec + 1 } else { char_count(line) };
                        (s_col, e_col)
                    }
                };
                // 両端とも可視範囲に切り詰める。始点が可視範囲より右にあると
                // 空の範囲になる（始点だけ切り詰めないと bg の長さを超えて panic する）
                let visible_end = sel_end_col
                    .saturating_sub(editor.h_scroll_offset)
                    .min(bg.len());
                let visible_start = sel_start_col
                    .saturating_sub(editor.h_scroll_offset)
                    .min(visible_end);
                for slot in &mut bg[visible_start..visible_end] {
                    *slot = Some(Color::DarkGray);
                }
            }
        }

        // 検索マッチのハイライト
        if !editor.search.query.is_empty() {
            for &(row, col, len) in &editor.search.matches {
                if row != i { continue; }
                // 選択範囲と同じく、両端とも可視範囲に切り詰める
                let m_end = (col + len)
                    .saturating_sub(editor.h_scroll_offset)
                    .min(bg.len());
                let m_start = col.saturating_sub(editor.h_scroll_offset).min(m_end);
                for slot in &mut bg[m_start..m_end] {
                    *slot = Some(Color::Yellow);
                }
            }
        }

        // シンタックスハイライト後、bg を文字単位でマージ
        let syntax_spans = highlight_sql(&visible_str);
        let mut char_offset = 0usize;
        for s in syntax_spans {
            let content = s.content.into_owned();
            let nchars = content.chars().count();
            // bg の連続性で chunk を切る
            let mut chunk_start = 0usize;
            let mut current_bg = if char_offset < bg.len() { bg[char_offset] } else { None };
            let chars: Vec<char> = content.chars().collect();
            for k in 0..chars.len() {
                let pos = char_offset + k;
                let here = if pos < bg.len() { bg[pos] } else { None };
                if here != current_bg {
                    let chunk: String = chars[chunk_start..k].iter().collect();
                    let mut style = s.style;
                    if let Some(c) = current_bg {
                        style = style.bg(c);
                        // 背景に黄色を載せたら前景はキーワード色だと見づらいので黒に
                        if c == Color::Yellow {
                            style = style.fg(Color::Black);
                        }
                    }
                    spans.push(Span::styled(chunk, style));
                    chunk_start = k;
                    current_bg = here;
                }
            }
            // 最終チャンク
            let chunk: String = chars[chunk_start..].iter().collect();
            let mut style = s.style;
            if let Some(c) = current_bg {
                style = style.bg(c);
                if c == Color::Yellow {
                    style = style.fg(Color::Black);
                }
            }
            spans.push(Span::styled(chunk, style));
            char_offset += nchars;
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

    display_lines
}

/// 検索バー (`/query [n/m]`) をエディタ最下行に描画する。
fn render_search_bar(f: &mut Frame, editor: &EditorState, area: Rect) {
    let cursor_glyph = if editor.search.active { "█" } else { "" };
    let prompt_color = if editor.search.active { Color::Yellow } else { Color::DarkGray };
    let count_label = if editor.search.matches.is_empty() {
        String::new()
    } else {
        format!(" [{}/{}]", editor.search.current + 1, editor.search.matches.len())
    };
    let line = Line::from(vec![
        Span::styled("/", Style::default().fg(prompt_color)),
        Span::raw(editor.search.query.clone()),
        Span::styled(cursor_glyph, Style::default().fg(Color::Gray)),
        Span::styled(count_label, Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

/// オートコンプリートのドロップダウンをカーソル直下に描画する。
/// `frame_area` に収まらない場合は描画しない。
fn render_completion_popup(
    f: &mut Frame,
    editor: &EditorState,
    cursor_pos: (u16, u16),
    frame_area: Rect,
) {
    let (cursor_x, cursor_y) = cursor_pos;
    let max_items = editor.completion.candidates.len().min(8);
    let popup_width = editor
        .completion
        .candidates
        .iter()
        .map(|c| UnicodeWidthStr::width(c.as_str()))
        .max()
        .unwrap_or(10)
        .max(10) as u16
        + 4;

    // prefix の表示幅だけ左にオフセット
    let prefix_width = UnicodeWidthStr::width(editor.completion.prefix.as_str()) as u16;
    let popup_x = cursor_x.saturating_sub(prefix_width);
    let popup_y = cursor_y + 1;
    let popup_height = max_items as u16 + 2; // ボーダー分

    if popup_y + popup_height > frame_area.height || popup_x + popup_width > frame_area.width {
        return;
    }

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

    /// 本文の各行から、背景が `color` の文字だけを集める（行番号と区切りの 2 span は除く）。
    fn chars_with_bg(lines: &[Line<'static>], color: Color) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans[2..]
                    .iter()
                    .filter(|s| s.style.bg == Some(color))
                    .map(|s| s.content.as_ref())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn search_match_beyond_right_edge_is_not_drawn() {
        // 2 つ目の zzz は 162 桁目で、幅 79 の可視範囲より右にある
        let mut e = EditorState::new();
        e.set_content(&format!("select zzz {} zzz", "a".repeat(150)));
        e.search.query = "zzz".to_string();
        e.recompute_matches();
        e.h_scroll_offset = 0;

        let lines = render_content_lines(&e, 1, 79, 1);

        assert_eq!(chars_with_bg(&lines, Color::Yellow), vec!["zzz"]);
    }

    #[test]
    fn selection_starting_beyond_right_edge_is_not_drawn() {
        // 長い行の 156 桁目で v を押し、j で短い行へ移った状態
        let mut e = EditorState::new();
        e.set_content(&format!("select {}\nx", "b".repeat(150)));
        e.mode = EditorMode::Visual;
        e.visual_anchor = Some((0, 156));
        e.cursor = (1, 0);
        e.h_scroll_offset = 0;

        let lines = render_content_lines(&e, 1, 79, 2);

        assert_eq!(chars_with_bg(&lines, Color::DarkGray), vec!["", "x"]);
    }

    /// `editor` をフォーカスありで `width`x`height` の TestBackend に 1 フレーム描く。
    fn draw_editor(
        e: &EditorState,
        width: u16,
        height: u16,
    ) -> ratatui::Terminal<ratatui::backend::TestBackend> {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render(f, e, true, f.area())).unwrap();
        terminal
    }

    #[test]
    fn cursor_after_cjk_text_is_placed_by_display_width() {
        // 本文の左端は x=4（枠 1 + 行番号 2 + 区切り 1）。"select 'あいう'" は 12 文字・15 セル
        let mut e = EditorState::new();
        e.set_content("select 'あいう'");

        let mut terminal = draw_editor(&e, 40, 5);

        let pos = ratatui::backend::Backend::get_cursor_position(terminal.backend_mut()).unwrap();
        assert_eq!((pos.x, pos.y), (4 + 15, 1));
    }

    #[test]
    fn completion_popup_for_cjk_prefix_aligns_and_fits_by_display_width() {
        // 「顧」は x=4+14 の 2 セル。ポップアップは「顧」の左端から、" 顧客一覧表 " 12 セル + 枠 2 の幅
        let mut e = EditorState::new();
        e.set_content("select * from 顧");
        e.completion.candidates = vec!["顧客一覧表".to_string()];
        e.completion.prefix = "顧".to_string();
        e.completion.active = true;

        let terminal = draw_editor(&e, 40, 6);

        let buf = terminal.backend().buffer();
        assert_eq!(buf[(18, 2)].symbol(), "┌");
        assert_eq!(buf[(18 + 13, 2)].symbol(), "┐");
    }

    #[test]
    fn visible_part_of_cjk_line_is_cut_by_display_width() {
        // 幅 5 に全角 3 文字（6 セル）は入らない。文字数で切ると 3 文字とも入る
        let mut e = EditorState::new();
        e.set_content("あいう");

        let lines = render_content_lines(&e, 1, 5, 1);

        let text: String = lines[0].spans[2..]
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(text, "あい");
    }
}
