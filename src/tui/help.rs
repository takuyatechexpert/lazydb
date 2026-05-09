use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
    Frame,
};

/// ヘルプポップアップを描画する。
///
/// `scroll` は呼び出し側が保持する縦スクロールオフセット。
/// 内容が画面に収まらない場合に備え、ここで `[0, max_scroll]` にクランプして書き戻す。
/// これにより、ターミナル縮小によって `*scroll` が末尾を超えても次フレームで自動的に整合する。
pub fn render(f: &mut Frame, area: Rect, scroll: &mut u16) {
    let popup = centered_rect(70, 85, area);
    f.render_widget(Clear, popup);

    let help_text = build_help_text();
    let total_lines = help_text.len() as u16;

    // 内側の高さ = ポップアップ高 - 上下ボーダー(2)
    let inner_h = popup.height.saturating_sub(2);
    let max_scroll = total_lines.saturating_sub(inner_h);

    if *scroll > max_scroll {
        *scroll = max_scroll;
    }

    let title = if max_scroll > 0 {
        format!(
            " Help  ({}/{})  j/k:scroll  PgDn/PgUp  g/G ",
            *scroll, max_scroll
        )
    } else {
        " Help ".to_string()
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(help_text)
        .block(block)
        .scroll((*scroll, 0));
    f.render_widget(paragraph, popup);

    // スクロールバー（オーバーフロー時のみ表示）
    if max_scroll > 0 {
        let mut sb_state =
            ScrollbarState::new(max_scroll as usize).position(*scroll as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None);
        f.render_stateful_widget(scrollbar, popup, &mut sb_state);
    }
}

/// 表示用の総行数を返す（テスト・スクロール上限計算で利用）。
pub fn total_lines() -> u16 {
    build_help_text().len() as u16
}

fn build_help_text() -> Vec<Line<'static>> {
    vec![
        section("Global"),
        help_line("Ctrl+E", "クエリ実行"),
        help_line("Ctrl+C", "接続切り替え"),
        help_line("Ctrl+H", "履歴ピッカー"),
        help_line("Ctrl+X", "エクスポート"),
        help_line("Tab/Shift+Tab", "パネル移動"),
        help_line("?", "ヘルプ表示/非表示"),
        help_line("Ctrl+Q", "終了"),
        Line::raw(""),
        section("Tabs"),
        help_line("Ctrl+T", "新規タブ追加"),
        help_line("Ctrl+W", "タブを閉じる"),
        help_line("Ctrl+N/P", "次/前のタブ"),
        Line::raw(""),
        section("Scroll & Navigation (3ペイン共通)"),
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
        section("Schema Browser"),
        help_line("Enter", "テーブル展開・折りたたみ"),
        help_line("s", "クイック SELECT * FROM"),
        help_line("y", "テーブル名をコピー"),
        help_line("r", "スキーマ再読み込み"),
        help_line("/", "テーブル名検索（Enter 確定 / Esc 取消）"),
        help_line("n / N", "次/前の一致へ移動"),
        Line::raw(""),
        section("Editor Normal"),
        help_line("i/a/A/o/O", "Insert モードに入る"),
        help_line("w/b/e", "単語移動"),
        help_line("^", "行の最初の非空白"),
        help_line("x/dd/D/C", "削除"),
        help_line("u/Ctrl+R", "undo/redo"),
        help_line("=", "クエリ全体をフォーマット"),
        Line::raw(""),
        section("Editor Insert"),
        help_line("Esc", "Normal モードに戻る"),
        Line::raw(""),
        section("Results"),
        help_line("y", "行データコピー"),
        help_line("cc", "カーソル行の UPDATE 文を Editor に追記"),
        Line::raw(""),
        section("Help (このポップアップ内)"),
        help_line("j/k/↑↓", "1行スクロール"),
        help_line("PgDn/PgUp", "ページスクロール"),
        help_line("Ctrl+D/Ctrl+U", "ページスクロール"),
        help_line("g / G", "先頭 / 末尾へ"),
        help_line("Esc / ? / q", "ヘルプを閉じる"),
    ]
}

fn section(label: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        label,
        Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
    ))
}

fn help_line(key: &'static str, desc: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{:<16}", key), Style::default().fg(Color::Yellow)),
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
