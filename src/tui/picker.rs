use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use super::{label_color, App};
use crate::config::connections::ConnectionConfig;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    // 画面中央にポップアップ配置
    let popup = centered_rect(50, 60, area);

    // 背景をクリア
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Select Connection ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let items: Vec<ListItem> = app
        .connections
        .iter()
        .enumerate()
        .map(|(i, conn)| {
            let is_selected = i == app.picker_cursor;
            let prefix = if is_selected { "▶ " } else { "  " };

            let name = conn.name();
            let label = conn.label().unwrap_or("-");
            let type_str = match conn {
                ConnectionConfig::Direct(_) => "direct",
                ConnectionConfig::Ssh(_) => "ssh",
                ConnectionConfig::Ssm(_) => "ssm",
            };

            let db_str = conn.db_type().to_string();

            let line = Line::from(vec![
                Span::raw(prefix),
                Span::styled(
                    format!("{:<20}", name),
                    if is_selected {
                        Style::default().add_modifier(Modifier::BOLD).fg(Color::White)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
                Span::styled(
                    format!(" [{}]", label),
                    Style::default().fg(label_color(label)),
                ),
                Span::styled(
                    format!("  {}", type_str),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("  {}", db_str),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);

            ListItem::new(line)
        })
        .collect();

    // "+ New Connection" 項目を追加
    let new_conn_idx = app.connections.len();
    let is_new_selected = app.picker_cursor == new_conn_idx;
    let new_prefix = if is_new_selected { "▶ " } else { "  " };
    let new_item = ListItem::new(Line::from(vec![
        Span::raw(new_prefix),
        Span::styled(
            "+ New Connection",
            if is_new_selected {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Green)
            },
        ),
    ]));
    let mut all_items = items;
    all_items.push(new_item);

    let list = List::new(all_items).block(block);
    f.render_widget(list, popup);

    // フッターヒント
    let hint_area = Rect {
        x: popup.x,
        y: popup.y + popup.height,
        width: popup.width,
        height: 1,
    };
    if hint_area.y < area.height {
        let hint = Paragraph::new(Line::from(vec![
            Span::styled(" j/k ", Style::default().fg(Color::Cyan)),
            Span::raw("移動  "),
            Span::styled("Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("選択  "),
            Span::styled("Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("閉じる"),
        ]))
        .style(Style::default().fg(Color::DarkGray));
        f.render_widget(hint, hint_area);
    }
}

pub fn render_history(f: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(70, 70, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Query History ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    if inner.height < 3 {
        return;
    }

    // 検索バー（1行目）
    let search_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    };
    let search_line = Line::from(vec![
        Span::styled(" 🔍 ", Style::default().fg(Color::Yellow)),
        Span::raw(&app.history_filter),
        Span::styled("█", Style::default().fg(Color::Gray)),
    ]);
    f.render_widget(Paragraph::new(search_line), search_area);

    // リスト領域
    let list_area = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: inner.height.saturating_sub(1),
    };

    if app.history_entries.is_empty() {
        let msg = if app.history_filter.is_empty() {
            "  履歴がありません"
        } else {
            "  一致する履歴がありません"
        };
        f.render_widget(
            Paragraph::new(msg).style(Style::default().fg(Color::DarkGray)),
            list_area,
        );
    } else {
        let items: Vec<ListItem> = app
            .history_entries
            .iter()
            .enumerate()
            .take(list_area.height as usize)
            .map(|(i, entry)| {
                let is_selected = i == app.history_cursor;
                let prefix = if is_selected { "▶ " } else { "  " };
                let query_preview: String = entry
                    .query
                    .chars()
                    .take(inner.width as usize - 30)
                    .map(|c| if c == '\n' { ' ' } else { c })
                    .collect();
                let time_str = entry.executed_at.format("%m-%d %H:%M").to_string();

                let line = Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(
                        format!("{:<width$}", query_preview, width = inner.width as usize - 28),
                        if is_selected {
                            Style::default()
                                .add_modifier(Modifier::BOLD)
                                .fg(Color::White)
                        } else {
                            Style::default().fg(Color::White)
                        },
                    ),
                    Span::styled(
                        format!(" {} ", time_str),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!("[{}]", entry.connection),
                        Style::default().fg(Color::Cyan),
                    ),
                ]);

                ListItem::new(line)
            })
            .collect();

        let list = List::new(items);
        f.render_widget(list, list_area);
    }

    // フッターヒント
    let hint_area = Rect {
        x: popup.x,
        y: popup.y + popup.height,
        width: popup.width,
        height: 1,
    };
    if hint_area.y < area.height {
        let hint = Paragraph::new(Line::from(vec![
            Span::styled(" ↑↓ ", Style::default().fg(Color::Cyan)),
            Span::raw("移動  "),
            Span::styled("Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("挿入  "),
            Span::styled("文字入力 ", Style::default().fg(Color::Cyan)),
            Span::raw("絞り込み  "),
            Span::styled("Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("閉じる"),
        ]))
        .style(Style::default().fg(Color::DarkGray));
        f.render_widget(hint, hint_area);
    }
}

pub fn render_export_format(f: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(30, 20, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Export Format ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let formats = ["CSV", "JSON"];
    let items: Vec<ListItem> = formats
        .iter()
        .enumerate()
        .map(|(i, fmt)| {
            let is_selected = i == app.export_cursor;
            let prefix = if is_selected { "▶ " } else { "  " };
            let style = if is_selected {
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(Color::White)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(Span::styled(format!("{}{}", prefix, fmt), style)))
        })
        .collect();

    let list = List::new(items).block(block);
    f.render_widget(list, popup);
}

pub fn render_export_path(f: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(60, 15, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Export Path ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let lines = vec![
        Line::from(Span::styled(
            " 保存先パスを入力してください:",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw(" "),
            Span::styled(&app.export_path_input, Style::default().fg(Color::White)),
            Span::styled("█", Style::default().fg(Color::Gray)),
        ]),
    ];

    f.render_widget(Paragraph::new(lines), inner);

    let hint_area = Rect {
        x: popup.x,
        y: popup.y + popup.height,
        width: popup.width,
        height: 1,
    };
    if hint_area.y < area.height {
        let hint = Paragraph::new(Line::from(vec![
            Span::styled("Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("保存  "),
            Span::styled("Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("キャンセル"),
        ]))
        .style(Style::default().fg(Color::DarkGray));
        f.render_widget(hint, hint_area);
    }
}

pub fn render_new_connection(f: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(55, 75, area);
    f.render_widget(Clear, popup);

    let title = format!(" New Connection ({} / {}) ",
        app.new_conn_form.conn_type.label(),
        app.new_conn_form.db_type.label(),
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let form = &app.new_conn_form;
    let mut lines: Vec<Line> = Vec::new();

    // Row 0: conn_type selector
    {
        let is_active = form.cursor == 0;
        let label_style = if is_active {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let value_style = if is_active {
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let hint = if is_active { "  h/l または ← → で切替" } else { "" };

        lines.push(Line::from(vec![
            Span::styled(format!("  {:<18}", "type"), label_style),
            Span::styled(format!("◀ {} ▶", form.conn_type.label()), value_style),
            Span::styled(hint, Style::default().fg(Color::DarkGray)),
        ]));
        lines.push(Line::raw(""));
    }

    // Row 1: db_type selector
    {
        let is_active = form.cursor == 1;
        let label_style = if is_active {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let value_style = if is_active {
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let hint = if is_active { "  h/l または ← → で切替" } else { "" };

        lines.push(Line::from(vec![
            Span::styled(format!("  {:<18}", "db_type"), label_style),
            Span::styled(format!("◀ {} ▶", form.db_type.label()), value_style),
            Span::styled(hint, Style::default().fg(Color::DarkGray)),
        ]));
        lines.push(Line::raw(""));
    }

    // Row 2+: dynamic fields
    for (i, (label, value)) in form.fields.iter().enumerate() {
        let is_active = form.cursor == i + 2;
        let label_style = if is_active {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        // readonly フィールドは bool トグル表示
        if *label == "readonly" {
            let value_style = if is_active {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let display = if value == "true" { "true" } else { "false" };
            let hint = if is_active { "  h/l/Space でトグル" } else { "" };
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<18}", label), label_style),
                Span::styled(format!("◀ {} ▶", display), value_style),
                Span::styled(hint, Style::default().fg(Color::DarkGray)),
            ]));
            lines.push(Line::raw(""));
            continue;
        }

        let cursor_indicator = if is_active { "█" } else { "" };

        // password フィールドはマスク表示
        let display_value: String = if *label == "password" && !value.is_empty() {
            "*".repeat(value.len())
        } else {
            value.clone()
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {:<18}", label), label_style),
            Span::styled(display_value, Style::default().fg(Color::White)),
            Span::styled(cursor_indicator, Style::default().fg(Color::Gray)),
        ]));
        lines.push(Line::raw(""));
    }

    f.render_widget(Paragraph::new(lines), inner);

    // フッターヒント
    let hint_area = Rect {
        x: popup.x,
        y: popup.y + popup.height,
        width: popup.width,
        height: 1,
    };
    if hint_area.y < area.height {
        let hint = Paragraph::new(Line::from(vec![
            Span::styled(" Tab/↑↓ ", Style::default().fg(Color::Cyan)),
            Span::raw("移動  "),
            Span::styled("h/l ← → ", Style::default().fg(Color::Cyan)),
            Span::raw("切替/トグル  "),
            Span::styled("Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("作成  "),
            Span::styled("Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("キャンセル"),
        ]))
        .style(Style::default().fg(Color::DarkGray));
        f.render_widget(hint, hint_area);
    }
}

/// area の中央に width% x height% のサブ領域を返す
fn centered_rect(width_percent: u16, height_percent: u16, area: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100 - height_percent) / 2),
        ])
        .split(area);

    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vert[1]);

    horiz[1]
}
