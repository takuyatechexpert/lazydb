use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render(f: &mut Frame, area: Rect) {
    let popup = centered_rect(60, 70, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let help_text = vec![
        Line::from(Span::styled(
            "Global",
            Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
        )),
        help_line("Ctrl+E", "クエリ実行"),
        help_line("Ctrl+C", "接続切り替え"),
        help_line("Ctrl+H", "履歴ピッカー"),
        help_line("Ctrl+X", "エクスポート"),
        help_line("Tab/Shift+Tab", "パネル移動"),
        help_line("?", "ヘルプ表示/非表示"),
        help_line("Ctrl+Q", "終了"),
        Line::raw(""),
        Line::from(Span::styled(
            "Tabs",
            Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
        )),
        help_line("Ctrl+T", "新規タブ追加"),
        help_line("Ctrl+W", "タブを閉じる"),
        help_line("Ctrl+N/P", "次/前のタブ"),
        Line::raw(""),
        Line::from(Span::styled(
            "Scroll & Navigation (3ペイン共通)",
            Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
        )),
        help_line("j/k/↑↓", "縦1単位移動"),
        help_line("h/l/←→", "横1単位移動"),
        help_line("g / G", "縦先頭 / 縦末尾"),
        help_line("0 / Home", "横先頭"),
        help_line("$ / End", "横末尾"),
        help_line("PgDn/PgUp", "縦20単位移動"),
        help_line("Ctrl+D/Ctrl+U", "縦20単位移動"),
        help_line("H / L", "横40単位移動"),
        help_line("zz", "カーソル行を画面中央に寄せる"),
        Line::raw(""),
        Line::from(Span::styled(
            "Schema Browser",
            Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
        )),
        help_line("Enter", "テーブル展開・折りたたみ"),
        help_line("s", "クイック SELECT * FROM"),
        help_line("y", "テーブル名をコピー"),
        help_line("r", "スキーマ再読み込み"),
        Line::raw(""),
        Line::from(Span::styled(
            "Editor Normal",
            Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
        )),
        help_line("i/a/A/o/O", "Insert モードに入る"),
        help_line("w/b/e", "単語移動"),
        help_line("^", "行の最初の非空白"),
        help_line("x/dd/D/C", "削除"),
        help_line("u/Ctrl+R", "undo/redo"),
        help_line("=", "クエリ全体をフォーマット"),
        Line::raw(""),
        Line::from(Span::styled(
            "Editor Insert",
            Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
        )),
        help_line("Esc", "Normal モードに戻る"),
        Line::raw(""),
        Line::from(Span::styled(
            "Results",
            Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
        )),
        help_line("y", "行データコピー"),
        help_line("cc", "カーソル行の UPDATE 文を Editor に追記"),
    ];

    let paragraph = Paragraph::new(help_text).block(block);
    f.render_widget(paragraph, popup);
}

fn help_line<'a>(key: &'a str, desc: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{:<16}", key),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw(desc),
    ])
}

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
