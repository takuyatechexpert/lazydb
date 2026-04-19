use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

use super::{editor, results, schema, App, Panel};

pub fn render_panels(f: &mut Frame, app: &App, area: Rect) {
    // 左30% / 右70%
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(area);

    // 右: タブバー(1行) + Editor(65%) + Results(35%)
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),      // タブバー
            Constraint::Percentage(65), // Editor
            Constraint::Percentage(35), // Results
        ])
        .split(horizontal[1]);

    let tab = &app.tabs[app.active_tab];
    let is_editor_focused = app.active_panel == Panel::Editor;
    let is_results_focused = app.active_panel == Panel::Results;

    schema::render(f, app, horizontal[0]);
    render_tab_bar(f, &app.tabs, app.active_tab, right[0]);
    editor::render(f, &tab.editor, is_editor_focused, right[1]);
    results::render(f, &tab.results, is_results_focused, right[2]);
}

pub fn render_tab_bar(f: &mut Frame, tabs: &[super::Tab], active_tab: usize, area: Rect) {
    use ratatui::{
        style::{Color, Style},
        text::{Line, Span},
        widgets::Paragraph,
    };

    let mut spans: Vec<Span> = Vec::new();

    for (i, tab) in tabs.iter().enumerate() {
        let label = format!(" {}:{} ", i + 1, tab.name);
        if i == active_tab {
            spans.push(Span::styled(
                label,
                Style::default().fg(Color::Black).bg(Color::White),
            ));
        } else {
            spans.push(Span::styled(
                label,
                Style::default().fg(Color::DarkGray),
            ));
        }
    }
    spans.push(Span::styled(" [+] ", Style::default().fg(Color::DarkGray)));

    let tab_bar = Paragraph::new(Line::from(spans));
    f.render_widget(tab_bar, area);
}
