use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

use super::{editor, results, schema, App};

pub fn render_panels(f: &mut Frame, app: &App, area: Rect) {
    // 左30% / 右70%
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(area);

    // 右: 上65% / 下35%
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(horizontal[1]);

    schema::render(f, app, horizontal[0]);
    editor::render(f, app, right[0]);
    results::render(f, app, right[1]);
}
