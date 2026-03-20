pub mod editor;
pub mod help;
pub mod layout;
pub mod picker;
pub mod results;
pub mod schema;

use crate::config::config::AppConfig;
use crate::config::connections::ConnectionConfig;
use crate::db::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};
use crate::db::postgres::PostgresAdapter;
use crate::db::{LimitApplier, ReadonlyChecker};
use crate::export::{self, ExportFormat};
use crate::history::{HistoryEntry, HistoryStore};
use crate::tunnel::Tunnel;
use anyhow::Result;
use crossterm::{
    event::{Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use editor::EditorState;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame, Terminal,
};
use results::ResultsState;
use schema::{ColumnEntry, SchemaState, TableEntry};
use std::io;
use std::path::PathBuf;
use tokio::sync::mpsc;

// ── 状態定義 ──

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    ConnectionPicker,
    NewConnectionWizard,
    HistoryPicker,
    ExportFormatPicker,
    ExportPathInput,
    Help,
}

#[derive(Debug, Clone)]
pub struct NewConnectionForm {
    pub fields: Vec<(&'static str, String)>,
    pub cursor: usize,
}

impl NewConnectionForm {
    pub fn new() -> Self {
        Self {
            fields: vec![
                ("name", String::new()),
                ("host", "localhost".to_string()),
                ("port", "5432".to_string()),
                ("database", String::new()),
                ("user", "postgres".to_string()),
                ("password", String::new()),
            ],
            cursor: 0,
        }
    }

    pub fn current_value_mut(&mut self) -> &mut String {
        &mut self.fields[self.cursor].1
    }

    fn get(&self, key: &str) -> &str {
        self.fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str()).unwrap_or("")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Panel {
    Schema,
    Editor,
    Results,
}

impl Panel {
    fn next(self) -> Self {
        match self {
            Panel::Schema => Panel::Editor,
            Panel::Editor => Panel::Results,
            Panel::Results => Panel::Schema,
        }
    }

    fn prev(self) -> Self {
        match self {
            Panel::Schema => Panel::Results,
            Panel::Editor => Panel::Schema,
            Panel::Results => Panel::Editor,
        }
    }
}

pub struct ActiveConnectionInfo {
    pub name: String,
    pub label: Option<String>,
    pub readonly: bool,
}

pub enum AppEvent {
    Key(KeyEvent),
    Tick,
    TunnelReady(Box<(Result<Tunnel>, ConnectionConfig)>),
    TablesLoaded(Result<Vec<TableInfo>>),
    ColumnsLoaded(String, Result<Vec<ColumnInfo>>),
    QueryCompleted(Result<QueryResult>, bool, String), // (result, auto_limited, original_query)
    ExportCompleted(Result<PathBuf>),
}

pub struct App {
    pub mode: AppMode,
    pub active_panel: Panel,
    pub connections: Vec<ConnectionConfig>,
    pub active_connection: Option<ActiveConnectionInfo>,
    pub picker_cursor: usize,
    pub schema: SchemaState,
    pub editor: EditorState,
    pub results: ResultsState,
    pub status_message: Option<String>,
    pub config: AppConfig,
    pub tx: mpsc::Sender<AppEvent>,
    // Phase 5: 履歴
    pub history_store: HistoryStore,
    pub history_entries: Vec<HistoryEntry>,
    pub history_filter: String,
    pub history_cursor: usize,
    // Phase 5: エクスポート
    pub export_format: Option<ExportFormat>,
    pub export_path_input: String,
    pub export_cursor: usize,
    // Phase 6: トンネル
    pub active_tunnel: Option<Tunnel>,
    // Phase 7: スピナー
    pub spinner_frame: usize,
    // Phase 9: 新規接続ウィザード
    pub new_conn_form: NewConnectionForm,
    // 接続時に解決済みのパスワード
    pub resolved_password: Option<String>,
}

impl App {
    pub fn new(connections: Vec<ConnectionConfig>, config: AppConfig, tx: mpsc::Sender<AppEvent>) -> Self {
        Self {
            mode: AppMode::ConnectionPicker,
            active_panel: Panel::Editor,
            connections,
            active_connection: None,
            picker_cursor: 0,
            schema: SchemaState::new(),
            editor: EditorState::new(),
            results: ResultsState::new(),
            status_message: None,
            config,
            tx,
            history_store: HistoryStore::new(),
            history_entries: Vec::new(),
            history_filter: String::new(),
            history_cursor: 0,
            export_format: None,
            export_path_input: String::new(),
            export_cursor: 0,
            active_tunnel: None,
            spinner_frame: 0,
            new_conn_form: NewConnectionForm::new(),
            resolved_password: None,
        }
    }

    pub fn handle_event(&mut self, event: AppEvent) -> std::ops::ControlFlow<()> {
        match event {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::Tick => {
                self.schema.tick();
                if self.editor.executing || self.schema.loading {
                    self.spinner_frame = (self.spinner_frame + 1) % 10;
                }
                std::ops::ControlFlow::Continue(())
            }
            AppEvent::TunnelReady(boxed) => {
                let (result, conn) = *boxed;
                match result {
                    Ok(tunnel) => {
                        self.active_tunnel = Some(tunnel);
                        self.status_message = Some(format!("トンネル確立: {}", conn.name()));
                        // トンネル経由でスキーマ取得
                        spawn_fetch_tables(&conn, self.resolved_password.clone(), self.tx.clone());
                    }
                    Err(e) => {
                        self.status_message = Some(format!("トンネルエラー: {}", e));
                        self.schema.loading = false;
                    }
                }
                std::ops::ControlFlow::Continue(())
            }
            AppEvent::TablesLoaded(result) => {
                self.schema.loading = false;
                match result {
                    Ok(tables) => {
                        self.schema.tables = tables
                            .into_iter()
                            .map(|t| TableEntry {
                                name: t.name,
                                expanded: false,
                                columns: Vec::new(),
                                columns_loaded: false,
                                columns_loading: false,
                            })
                            .collect();
                        self.schema.cursor = 0;
                        let count = self.schema.tables.len();
                        self.status_message = Some(format!("{} テーブルを取得しました", count));
                    }
                    Err(e) => {
                        self.status_message = Some(format!("スキーマ取得エラー: {}", e));
                    }
                }
                std::ops::ControlFlow::Continue(())
            }
            AppEvent::QueryCompleted(result, auto_limited, original_query) => {
                self.editor.executing = false;
                match result {
                    Ok(qr) => {
                        let row_count = qr.rows.len();
                        let duration = qr.duration_ms;
                        // 履歴に保存（セミコロン付き）
                        if let Some(ref conn) = self.active_connection {
                            let query_with_semi = if original_query.trim_end().ends_with(';') {
                                original_query.clone()
                            } else {
                                format!("{};", original_query)
                            };
                            let _ = self.history_store.append(
                                &query_with_semi,
                                &conn.name,
                                row_count,
                                duration,
                            );
                        }
                        self.results.set_result(qr, auto_limited);
                        self.status_message = Some(format!(
                            "{} rows  ({:.3}s)",
                            row_count,
                            duration as f64 / 1000.0
                        ));
                    }
                    Err(e) => {
                        let msg = format!("{}", e);
                        self.results.set_error(msg.clone());
                        self.status_message = Some(format!("エラー: {}", msg));
                    }
                }
                std::ops::ControlFlow::Continue(())
            }
            AppEvent::ExportCompleted(result) => {
                match result {
                    Ok(path) => {
                        self.status_message = Some(format!("エクスポート完了: {}", path.display()));
                    }
                    Err(e) => {
                        self.status_message = Some(format!("エクスポートエラー: {}", e));
                    }
                }
                std::ops::ControlFlow::Continue(())
            }
            AppEvent::ColumnsLoaded(table_name, result) => {
                if let Some(table) = self.schema.tables.iter_mut().find(|t| t.name == table_name) {
                    table.columns_loading = false;
                    match result {
                        Ok(columns) => {
                            table.columns = columns
                                .into_iter()
                                .map(|c| ColumnEntry {
                                    name: c.name,
                                    col_type: c.col_type,
                                })
                                .collect();
                            table.columns_loaded = true;
                        }
                        Err(e) => {
                            self.status_message = Some(format!("カラム取得エラー: {}", e));
                        }
                    }
                }
                std::ops::ControlFlow::Continue(())
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> std::ops::ControlFlow<()> {
        // Ctrl+Q: 終了（全モード共通）
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
            return std::ops::ControlFlow::Break(());
        }

        match &self.mode {
            AppMode::ConnectionPicker => self.handle_picker_key(key),
            AppMode::NewConnectionWizard => self.handle_new_conn_key(key),
            AppMode::HistoryPicker => self.handle_history_picker_key(key),
            AppMode::ExportFormatPicker => self.handle_export_format_key(key),
            AppMode::ExportPathInput => self.handle_export_path_key(key),
            AppMode::Help => self.handle_help_key(key),
            AppMode::Normal => self.handle_normal_key(key),
        }
    }

    fn handle_picker_key(&mut self, key: KeyEvent) -> std::ops::ControlFlow<()> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let total = self.connections.len() + 1; // +1 for "New Connection"
                self.picker_cursor = (self.picker_cursor + 1) % total;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let total = self.connections.len() + 1;
                self.picker_cursor = if self.picker_cursor == 0 {
                    total - 1
                } else {
                    self.picker_cursor - 1
                };
            }
            KeyCode::Enter => {
                // "+ New Connection" が選択された場合
                if self.picker_cursor == self.connections.len() {
                    self.new_conn_form = NewConnectionForm::new();
                    self.mode = AppMode::NewConnectionWizard;
                    return std::ops::ControlFlow::Continue(());
                }
                if let Some(conn) = self.connections.get(self.picker_cursor).cloned() {
                    // パスワード解決（prompt の場合は raw mode を一時的に抜ける）
                    let password = match conn.resolve_password() {
                        Ok(pw) => pw,
                        Err(e) => {
                            self.status_message = Some(format!("パスワードエラー: {}", e));
                            return std::ops::ControlFlow::Continue(());
                        }
                    };
                    self.resolved_password = password;

                    // 前のトンネルを破棄
                    if let Some(mut tunnel) = self.active_tunnel.take() {
                        tokio::spawn(async move {
                            tunnel.kill().await;
                        });
                    }
                    self.active_connection = Some(ActiveConnectionInfo {
                        name: conn.name().to_string(),
                        label: conn.label().map(String::from),
                        readonly: conn.is_readonly(),
                    });
                    self.mode = AppMode::Normal;
                    self.schema = SchemaState::new();
                    self.schema.loading = true;
                    match &conn {
                        ConnectionConfig::Direct(_) => {
                            self.status_message = Some(format!("接続中: {}...", conn.name()));
                            spawn_fetch_tables(&conn, self.resolved_password.clone(), self.tx.clone());
                        }
                        ConnectionConfig::Ssh(_) | ConnectionConfig::Ssm(_) => {
                            self.status_message = Some(format!("トンネル確立中: {}...", conn.name()));
                            spawn_tunnel(conn, self.tx.clone());
                        }
                    }
                }
            }
            KeyCode::Esc => {
                if self.active_connection.is_some() {
                    self.mode = AppMode::Normal;
                }
            }
            _ => {}
        }
        std::ops::ControlFlow::Continue(())
    }

    fn handle_help_key(&mut self, key: KeyEvent) -> std::ops::ControlFlow<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                self.mode = AppMode::Normal;
            }
            _ => {}
        }
        std::ops::ControlFlow::Continue(())
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> std::ops::ControlFlow<()> {
        // Ctrl+C: 接続切り替え
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.mode = AppMode::ConnectionPicker;
            return std::ops::ControlFlow::Continue(());
        }

        // Ctrl+H: 履歴ピッカー
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('h') {
            self.open_history_picker();
            return std::ops::ControlFlow::Continue(());
        }

        // Ctrl+X: エクスポート
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('x') {
            self.open_export_picker();
            return std::ops::ControlFlow::Continue(());
        }

        // Ctrl+E: クエリ実行（全パネル共通）
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('e') {
            self.execute_query();
            return std::ops::ControlFlow::Continue(());
        }

        // Ctrl+D / Ctrl+U: Results 縦ページ移動
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('d') {
            if self.active_panel == Panel::Results {
                self.results.page_down(20);
            }
            return std::ops::ControlFlow::Continue(());
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('u') {
            if self.active_panel == Panel::Results {
                self.results.page_up(20);
            }
            return std::ops::ControlFlow::Continue(());
        }

        // Ctrl+R: redo（Editor Normal モードのみ）
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('r') {
            if self.active_panel == Panel::Editor
                && self.editor.mode == editor::EditorMode::Normal
            {
                self.editor.redo();
            }
            return std::ops::ControlFlow::Continue(());
        }

        match key.code {
            KeyCode::Tab if self.active_panel == Panel::Editor && self.editor.completion.active => {
                // プルダウン表示中は Editor に委譲
                self.handle_editor_key(key);
            }
            KeyCode::Tab => {
                self.active_panel = self.active_panel.next();
            }
            KeyCode::BackTab if self.active_panel == Panel::Editor && self.editor.completion.active => {
                self.handle_editor_key(key);
            }
            KeyCode::BackTab => {
                self.active_panel = self.active_panel.prev();
            }
            KeyCode::Char('?') if !(self.active_panel == Panel::Editor && self.editor.mode == editor::EditorMode::Insert) => {
                self.mode = AppMode::Help;
            }
            _ => {
                match self.active_panel {
                    Panel::Schema => self.handle_schema_key(key),
                    Panel::Editor => self.handle_editor_key(key),
                    Panel::Results => self.handle_results_key(key),
                }
            }
        }
        std::ops::ControlFlow::Continue(())
    }

    fn handle_schema_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.schema.move_down();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.schema.move_up();
            }
            KeyCode::Enter => {
                if let Some(result) = self.schema.toggle_expand() {
                    match result {
                        schema::ToggleResult::NeedFetchColumns(table_name) => {
                            if let Some(conn) = self.connections.get(self.picker_cursor).cloned() {
                                spawn_fetch_columns(&conn, &table_name, self.resolved_password.clone(), self.tx.clone());
                            }
                        }
                    }
                }
            }
            KeyCode::Char('s') => {
                if let Some(name) = self.schema.current_table_name() {
                    let quoted = quote_identifier(&name);
                    let query = format!("SELECT * FROM {} LIMIT 100;", quoted);
                    self.editor.set_content(&query);
                    self.active_panel = Panel::Editor;
                    self.execute_query();
                }
            }
            KeyCode::Char('y') => {
                if let Some(name) = self.schema.current_table_name() {
                    match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(&name)) {
                        Ok(_) => {
                            self.status_message = Some(format!("コピー: {}", name));
                        }
                        Err(e) => {
                            self.status_message = Some(format!("コピー失敗: {}", e));
                        }
                    }
                }
            }
            KeyCode::Char('r') => {
                if let Some(conn) = self.connections.get(self.picker_cursor).cloned() {
                    self.schema = SchemaState::new();
                    self.schema.loading = true;
                    self.status_message = Some("スキーマ再読み込み中...".to_string());
                    spawn_fetch_tables(&conn, self.resolved_password.clone(), self.tx.clone());
                }
            }
            _ => {}
        }
    }

    fn handle_editor_key(&mut self, key: KeyEvent) {
        match self.editor.mode {
            editor::EditorMode::Normal => self.handle_editor_normal_key(key),
            editor::EditorMode::Insert => self.handle_editor_insert_key(key),
        }
    }

    fn handle_editor_normal_key(&mut self, key: KeyEvent) {
        // pending_g の処理: 前回 g が押されていたら gg として処理
        if self.editor.pending_g {
            self.editor.pending_g = false;
            if key.code == KeyCode::Char('g') {
                self.editor.move_to_top();
                return;
            }
            // g + 他のキーは無視して通常処理に fallthrough
        }

        match key.code {
            // Insert モード遷移
            KeyCode::Char('i') => self.editor.enter_insert(),
            KeyCode::Char('a') => self.editor.enter_insert_after(),
            KeyCode::Char('A') => self.editor.enter_insert_end(),
            KeyCode::Char('o') => self.editor.enter_insert_below(),
            KeyCode::Char('O') => self.editor.enter_insert_above(),
            // カーソル移動
            KeyCode::Char('h') | KeyCode::Left => self.editor.move_left(),
            KeyCode::Char('l') | KeyCode::Right => self.editor.move_right(),
            KeyCode::Char('j') | KeyCode::Down => self.editor.move_down(),
            KeyCode::Char('k') | KeyCode::Up => self.editor.move_up(),
            KeyCode::Char('w') => self.editor.move_word_forward(),
            KeyCode::Char('b') => self.editor.move_word_back(),
            KeyCode::Char('e') => self.editor.move_word_end(),
            KeyCode::Char('0') | KeyCode::Home => self.editor.move_home(),
            KeyCode::Char('$') | KeyCode::End => self.editor.move_end(),
            KeyCode::Char('^') => self.editor.move_first_non_blank(),
            KeyCode::Char('g') => {
                self.editor.pending_g = true;
            }
            KeyCode::Char('G') => self.editor.move_to_bottom(),
            // 編集
            KeyCode::Char('x') => self.editor.delete_char_at_cursor(),
            KeyCode::Char('d') => {
                // dd: 行削除（簡易実装: d を押したら即行削除）
                self.editor.delete_line();
            }
            KeyCode::Char('D') => self.editor.delete_to_end(),
            KeyCode::Char('C') => self.editor.change_to_end(),
            KeyCode::Char('u') => self.editor.undo(),
            _ => {}
        }
    }

    fn handle_editor_insert_key(&mut self, key: KeyEvent) {
        // プルダウン表示中の特殊キー
        if self.editor.completion.active {
            match key.code {
                KeyCode::Tab | KeyCode::Down => {
                    self.editor.completion.next();
                    return;
                }
                KeyCode::BackTab | KeyCode::Up => {
                    self.editor.completion.prev();
                    return;
                }
                KeyCode::Enter => {
                    self.editor.accept_completion();
                    return;
                }
                KeyCode::Esc => {
                    self.editor.completion.close();
                    return; // Normal には戻さない
                }
                _ => {
                    // 他のキーは通常処理に fallthrough（プルダウンは更新される）
                }
            }
        }

        match key.code {
            KeyCode::Esc => self.editor.enter_normal(),
            KeyCode::Char(ch) => self.editor.insert_char(ch),
            KeyCode::Enter => self.editor.insert_newline(),
            KeyCode::Backspace => self.editor.backspace(),
            KeyCode::Delete => self.editor.delete(),
            KeyCode::Left => self.editor.move_left(),
            KeyCode::Right => self.editor.move_right(),
            KeyCode::Up => self.editor.move_up(),
            KeyCode::Down => self.editor.move_down(),
            KeyCode::Home => self.editor.move_home(),
            KeyCode::End => self.editor.move_end(),
            _ => {}
        }

        // サジェスト更新（文字入力・削除後）
        self.update_editor_completion();
    }

    fn update_editor_completion(&mut self) {
        let table_names: Vec<String> = self.schema.tables.iter().map(|t| t.name.clone()).collect();
        let table_columns: Vec<(String, Vec<String>)> = self
            .schema
            .tables
            .iter()
            .filter(|t| t.columns_loaded)
            .map(|t| {
                (
                    t.name.clone(),
                    t.columns.iter().map(|c| c.name.clone()).collect(),
                )
            })
            .collect();
        self.editor
            .update_completion(&table_names, &table_columns);
    }

    fn handle_results_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.results.scroll_down();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.results.scroll_up();
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.results.scroll_right(4);
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.results.scroll_left(4);
            }
            KeyCode::Char('g') => {
                self.results.scroll_to_top();
            }
            KeyCode::Char('G') => {
                self.results.scroll_to_bottom();
            }
            KeyCode::Home | KeyCode::Char('0') => {
                self.results.h_scroll_home();
            }
            KeyCode::End | KeyCode::Char('$') => {
                self.results.h_scroll_end();
            }
            KeyCode::PageDown => {
                self.results.page_down(20);
            }
            KeyCode::PageUp => {
                self.results.page_up(20);
            }
            KeyCode::Char('L') => {
                self.results.h_page_right();
            }
            KeyCode::Char('H') => {
                self.results.h_page_left();
            }
            KeyCode::Char('y') => {
                if let Some(csv) = self.results.copy_current_row() {
                    match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(&csv)) {
                        Ok(_) => {
                            self.status_message = Some("行データをコピーしました".to_string());
                        }
                        Err(e) => {
                            self.status_message = Some(format!("コピー失敗: {}", e));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_new_conn_key(&mut self, key: KeyEvent) -> std::ops::ControlFlow<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::ConnectionPicker;
            }
            KeyCode::Tab | KeyCode::Down => {
                self.new_conn_form.cursor =
                    (self.new_conn_form.cursor + 1) % self.new_conn_form.fields.len();
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.new_conn_form.cursor = if self.new_conn_form.cursor == 0 {
                    self.new_conn_form.fields.len() - 1
                } else {
                    self.new_conn_form.cursor - 1
                };
            }
            KeyCode::Char(ch) => {
                self.new_conn_form.current_value_mut().push(ch);
            }
            KeyCode::Backspace => {
                self.new_conn_form.current_value_mut().pop();
            }
            KeyCode::Enter => {
                // 必須フィールドチェック
                let name = self.new_conn_form.get("name").to_string();
                let host = self.new_conn_form.get("host").to_string();
                let database = self.new_conn_form.get("database").to_string();
                let user = self.new_conn_form.get("user").to_string();

                if name.is_empty() || database.is_empty() {
                    self.status_message = Some("name と database は必須です".to_string());
                    return std::ops::ControlFlow::Continue(());
                }

                let port: u16 = self.new_conn_form.get("port").parse().unwrap_or(5432);
                let password_raw = self.new_conn_form.get("password").to_string();
                let password = if password_raw.is_empty() {
                    None
                } else {
                    Some(password_raw)
                };

                use crate::config::connections::{DirectConfig, save_connection};

                let conn = ConnectionConfig::Direct(DirectConfig {
                    name: name.clone(),
                    label: None,
                    readonly: false,
                    host,
                    port,
                    database,
                    user,
                    password,
                });

                // connections.yml に保存
                if let Err(e) = save_connection(&conn) {
                    self.status_message = Some(format!("保存エラー: {}", e));
                    return std::ops::ControlFlow::Continue(());
                }

                // connections に追加して即接続
                self.connections.push(conn.clone());
                self.picker_cursor = self.connections.len() - 1;
                self.resolved_password = conn.resolve_password().ok().flatten();
                self.active_connection = Some(ActiveConnectionInfo {
                    name: conn.name().to_string(),
                    label: conn.label().map(String::from),
                    readonly: conn.is_readonly(),
                });
                self.mode = AppMode::Normal;
                self.schema = SchemaState::new();
                self.schema.loading = true;
                self.status_message = Some(format!("接続中: {}...", conn.name()));
                spawn_fetch_tables(&conn, self.resolved_password.clone(), self.tx.clone());
            }
            _ => {}
        }
        std::ops::ControlFlow::Continue(())
    }

    fn open_history_picker(&mut self) {
        self.history_filter.clear();
        self.history_cursor = 0;
        self.history_entries = self.history_store.search("").unwrap_or_default();
        self.mode = AppMode::HistoryPicker;
    }

    fn refresh_history_filter(&mut self) {
        self.history_entries = self
            .history_store
            .search(&self.history_filter)
            .unwrap_or_default();
        self.history_cursor = 0;
    }

    fn handle_history_picker_key(&mut self, key: KeyEvent) -> std::ops::ControlFlow<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
            }
            KeyCode::Down | KeyCode::Tab => {
                if !self.history_entries.is_empty() {
                    self.history_cursor = (self.history_cursor + 1) % self.history_entries.len();
                }
            }
            KeyCode::Up | KeyCode::BackTab => {
                if !self.history_entries.is_empty() {
                    self.history_cursor = if self.history_cursor == 0 {
                        self.history_entries.len() - 1
                    } else {
                        self.history_cursor - 1
                    };
                }
            }
            KeyCode::Enter => {
                if let Some(entry) = self.history_entries.get(self.history_cursor) {
                    self.editor.set_content(&entry.query);
                    self.mode = AppMode::Normal;
                    self.status_message = Some("履歴からクエリを挿入しました".to_string());
                }
            }
            KeyCode::Backspace => {
                self.history_filter.pop();
                self.refresh_history_filter();
            }
            KeyCode::Char(ch) => {
                self.history_filter.push(ch);
                self.refresh_history_filter();
            }
            _ => {}
        }
        std::ops::ControlFlow::Continue(())
    }

    fn open_export_picker(&mut self) {
        if self.results.result.is_none() {
            self.status_message = Some("エクスポートするクエリ結果がありません".to_string());
            return;
        }
        self.export_cursor = 0;
        self.mode = AppMode::ExportFormatPicker;
    }

    fn handle_export_format_key(&mut self, key: KeyEvent) -> std::ops::ControlFlow<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.export_cursor = (self.export_cursor + 1) % 2;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.export_cursor = if self.export_cursor == 0 { 1 } else { 0 };
            }
            KeyCode::Enter => {
                let format = if self.export_cursor == 0 {
                    ExportFormat::Csv
                } else {
                    ExportFormat::Json
                };
                self.export_format = Some(format);
                let default_path = dirs::download_dir()
                    .unwrap_or_else(|| PathBuf::from("~/Downloads"))
                    .join(format!("query_result.{}", format.extension()));
                self.export_path_input = default_path.to_string_lossy().to_string();
                self.mode = AppMode::ExportPathInput;
            }
            _ => {}
        }
        std::ops::ControlFlow::Continue(())
    }

    fn handle_export_path_key(&mut self, key: KeyEvent) -> std::ops::ControlFlow<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
            }
            KeyCode::Enter => {
                if let (Some(ref qr), Some(format)) = (&self.results.result, self.export_format) {
                    let path = PathBuf::from(&self.export_path_input);
                    let qr_clone = qr.clone();
                    let tx = self.tx.clone();
                    tokio::spawn(async move {
                        let result = export::export_to_file(&qr_clone, &path, format)
                            .map(|_| path);
                        let _ = tx.send(AppEvent::ExportCompleted(result)).await;
                    });
                    self.status_message = Some("エクスポート中...".to_string());
                }
                self.mode = AppMode::Normal;
            }
            KeyCode::Backspace => {
                self.export_path_input.pop();
            }
            KeyCode::Char(ch) => {
                self.export_path_input.push(ch);
            }
            _ => {}
        }
        std::ops::ControlFlow::Continue(())
    }

    fn execute_query(&mut self) {
        if self.editor.executing {
            return;
        }

        let query = match self.editor.get_query_at_cursor() {
            Some(q) => q,
            None => {
                self.status_message = Some("実行するクエリがありません".to_string());
                return;
            }
        };

        // readonly チェック
        if let Some(ref conn_info) = self.active_connection {
            if conn_info.readonly {
                if let Err(e) = ReadonlyChecker.check(&query) {
                    self.results.set_error(format!("{}", e));
                    self.status_message = Some(format!("{}", e));
                    return;
                }
            }
        }

        // LIMIT 付与
        let applier = LimitApplier {
            default_limit: self.config.default_limit,
        };
        let (final_query, auto_limited) = applier.apply(&query);

        // 接続設定取得
        if let Some(conn) = self.connections.get(self.picker_cursor).cloned() {
            self.editor.executing = true;
            self.status_message = Some("クエリ実行中...".to_string());
            spawn_execute_query(&conn, &final_query, auto_limited, &query, self.resolved_password.clone(), self.tx.clone());
        } else {
            self.status_message = Some("接続が選択されていません".to_string());
        }
    }
}

// ── 非同期スキーマ取得 ──

/// PostgreSQL の識別子をクォートが必要ならダブルクォートで囲む
fn quote_identifier(name: &str) -> String {
    // 小文字英字・数字・アンダースコアのみで構成され、数字で始まらなければクォート不要
    let needs_quote = name.is_empty()
        || name.chars().next().is_none_or(|c| !c.is_ascii_lowercase() && c != '_')
        || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if needs_quote {
        format!("\"{}\"", name.replace('"', "\"\""))
    } else {
        name.to_string()
    }
}

fn build_adapter(conn: &ConnectionConfig, password: Option<String>) -> Option<PostgresAdapter> {
    match conn {
        ConnectionConfig::Direct(c) => Some(PostgresAdapter::new(
            c.host.clone(),
            c.port,
            c.database.clone(),
            c.user.clone(),
            password,
        )),
        ConnectionConfig::Ssh(c) => Some(PostgresAdapter::new(
            "127.0.0.1".to_string(),
            c.local_port,
            c.database.clone(),
            c.user.clone(),
            password,
        )),
        ConnectionConfig::Ssm(c) => Some(PostgresAdapter::new(
            "127.0.0.1".to_string(),
            c.local_port,
            c.database.clone(),
            c.user.clone(),
            password,
        )),
    }
}

fn spawn_tunnel(conn: ConnectionConfig, tx: mpsc::Sender<AppEvent>) {
    tokio::spawn(async move {
        let result = match &conn {
            ConnectionConfig::Ssh(c) => {
                crate::tunnel::ssh::SshTunnel::start(
                    &c.ssh_host,
                    c.ssh_user.as_deref(),
                    &c.remote_db_host,
                    c.remote_db_port,
                    c.local_port,
                )
                .await
                .map(Tunnel::Ssh)
            }
            ConnectionConfig::Ssm(c) => {
                crate::tunnel::ssm::SsmTunnel::start(
                    &c.instance_id,
                    &c.ssh_user,
                    c.ssh_key.as_deref(),
                    c.aws_profile.as_deref(),
                    &c.remote_db_host,
                    c.remote_db_port,
                    c.local_port,
                )
                .await
                .map(Tunnel::Ssm)
            }
            _ => unreachable!(),
        };
        let _ = tx.send(AppEvent::TunnelReady(Box::new((result, conn)))).await;
    });
}

fn spawn_fetch_tables(conn: &ConnectionConfig, password: Option<String>, tx: mpsc::Sender<AppEvent>) {
    if let Some(adapter) = build_adapter(conn, password) {
        tokio::spawn(async move {
            let result = adapter.fetch_tables().await;
            let _ = tx.send(AppEvent::TablesLoaded(result)).await;
        });
    }
}

fn spawn_fetch_columns(conn: &ConnectionConfig, table_name: &str, password: Option<String>, tx: mpsc::Sender<AppEvent>) {
    if let Some(adapter) = build_adapter(conn, password) {
        let table = table_name.to_string();
        tokio::spawn(async move {
            let result = adapter.fetch_columns(&table).await;
            let _ = tx.send(AppEvent::ColumnsLoaded(table, result)).await;
        });
    }
}

fn spawn_execute_query(
    conn: &ConnectionConfig,
    query: &str,
    auto_limited: bool,
    original_query: &str,
    password: Option<String>,
    tx: mpsc::Sender<AppEvent>,
) {
    if let Some(adapter) = build_adapter(conn, password) {
        let query = query.to_string();
        let original = original_query.to_string();
        tokio::spawn(async move {
            let result = adapter.execute(&query).await;
            let _ = tx.send(AppEvent::QueryCompleted(result, auto_limited, original)).await;
        });
    }
}

// ── 描画 ──

fn render(f: &mut Frame, app: &App) {
    let size = f.area();

    // ヘッダー（1行）+ メインエリア + フッター（1行）
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(size);

    render_header(f, app, outer[0]);
    layout::render_panels(f, app, outer[1]);
    render_footer(f, app, outer[2]);

    // オーバーレイ
    match app.mode {
        AppMode::ConnectionPicker => picker::render(f, app, size),
        AppMode::NewConnectionWizard => picker::render_new_connection(f, app, size),
        AppMode::HistoryPicker => picker::render_history(f, app, size),
        AppMode::ExportFormatPicker => picker::render_export_format(f, app, size),
        AppMode::ExportPathInput => picker::render_export_path(f, app, size),
        AppMode::Help => help::render(f, size),
        _ => {}
    }
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![
        Span::styled(" lazydb ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    ];

    if let Some(ref conn) = app.active_connection {
        spans.push(Span::raw(" ● "));
        spans.push(Span::styled(
            &conn.name,
            Style::default().add_modifier(Modifier::BOLD),
        ));
        if let Some(ref label) = conn.label {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("[{}]", label),
                Style::default().fg(label_color(label)),
            ));
        }
        if conn.readonly {
            spans.push(Span::styled(" [RO]", Style::default().fg(Color::Yellow)));
        }
    } else {
        spans.push(Span::styled(" 未接続", Style::default().fg(Color::DarkGray)));
    }

    // 右端にキーヒント
    let hint = " [?]Help [Ctrl+Q]Quit ";
    let header_content_width: usize = spans.iter().map(|s| s.width()).sum();
    let padding = area.width as usize - header_content_width.min(area.width as usize) - hint.len().min(area.width as usize);
    if padding > 0 {
        spans.push(Span::raw(" ".repeat(padding)));
    }
    spans.push(Span::styled(hint, Style::default().fg(Color::DarkGray)));

    let header = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(Color::DarkGray).fg(Color::White));
    f.render_widget(header, area);
}

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let msg = app.status_message.as_deref().unwrap_or("");
    let (panel_label, mode_span) = match app.active_panel {
        Panel::Schema => ("Schema".to_string(), None),
        Panel::Editor => {
            let mode_str = match app.editor.mode {
                editor::EditorMode::Normal => "NORMAL",
                editor::EditorMode::Insert => "INSERT",
            };
            let mode_color = match app.editor.mode {
                editor::EditorMode::Normal => Color::DarkGray,
                editor::EditorMode::Insert => Color::Green,
            };
            (
                "Editor".to_string(),
                Some(Span::styled(
                    format!(" {} ", mode_str),
                    Style::default().fg(Color::Black).bg(mode_color),
                )),
            )
        }
        Panel::Results => ("Results".to_string(), None),
    };

    let spinner = if app.editor.executing || app.schema.loading {
        format!(" {} ", SPINNER_FRAMES[app.spinner_frame % SPINNER_FRAMES.len()])
    } else {
        String::new()
    };

    let mut spans = vec![
        Span::styled(
            format!(" [{}] ", panel_label),
            Style::default().fg(Color::Cyan),
        ),
    ];
    if let Some(mode) = mode_span {
        spans.push(mode);
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(&spinner, Style::default().fg(Color::Yellow)));
    spans.push(Span::raw(msg));

    let footer = Paragraph::new(Line::from(spans))
    .style(Style::default().bg(Color::DarkGray).fg(Color::White));
    f.render_widget(footer, area);
}

pub fn label_color(label: &str) -> Color {
    match label.to_lowercase().as_str() {
        "local" => Color::Green,
        "dev" => Color::Cyan,
        "stg" => Color::Yellow,
        "prd" | "prod" | "production" => Color::Red,
        _ => Color::White,
    }
}

// ── エントリポイント ──

pub async fn run(connections: Vec<ConnectionConfig>, config: AppConfig, initial_connection: Option<&str>) -> Result<()> {
    // パニック時にターミナルを復元するフック
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // ターミナルセットアップ
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;

    // App + イベントチャネル
    let (tx, mut rx) = mpsc::channel::<AppEvent>(100);
    let mut app = App::new(connections, config.clone(), tx.clone());

    // 初期接続の決定: --connection > config.default_connection > ピッカー表示
    let auto_conn_name = initial_connection
        .map(String::from)
        .or_else(|| {
            if config.auto_connect {
                config.default_connection.clone()
            } else {
                None
            }
        });

    if let Some(ref conn_name) = auto_conn_name {
        if let Some(idx) = app.connections.iter().position(|c| c.name() == conn_name) {
            app.picker_cursor = idx;
            let conn = app.connections[idx].clone();
            app.resolved_password = conn.resolve_password().ok().flatten();
            app.active_connection = Some(ActiveConnectionInfo {
                name: conn.name().to_string(),
                label: conn.label().map(String::from),
                readonly: conn.is_readonly(),
            });
            app.mode = AppMode::Normal;
            app.schema = SchemaState::new();
            app.schema.loading = true;
            match &conn {
                ConnectionConfig::Direct(_) => {
                    app.status_message = Some(format!("接続中: {}...", conn.name()));
                    spawn_fetch_tables(&conn, app.resolved_password.clone(), app.tx.clone());
                }
                ConnectionConfig::Ssh(_) | ConnectionConfig::Ssm(_) => {
                    app.status_message = Some(format!("トンネル確立中: {}...", conn.name()));
                    spawn_tunnel(conn, app.tx.clone());
                }
            }
        }
    }

    // キー入力読み取りタスク（poll + read でブロッキングを回避）
    let key_tx = tx.clone();
    tokio::spawn(async move {
        loop {
            let event = tokio::task::spawn_blocking(|| {
                // 100ms タイムアウトでポーリング → キーがあれば read
                if crossterm::event::poll(std::time::Duration::from_millis(100))
                    .unwrap_or(false)
                {
                    crossterm::event::read().ok()
                } else {
                    None
                }
            })
            .await;
            match event {
                Ok(Some(Event::Key(key))) => {
                    if key_tx.send(AppEvent::Key(key)).await.is_err() {
                        break;
                    }
                }
                Ok(Some(_)) => {} // マウスイベント等は無視
                Ok(None) => {
                    // タイムアウト: チャネルが閉じていたら終了
                    if key_tx.is_closed() {
                        break;
                    }
                }
                _ => break,
            }
        }
    });

    // Tick タスク（スピナー等のアニメーション用）
    let tick_tx = tx;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
        loop {
            interval.tick().await;
            if tick_tx.send(AppEvent::Tick).await.is_err() {
                break;
            }
        }
    });

    // メインループ
    loop {
        // スクロール調整
        let term_size = terminal.size()?;
        let term_height = term_size.height as usize;
        let term_width = term_size.width as usize;
        let editor_height = (term_height.saturating_sub(2) * 65 / 100).saturating_sub(2);
        app.editor.adjust_scroll(editor_height);
        // Results パネルの表示幅を更新（右70% - ボーダー2）
        app.results.visible_width = (term_width * 70 / 100).saturating_sub(2);

        terminal.draw(|f| render(f, &app))?;

        if let Some(event) = rx.recv().await {
            if app.handle_event(event).is_break() {
                break;
            }
        } else {
            break;
        }
    }

    // トンネルのクリーンアップ
    if let Some(mut tunnel) = app.active_tunnel.take() {
        tunnel.kill().await;
    }

    // ターミナル復元
    crossterm::terminal::disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;

    Ok(())
}
