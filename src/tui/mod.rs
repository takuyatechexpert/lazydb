pub mod cc_edit;
pub mod editor;
pub mod help;
pub mod layout;
pub mod picker;
pub mod results;
pub mod schema;
pub mod scrollable;

use crate::config::config::AppConfig;
use crate::config::connections::{ConnectionConfig, DbType};
use crate::db::adapter::{ColumnInfo, QueryResult, TableInfo};
use crate::db::mysql::MysqlAdapter;
use crate::db::postgres::PostgresAdapter;
use crate::db::redis::RedisAdapter;
use crate::db::sqlite::SqliteAdapter;
use crate::db::{AnyAdapter, LimitApplier, ReadonlyChecker, RedisReadonlyChecker};
use crate::export::{self, ExportFormat};
use crate::history::{HistoryEntry, HistoryStore};
use crate::session::{ConnectionSession, SessionState, TabSnapshot};
use crate::tunnel::Tunnel;
use anyhow::Result;
use crossterm::{
    event::{
        DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyModifiers,
    },
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
use cc_edit::{CcAnalysis, CcEligibility};
use results::ResultsState;
use schema::{ColumnEntry, SchemaState, TableEntry};
use scrollable::Scrollable;
use std::io;
use std::path::PathBuf;
use tokio::sync::mpsc;

// ── タブ ──

const MAX_TABS: usize = 10;

pub struct Tab {
    pub id: usize,
    pub name: String,
    pub editor: EditorState,
    pub results: ResultsState,
    /// cc 2連打検出: このタブで Results フォーカス中に c が押された直後だけ true
    pub pending_c: bool,
}

impl Tab {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            name: "Query".to_string(),
            editor: EditorState::new(),
            results: ResultsState::new(),
            pending_c: false,
        }
    }
}

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

#[derive(Debug, Clone, PartialEq)]
pub enum ConnFormType {
    Direct,
    Ssh,
    Ssm,
}

impl ConnFormType {
    fn label(&self) -> &'static str {
        match self {
            ConnFormType::Direct => "direct",
            ConnFormType::Ssh => "ssh",
            ConnFormType::Ssm => "ssm",
        }
    }

    fn cycle_next(&self) -> Self {
        match self {
            ConnFormType::Direct => ConnFormType::Ssh,
            ConnFormType::Ssh => ConnFormType::Ssm,
            ConnFormType::Ssm => ConnFormType::Direct,
        }
    }

    fn cycle_prev(&self) -> Self {
        match self {
            ConnFormType::Direct => ConnFormType::Ssm,
            ConnFormType::Ssh => ConnFormType::Direct,
            ConnFormType::Ssm => ConnFormType::Ssh,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DbTypeChoice {
    Pg,
    My,
    /// SQLite はトンネル不要のローカルファイル接続。
    /// 選択時は conn_type が自動的に Direct に固定される。
    Sl,
    /// Redis はネットワーク接続（host/port）だが SQL ではないため、
    /// クエリエディタには Redis コマンドをそのまま入力する。
    Rd,
}

impl DbTypeChoice {
    fn label(&self) -> &'static str {
        match self {
            DbTypeChoice::Pg => "pg",
            DbTypeChoice::My => "my",
            DbTypeChoice::Sl => "sqlite",
            DbTypeChoice::Rd => "redis",
        }
    }

    /// 次の選択肢へ循環（pg → my → sqlite → redis → pg）
    fn cycle_next(&self) -> Self {
        match self {
            DbTypeChoice::Pg => DbTypeChoice::My,
            DbTypeChoice::My => DbTypeChoice::Sl,
            DbTypeChoice::Sl => DbTypeChoice::Rd,
            DbTypeChoice::Rd => DbTypeChoice::Pg,
        }
    }

    fn cycle_prev(&self) -> Self {
        match self {
            DbTypeChoice::Pg => DbTypeChoice::Rd,
            DbTypeChoice::My => DbTypeChoice::Pg,
            DbTypeChoice::Sl => DbTypeChoice::My,
            DbTypeChoice::Rd => DbTypeChoice::Sl,
        }
    }

    fn to_db_type(&self) -> DbType {
        match self {
            DbTypeChoice::Pg => DbType::Postgresql,
            DbTypeChoice::My => DbType::Mysql,
            DbTypeChoice::Sl => DbType::Sqlite,
            DbTypeChoice::Rd => DbType::Redis,
        }
    }

    fn default_port(&self) -> &'static str {
        match self {
            DbTypeChoice::Pg => "5432",
            DbTypeChoice::My => "3306",
            DbTypeChoice::Sl => "",
            DbTypeChoice::Rd => "6379",
        }
    }

    fn default_user(&self) -> &'static str {
        match self {
            DbTypeChoice::Pg => "postgres",
            DbTypeChoice::My => "root",
            DbTypeChoice::Sl => "",
            DbTypeChoice::Rd => "",
        }
    }

    /// SQLite かどうか
    fn is_sqlite(&self) -> bool {
        matches!(self, DbTypeChoice::Sl)
    }
}

/// フォームの利用モード（タイトル表示・保存挙動の分岐用）
#[derive(Debug, Clone, PartialEq)]
pub enum FormMode {
    /// 新規追加
    New,
    /// 既存接続のコピーを基に新規追加
    Duplicate,
    /// 既存接続を上書き編集（usize はピッカー上の対象 index）
    Edit(usize),
}

impl FormMode {
    pub fn title(&self) -> &'static str {
        match self {
            FormMode::New => "New Connection",
            FormMode::Duplicate => "Duplicate Connection",
            FormMode::Edit(_) => "Edit Connection",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewConnectionForm {
    pub conn_type: ConnFormType,
    pub db_type: DbTypeChoice,
    pub fields: Vec<(&'static str, String)>,
    pub cursor: usize,
    pub form_mode: FormMode,
    // cursor == 0 は conn_type 選択行、cursor == 1 は db_type 選択行
    // 実際のフィールドは cursor >= 2
}

impl NewConnectionForm {
    pub fn new() -> Self {
        let mut form = Self {
            conn_type: ConnFormType::Direct,
            db_type: DbTypeChoice::Pg,
            fields: Vec::new(),
            cursor: 0,
            form_mode: FormMode::New,
        };
        form.rebuild_fields();
        form
    }

    /// 既存の ConnectionConfig からフォームを構築する。
    ///
    /// `mode` には FormMode::Edit(index) または FormMode::Duplicate を渡す。
    /// password フィールドは常に空で初期化する（既存値の表示・編集はしない）：
    /// - Edit 時に空のまま保存すると既存設定（`keychain:NAME` 等）を維持する
    /// - 入力すれば新しい値で keychain に上書き保存する
    pub fn from_connection(conn: &ConnectionConfig, mode: FormMode) -> Self {
        use crate::config::connections::ConnectionConfig as CC;

        let conn_type = match conn {
            CC::Direct(_) => ConnFormType::Direct,
            CC::Ssh(_) => ConnFormType::Ssh,
            CC::Ssm(_) => ConnFormType::Ssm,
            // SQLite は SSH/SSM と無縁。conn_type は Direct 扱いとしておき、
            // UI 上は db_type=sqlite により path 専用フォームが描画される。
            CC::Sqlite(_) => ConnFormType::Direct,
        };
        let db_type = match conn.db_type() {
            DbType::Postgresql => DbTypeChoice::Pg,
            DbType::Mysql => DbTypeChoice::My,
            DbType::Sqlite => DbTypeChoice::Sl,
            DbType::Redis => DbTypeChoice::Rd,
        };

        let mut form = Self {
            conn_type,
            db_type,
            fields: Vec::new(),
            cursor: 0,
            form_mode: mode.clone(),
        };
        form.rebuild_fields();

        // 共通フィールドを上書き
        let name_value = match &mode {
            FormMode::Duplicate => format!("{}-copy", conn.name()),
            _ => conn.name().to_string(),
        };
        form.set_field("name", &name_value);
        if let Some(label) = conn.label() {
            form.set_field("label", label);
        }
        form.set_field("readonly", if conn.is_readonly() { "true" } else { "false" });

        match conn {
            CC::Sqlite(c) => {
                form.set_field("path", &c.path);
            }
            CC::Direct(c) => {
                form.set_field("host", &c.host);
                form.set_field("port", &c.port.to_string());
                form.set_field("database", &c.database);
                form.set_field("user", &c.user);
            }
            CC::Ssh(c) => {
                form.set_field("ssh_host", &c.ssh_host);
                if let Some(ref u) = c.ssh_user {
                    form.set_field("ssh_user", u);
                }
                form.set_field("remote_db_host", &c.remote_db_host);
                form.set_field("remote_db_port", &c.remote_db_port.to_string());
                form.set_field("local_port", &c.local_port.to_string());
                form.set_field("database", &c.database);
                form.set_field("user", &c.user);
            }
            CC::Ssm(c) => {
                form.set_field("instance_id", &c.instance_id);
                form.set_field("ssh_user", &c.ssh_user);
                if let Some(ref k) = c.ssh_key {
                    form.set_field("ssh_key", k);
                }
                if let Some(ref p) = c.aws_profile {
                    form.set_field("aws_profile", p);
                }
                form.set_field("remote_db_host", &c.remote_db_host);
                form.set_field("remote_db_port", &c.remote_db_port.to_string());
                form.set_field("local_port", &c.local_port.to_string());
                form.set_field("database", &c.database);
                form.set_field("user", &c.user);
            }
        }

        // password はセキュリティ上、復元しない（空のまま）
        // type / db_type 行をスキップして name フィールドにフォーカス
        form.cursor = 2;
        form
    }

    fn set_field(&mut self, key: &str, value: &str) {
        if let Some((_, v)) = self.fields.iter_mut().find(|(k, _)| *k == key) {
            *v = value.to_string();
        }
    }

    /// conn_type と db_type に基づいてフィールドを再構築する
    fn rebuild_fields(&mut self) {
        // SQLite の場合は conn_type を Direct に固定して、path のみのフォームを構築する。
        // ホスト/ポート/ユーザ/パスワードといった概念がそもそも存在しないため。
        if self.db_type.is_sqlite() {
            self.conn_type = ConnFormType::Direct;
            self.fields = vec![
                ("name", String::new()),
                ("label", String::new()),
                ("path", String::new()),
                ("readonly", "false".to_string()),
            ];
            self.cursor = 0;
            return;
        }

        let port = self.db_type.default_port().to_string();
        let user = self.db_type.default_user().to_string();

        let mut fields: Vec<(&'static str, String)> = vec![
            ("name", String::new()),
            ("label", String::new()),
        ];

        match self.conn_type {
            ConnFormType::Direct => {
                fields.extend([
                    ("host", "localhost".to_string()),
                    ("port", port),
                    ("database", String::new()),
                    ("user", user),
                    ("password", String::new()),
                    ("readonly", "false".to_string()),
                ]);
            }
            ConnFormType::Ssh => {
                fields.extend([
                    ("ssh_host", String::new()),
                    ("ssh_user", String::new()),
                    ("remote_db_host", String::new()),
                    ("remote_db_port", port.clone()),
                    ("local_port", String::new()),
                    ("database", String::new()),
                    ("user", user),
                    ("password", String::new()),
                    ("readonly", "false".to_string()),
                ]);
            }
            ConnFormType::Ssm => {
                fields.extend([
                    ("instance_id", String::new()),
                    ("ssh_user", "ec2-user".to_string()),
                    ("ssh_key", String::new()),
                    ("aws_profile", String::new()),
                    ("remote_db_host", String::new()),
                    ("remote_db_port", port.clone()),
                    ("local_port", String::new()),
                    ("database", String::new()),
                    ("user", user),
                    ("password", String::new()),
                    ("readonly", "false".to_string()),
                ]);
            }
        }

        self.fields = fields;
        // cursor を先頭の type 選択行に戻す
        self.cursor = 0;
    }

    /// 現在のカーソル位置のフィールド値を取得（type/db_type 行の場合は None）
    pub fn current_field_mut(&mut self) -> Option<&mut String> {
        if self.cursor >= 2 {
            let idx = self.cursor - 2;
            self.fields.get_mut(idx).map(|(_, v)| v)
        } else {
            None
        }
    }

    /// 現在のカーソル位置のフィールド名を取得（type/db_type 行の場合は None）
    pub fn current_field_name(&self) -> Option<&'static str> {
        if self.cursor >= 2 {
            let idx = self.cursor - 2;
            self.fields.get(idx).map(|(k, _)| *k)
        } else {
            None
        }
    }

    /// 現在のカーソル位置が bool トグルフィールド（readonly）かどうか
    pub fn is_current_bool_toggle(&self) -> bool {
        matches!(self.current_field_name(), Some("readonly"))
    }

    /// bool トグルフィールドの値を反転する
    pub fn toggle_current_bool(&mut self) {
        if let Some(val) = self.current_field_mut() {
            *val = if val == "true" {
                "false".to_string()
            } else {
                "true".to_string()
            };
        }
    }

    fn get(&self, key: &str) -> &str {
        self.fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str()).unwrap_or("")
    }

    /// 表示行数（type + db_type + フィールド数）
    pub fn total_rows(&self) -> usize {
        2 + self.fields.len()
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
    /// Redis 接続の場合 true（SQL ではないため LIMIT 付与や SQL用 readonly チェックを抑止する）
    pub is_redis: bool,
}

pub enum AppEvent {
    Key(KeyEvent),
    Paste(String),
    Tick,
    TunnelReady(Box<(Result<Tunnel>, ConnectionConfig)>),
    TablesLoaded(Result<Vec<TableInfo>>),
    ColumnsLoaded(String, Result<Vec<ColumnInfo>>),
    QueryCompleted(Result<QueryResult>, bool, String, usize), // (result, auto_limited, original_query, tab_id)
    ExportCompleted(Result<PathBuf>),
}

pub struct App {
    pub mode: AppMode,
    pub active_panel: Panel,
    pub connections: Vec<ConnectionConfig>,
    pub active_connection: Option<ActiveConnectionInfo>,
    pub picker_cursor: usize,
    pub schema: SchemaState,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    pub next_tab_id: usize,
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
    /// vim の `zz` 2 連打検出: 直前のキーが（フォーカス中ペインで）`z` だったときだけ true
    pub pending_z: bool,
    /// ヘルプポップアップの縦スクロールオフセット。
    /// `?` で開くたびに 0 にリセットされ、ポップアップ内の j/k/PgDn/g などで操作する。
    pub help_scroll: u16,
    /// 接続単位で永続化したタブ群のインメモリキャッシュ。
    /// 接続切替時に「直前の接続のタブ群を取り込み → 次の接続のタブ群を展開」する。
    pub session_state: SessionState,
}

impl App {
    /// OS クリップボードへコピー。成功なら Ok、失敗なら arboard のエラーを返す。
    /// fire-and-forget したい場合は `let _ = App::copy_to_clipboard(...)`、
    /// ステータスに反映したい場合は `match App::copy_to_clipboard(...)` で扱う。
    fn copy_to_clipboard(text: &str) -> Result<(), arboard::Error> {
        arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text.to_string()))
    }

    /// `text` を OS クリップボードに転送し、同時に内部レジスタにも保存する。
    /// Visual モードの y/d/c などで「クリップボード反映＋register 更新」を必ずセットで行うための共通ヘルパ。
    fn yank_to_register_and_clipboard(&mut self, text: String, kind: editor::YankKind) {
        let _ = App::copy_to_clipboard(&text);
        let idx = self.active_tab;
        self.tabs[idx].editor.set_register(text, kind);
    }

    pub fn new(connections: Vec<ConnectionConfig>, config: AppConfig, tx: mpsc::Sender<AppEvent>) -> Self {
        Self {
            mode: AppMode::ConnectionPicker,
            active_panel: Panel::Editor,
            connections,
            active_connection: None,
            picker_cursor: 0,
            schema: SchemaState::new(),
            tabs: vec![Tab::new(1)],
            active_tab: 0,
            next_tab_id: 2,
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
            pending_z: false,
            help_scroll: 0,
            session_state: SessionState::default(),
        }
    }

    /// 接続切替直前に呼び出すヘルパ。
    /// 現在のタブ群を `session_state` に取り込んだうえで、新接続のタブ群を展開する。
    /// 新接続に保存済みタブが無い場合は初期状態（Tab 1 つ）に戻す。
    pub fn switch_connection_session(&mut self, prev: Option<&str>, next: &str) {
        if let Some(prev_name) = prev {
            let snap = self.capture_connection_session();
            self.session_state.set(prev_name.to_string(), snap);
        }
        let next_session = self
            .session_state
            .get(next)
            .cloned()
            .unwrap_or_default();
        self.apply_connection_session(next_session);
    }

    // ── タブ操作 ──

    /// 新規タブをアクティブタブの直後に追加。上限到達時は status_message を設定。
    pub fn add_tab(&mut self) {
        if self.tabs.len() >= MAX_TABS {
            self.status_message = Some(format!("タブ上限({})に達しています", MAX_TABS));
            return;
        }
        let new_tab = Tab::new(self.next_tab_id);
        self.next_tab_id += 1;
        let insert_pos = self.active_tab + 1;
        self.tabs.insert(insert_pos, new_tab);
        self.active_tab = insert_pos;
        self.active_panel = Panel::Editor;
    }

    /// アクティブタブを閉じる。最後の1タブは閉じない。
    pub fn close_tab(&mut self) {
        if self.tabs.len() <= 1 {
            return;
        }
        self.tabs.remove(self.active_tab);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
    }

    /// 次のタブへ切り替え（ラップアラウンド）
    pub fn next_tab(&mut self) {
        self.active_tab = (self.active_tab + 1) % self.tabs.len();
    }

    /// 前のタブへ切り替え（ラップアラウンド）
    pub fn prev_tab(&mut self) {
        if self.active_tab == 0 {
            self.active_tab = self.tabs.len() - 1;
        } else {
            self.active_tab -= 1;
        }
    }

    // ── セッション永続化（接続単位） ──

    /// 現在のタブ群を ConnectionSession に書き出す。
    /// editor の本文と論理カーソル位置のみを保存する（結果セットや実行履歴は対象外）。
    pub fn capture_connection_session(&self) -> ConnectionSession {
        let tabs = self
            .tabs
            .iter()
            .map(|t| TabSnapshot {
                name: t.name.clone(),
                content: t.editor.lines.join("\n"),
                cursor_row: t.editor.cursor.0,
                cursor_col: t.editor.cursor.1,
            })
            .collect();
        ConnectionSession {
            tabs,
            active_tab: self.active_tab,
        }
    }

    /// 接続単位の保存タブをエディタへ展開する。
    /// `session.tabs` が空のときは初期状態（Tab 1 つ）にリセットする。
    pub fn apply_connection_session(&mut self, session: ConnectionSession) {
        if session.tabs.is_empty() {
            self.tabs = vec![Tab::new(1)];
            self.active_tab = 0;
            self.next_tab_id = 2;
            return;
        }
        let mut tabs = Vec::with_capacity(session.tabs.len().min(MAX_TABS));
        for (i, snap) in session.tabs.into_iter().take(MAX_TABS).enumerate() {
            let mut tab = Tab::new(i + 1);
            tab.name = snap.name;
            tab.editor.lines = if snap.content.is_empty() {
                vec![String::new()]
            } else {
                snap.content.split('\n').map(String::from).collect()
            };
            let row = snap.cursor_row.min(tab.editor.lines.len().saturating_sub(1));
            let col = snap.cursor_col.min(tab.editor.lines[row].chars().count());
            tab.editor.cursor = (row, col);
            tabs.push(tab);
        }
        self.next_tab_id = tabs.len() + 1;
        self.active_tab = session.active_tab.min(tabs.len().saturating_sub(1));
        self.tabs = tabs;
    }

    // ── 読み取り専用アクセサ ──

    pub fn active_editor(&self) -> &EditorState {
        &self.tabs[self.active_tab].editor
    }

    #[allow(dead_code)]
    pub fn active_results(&self) -> &ResultsState {
        &self.tabs[self.active_tab].results
    }

    pub fn handle_event(&mut self, event: AppEvent) -> std::ops::ControlFlow<()> {
        match event {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::Paste(text) => self.handle_paste(text),
            AppEvent::Tick => {
                self.schema.tick();
                let idx = self.active_tab;
                if self.tabs[idx].editor.executing || self.schema.loading {
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
            AppEvent::QueryCompleted(result, auto_limited, original_query, tab_id) => {
                // tab_id で対象タブを特定（閉じられていたら破棄）
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                    tab.editor.executing = false;
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
                            tab.results.set_result(qr, auto_limited, original_query.clone());
                            let analysis = CcAnalysis::from_query(&original_query);
                            let eligibility = cc_edit::compute_eligibility(&analysis, &self.schema);
                            tab.results.set_cc_eligibility(eligibility.clone());
                            if matches!(eligibility, CcEligibility::ColumnsNotLoaded) {
                                if let Some(table) = analysis.table.clone() {
                                    self.maybe_auto_fetch_columns(&table);
                                }
                            }
                            self.status_message = Some(format!(
                                "{} rows  ({:.3}s)",
                                row_count,
                                duration as f64 / 1000.0
                            ));
                        }
                        Err(e) => {
                            let msg = format!("{}", e);
                            tab.results.set_error(msg.clone());
                            self.status_message = Some(format!("エラー: {}", msg));
                        }
                    }
                }
                // タブが閉じられていた場合は結果を破棄
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
                                    is_primary_key: c.is_primary_key,
                                })
                                .collect();
                            table.columns_loaded = true;
                        }
                        Err(e) => {
                            self.status_message = Some(format!("カラム取得エラー: {}", e));
                        }
                    }
                }
                // schema が更新された可能性があるので、last_query を持つ全タブの cc_eligibility を再計算
                let schema = &self.schema;
                for tab in self.tabs.iter_mut() {
                    if let Some(last_query) = tab.results.last_query.as_deref() {
                        let analysis = CcAnalysis::from_query(last_query);
                        let eligibility = cc_edit::compute_eligibility(&analysis, schema);
                        tab.results.set_cc_eligibility(eligibility);
                    }
                }
                std::ops::ControlFlow::Continue(())
            }
        }
    }

    /// 指定テーブルのカラム情報がロード済みでもロード中でもなければ、
    /// fetch_columns をバックグラウンドで発行する。
    /// - schema.tables に該当テーブルが無い場合は何もしない
    /// - active_connection が無い or ConnectionConfig が取得できない場合は何もしない
    fn maybe_auto_fetch_columns(&mut self, table: &str) {
        let Some(conn) = self.connections.get(self.picker_cursor).cloned() else {
            return;
        };
        let Some(entry) = self
            .schema
            .tables
            .iter_mut()
            .find(|t| t.name.eq_ignore_ascii_case(table))
        else {
            return;
        };
        if entry.columns_loaded || entry.columns_loading {
            return;
        }
        entry.columns_loading = true;
        let table_name = entry.name.clone();
        spawn_fetch_columns(
            &conn,
            &table_name,
            self.resolved_password.clone(),
            self.tx.clone(),
        );
    }

    fn handle_paste(&mut self, text: String) -> std::ops::ControlFlow<()> {
        match self.mode {
            AppMode::Normal => {
                // ペーストはエディタペインがアクティブな時のみ受け付ける
                if self.active_panel != Panel::Editor {
                    return std::ops::ControlFlow::Continue(());
                }
                let idx = self.active_tab;
                // 補完ポップアップは閉じる（ペースト挿入と相性が悪いため）
                self.tabs[idx].editor.completion.close();
                // Insert モードへ遷移してから一括挿入する
                if self.tabs[idx].editor.mode != editor::EditorMode::Insert {
                    self.tabs[idx].editor.enter_insert();
                }
                self.tabs[idx].editor.insert_str(&text);
                self.update_editor_completion();
            }
            AppMode::NewConnectionWizard => {
                // type / db_type / 真偽値トグル行ではペースト無効。
                // それ以外のテキストフィールド（name / host / path / password など）には
                // 制御文字を除去した上で末尾に追記する。
                if self.new_conn_form.cursor < 2 || self.new_conn_form.is_current_bool_toggle() {
                    return std::ops::ControlFlow::Continue(());
                }
                let sanitized: String = text.chars().filter(|c| !c.is_control()).collect();
                if sanitized.is_empty() {
                    return std::ops::ControlFlow::Continue(());
                }
                if let Some(val) = self.new_conn_form.current_field_mut() {
                    val.push_str(&sanitized);
                }
            }
            AppMode::ExportPathInput => {
                // エクスポート先パス入力にもペーストできるように対応
                let sanitized: String = text.chars().filter(|c| !c.is_control()).collect();
                self.export_path_input.push_str(&sanitized);
            }
            AppMode::HistoryPicker => {
                // 履歴フィルタにペーストできるように対応（フィルタは1行のため改行除去）
                let sanitized: String = text.chars().filter(|c| !c.is_control()).collect();
                if !sanitized.is_empty() {
                    self.history_filter.push_str(&sanitized);
                    self.refresh_history_filter();
                }
            }
            _ => {}
        }
        std::ops::ControlFlow::Continue(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> std::ops::ControlFlow<()> {
        // Ctrl+Q: 終了（全モード共通）
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
            return std::ops::ControlFlow::Break(());
        }

        // Normal モード以外では zz チョード状態を保持しない
        if !matches!(self.mode, AppMode::Normal) {
            self.pending_z = false;
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
                    // セッション切り替え: 直前の接続のタブを退避し、新接続のタブを展開する。
                    // 同じ接続を再選択した場合は何もしない（タブをそのまま維持）。
                    let prev_name = self
                        .active_connection
                        .as_ref()
                        .map(|c| c.name.clone());
                    if prev_name.as_deref() != Some(conn.name()) {
                        self.switch_connection_session(prev_name.as_deref(), conn.name());
                    }
                    self.active_connection = Some(ActiveConnectionInfo {
                        name: conn.name().to_string(),
                        label: conn.label().map(String::from),
                        readonly: conn.is_readonly(),
                        is_redis: matches!(conn.db_type(), DbType::Redis),
                    });
                    self.mode = AppMode::Normal;
                    // 結果セットはクエリ再実行が必要なのでクリア（editor は session で復元済み）
                    self.tabs.iter_mut().for_each(|t| t.results.clear());
                    self.schema = SchemaState::new();
                    self.schema.loading = true;
                    match &conn {
                        ConnectionConfig::Direct(_) | ConnectionConfig::Sqlite(_) => {
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
            KeyCode::Char('d') if self.picker_cursor < self.connections.len() => {
                // 選択中の接続を複製してフォームを開く
                if let Some(conn) = self.connections.get(self.picker_cursor) {
                    self.new_conn_form =
                        NewConnectionForm::from_connection(conn, FormMode::Duplicate);
                    self.mode = AppMode::NewConnectionWizard;
                }
            }
            KeyCode::Char('e') if self.picker_cursor < self.connections.len() => {
                // 選択中の接続を編集
                if let Some(conn) = self.connections.get(self.picker_cursor) {
                    self.new_conn_form = NewConnectionForm::from_connection(
                        conn,
                        FormMode::Edit(self.picker_cursor),
                    );
                    self.mode = AppMode::NewConnectionWizard;
                }
            }
            KeyCode::Esc if self.active_connection.is_some() => {
                self.mode = AppMode::Normal;
            }
            _ => {}
        }
        std::ops::ControlFlow::Continue(())
    }

    fn handle_help_key(&mut self, key: KeyEvent) -> std::ops::ControlFlow<()> {
        // ヘルプ本文は help::total_lines() で総行数が分かる。
        // 表示行数（ポップアップ内側高さ）はターミナル依存だが、
        // 上限は描画時に再クランプされるので、ここでは total_lines を上限として扱えば安全。
        let total = help::total_lines();
        let page: u16 = 10;
        match (key.code, key.modifiers.contains(KeyModifiers::CONTROL)) {
            (KeyCode::Esc, _) | (KeyCode::Char('?'), false) | (KeyCode::Char('q'), false) => {
                self.mode = AppMode::Normal;
                self.help_scroll = 0;
            }
            (KeyCode::Char('j'), false) | (KeyCode::Down, _) => {
                self.help_scroll = self.help_scroll.saturating_add(1).min(total);
            }
            (KeyCode::Char('k'), false) | (KeyCode::Up, _) => {
                self.help_scroll = self.help_scroll.saturating_sub(1);
            }
            (KeyCode::PageDown, _) | (KeyCode::Char('d'), true) => {
                self.help_scroll = self.help_scroll.saturating_add(page).min(total);
            }
            (KeyCode::PageUp, _) | (KeyCode::Char('u'), true) => {
                self.help_scroll = self.help_scroll.saturating_sub(page);
            }
            (KeyCode::Char('g'), false) => {
                self.help_scroll = 0;
            }
            (KeyCode::Char('G'), false) => {
                self.help_scroll = total;
            }
            _ => {}
        }
        std::ops::ControlFlow::Continue(())
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> std::ops::ControlFlow<()> {
        let idx = self.active_tab;

        // Schema パネルで `/` 検索入力中はすべてのキーを検索ハンドラに委譲する
        // （Tab/?/Ctrl+* などのグローバルショートカットを横取りされないようにする）
        if self.active_panel == Panel::Schema && self.schema.search_active {
            self.handle_schema_search_input(key);
            return std::ops::ControlFlow::Continue(());
        }

        // Editor パネルで `/` 検索入力中も同様に検索ハンドラへ委譲する
        if self.active_panel == Panel::Editor && self.tabs[idx].editor.search.active {
            self.handle_editor_search_input(key);
            return std::ops::ControlFlow::Continue(());
        }

        // zz チョード: 各ペインのカーソル行を画面中央へ寄せる
        // Editor の Insert/Visual/VisualLine モードでは 'z' は別意味なので除外
        let in_editor_text_input = self.active_panel == Panel::Editor
            && matches!(
                self.tabs[idx].editor.mode,
                editor::EditorMode::Insert
                    | editor::EditorMode::Visual
                    | editor::EditorMode::VisualLine
            );
        if !key.modifiers.contains(KeyModifiers::CONTROL)
            && !in_editor_text_input
            && key.code == KeyCode::Char('z')
        {
            if self.pending_z {
                self.pending_z = false;
                match self.active_panel {
                    Panel::Schema => self.schema.center_on_cursor(20),
                    Panel::Editor => self.tabs[idx].editor.center_on_cursor(20),
                    Panel::Results => self.tabs[idx].results.center_on_cursor(20),
                }
            } else {
                self.pending_z = true;
            }
            return std::ops::ControlFlow::Continue(());
        }
        // z 以外の通常キーは pending_z をリセット（チョード中断）
        self.pending_z = false;

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

        // Ctrl+T: 新規タブ追加
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('t') {
            self.add_tab();
            return std::ops::ControlFlow::Continue(());
        }

        // Ctrl+W: アクティブタブを閉じる
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('w') {
            self.close_tab();
            return std::ops::ControlFlow::Continue(());
        }

        // Ctrl+N: 次のタブへ
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('n') {
            self.next_tab();
            return std::ops::ControlFlow::Continue(());
        }

        // Ctrl+P: 前のタブへ
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
            self.prev_tab();
            return std::ops::ControlFlow::Continue(());
        }

        // Ctrl+R: redo（Editor Normal モードのみ）
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('r') {
            if self.active_panel == Panel::Editor
                && self.tabs[idx].editor.mode == editor::EditorMode::Normal
            {
                self.tabs[idx].editor.redo();
            }
            return std::ops::ControlFlow::Continue(());
        }

        match key.code {
            KeyCode::Tab if self.active_panel == Panel::Editor && self.tabs[idx].editor.completion.active => {
                // プルダウン表示中は Editor に委譲
                self.handle_editor_key(key);
            }
            KeyCode::Tab => {
                self.active_panel = self.active_panel.next();
                self.tabs[self.active_tab].pending_c = false;
            }
            KeyCode::BackTab if self.active_panel == Panel::Editor && self.tabs[idx].editor.completion.active => {
                self.handle_editor_key(key);
            }
            KeyCode::BackTab => {
                self.active_panel = self.active_panel.prev();
                self.tabs[self.active_tab].pending_c = false;
            }
            KeyCode::Char('?')
                if self.active_panel != Panel::Editor
                    || (self.tabs[idx].editor.mode != editor::EditorMode::Insert
                        && !self.tabs[idx].editor.search.active) =>
            {
                self.mode = AppMode::Help;
                self.help_scroll = 0;
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

    /// `/` 検索モード入力中のキーハンドラ。
    /// Enter で確定（クエリ保持）、Esc で取消（クエリ破棄）、Backspace で 1 文字削除、
    /// その他の通常の Char は検索クエリへ追加する。
    fn handle_schema_search_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.schema.cancel_search();
            }
            KeyCode::Enter => {
                self.schema.confirm_search();
            }
            KeyCode::Backspace => {
                if self.schema.search_query.is_empty() {
                    // 空ならそのまま検索モード抜け
                    self.schema.cancel_search();
                } else {
                    self.schema.pop_search_char();
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.schema.push_search_char(ch);
            }
            _ => {}
        }
    }

    fn handle_schema_key(&mut self, key: KeyEvent) {
        // `/`, `n`, `N`, Esc は scrollable に渡す前に確実に拾う
        match key.code {
            KeyCode::Char('/') => {
                self.schema.enter_search();
                return;
            }
            KeyCode::Char('n') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if !self.schema.find_next() && !self.schema.search_query.is_empty() {
                    self.status_message =
                        Some(format!("一致なし: {}", self.schema.search_query));
                }
                return;
            }
            KeyCode::Char('N') => {
                if !self.schema.find_prev() && !self.schema.search_query.is_empty() {
                    self.status_message =
                        Some(format!("一致なし: {}", self.schema.search_query));
                }
                return;
            }
            KeyCode::Esc if !self.schema.search_query.is_empty() => {
                self.schema.search_query.clear();
                return;
            }
            _ => {}
        }
        if scrollable::dispatch_scroll_key(&mut self.schema, &key, 20) {
            return;
        }
        match key.code {
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
                    let db_type = self.connections.get(self.picker_cursor)
                        .map(|c| c.db_type().clone())
                        .unwrap_or_default();
                    let quoted = quote_identifier(&name, &db_type);
                    let query = format!("SELECT * FROM {} LIMIT 100;", quoted);
                    let idx = self.active_tab;
                    self.tabs[idx].editor.set_content(&query);
                    self.active_panel = Panel::Editor;
                    self.execute_query();
                }
            }
            KeyCode::Char('y') => {
                if let Some(name) = self.schema.current_table_name() {
                    match App::copy_to_clipboard(&name) {
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
        let idx = self.active_tab;
        if self.tabs[idx].editor.search.active {
            self.handle_editor_search_input(key);
            return;
        }
        match self.tabs[idx].editor.mode {
            editor::EditorMode::Normal => self.handle_editor_normal_key(key),
            editor::EditorMode::Insert => self.handle_editor_insert_key(key),
            editor::EditorMode::Visual | editor::EditorMode::VisualLine => {
                self.handle_editor_visual_key(key)
            }
        }
    }

    /// Editor の `/` 検索バー入力中のキーハンドラ。
    fn handle_editor_search_input(&mut self, key: KeyEvent) {
        let idx = self.active_tab;
        let editor = &mut self.tabs[idx].editor;
        match key.code {
            KeyCode::Esc => {
                editor.search_cancel();
            }
            KeyCode::Enter => {
                editor.search_confirm();
                if editor.search.matches.is_empty() && !editor.search.query.is_empty() {
                    let q = editor.search.query.clone();
                    self.status_message = Some(format!("一致なし: {}", q));
                }
            }
            KeyCode::Backspace => {
                if editor.search.query.is_empty() {
                    editor.search_cancel();
                } else {
                    editor.search_pop_char();
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.search_push_char(ch);
            }
            _ => {}
        }
    }

    fn handle_editor_normal_key(&mut self, key: KeyEvent) {
        let idx = self.active_tab;
        // 保留中のチョード（r/g/d/c/y/di/ci/yi）を最優先で処理
        let pending = self.tabs[idx].editor.pending_chord;
        if pending != editor::PendingChord::None {
            self.handle_editor_normal_chord(pending, key);
            return;
        }

        // 共通スクロール・画面移動キー（h/j/k/l/g/G/0/$/PageUp/PageDown/Ctrl+D/Ctrl+U/H/L/Home/End/矢印）
        // を先に dispatch する。pending が無い前提なので g 単独はチョード待機に回したい。
        // → g/G は dispatch 任せだと gg のチョードが扱えないため、g 単独だけ手前で捕まえる。
        if let KeyCode::Char('g') = key.code {
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                self.tabs[idx].editor.pending_chord = editor::PendingChord::GotoG;
                return;
            }
        }

        if scrollable::dispatch_scroll_key(&mut self.tabs[idx].editor, &key, 20) {
            return;
        }

        match key.code {
            // Insert モード遷移
            KeyCode::Char('i') => self.tabs[idx].editor.enter_insert(),
            KeyCode::Char('a') => self.tabs[idx].editor.enter_insert_after(),
            KeyCode::Char('A') => self.tabs[idx].editor.enter_insert_end(),
            KeyCode::Char('o') => self.tabs[idx].editor.enter_insert_below(),
            KeyCode::Char('O') => self.tabs[idx].editor.enter_insert_above(),
            // Visual モード
            KeyCode::Char('v') => self.tabs[idx].editor.enter_visual(),
            KeyCode::Char('V') => self.tabs[idx].editor.enter_visual_line(),
            // 単語移動
            KeyCode::Char('w') => self.tabs[idx].editor.move_word_forward(),
            KeyCode::Char('b') => self.tabs[idx].editor.move_word_back(),
            KeyCode::Char('e') => self.tabs[idx].editor.move_word_end(),
            KeyCode::Char('^') => self.tabs[idx].editor.move_first_non_blank(),
            // 編集
            KeyCode::Char('x') => self.tabs[idx].editor.delete_char_at_cursor(),
            KeyCode::Char('s') => self.tabs[idx].editor.substitute_char(),
            KeyCode::Char('S') => self.tabs[idx].editor.substitute_line(),
            KeyCode::Char('J') => self.tabs[idx].editor.join_lines(),
            KeyCode::Char('~') => self.tabs[idx].editor.toggle_case(),
            KeyCode::Char('p') => self.tabs[idx].editor.paste_after(),
            KeyCode::Char('P') => self.tabs[idx].editor.paste_before(),
            // チョード開始
            KeyCode::Char('d') => self.tabs[idx].editor.pending_chord = editor::PendingChord::Operator('d'),
            KeyCode::Char('y') => self.tabs[idx].editor.pending_chord = editor::PendingChord::Operator('y'),
            KeyCode::Char('c') => self.tabs[idx].editor.pending_chord = editor::PendingChord::Operator('c'),
            KeyCode::Char('r') => self.tabs[idx].editor.pending_chord = editor::PendingChord::Replace,
            // 単発の大文字オペレータ
            KeyCode::Char('D') => self.tabs[idx].editor.delete_to_end(),
            KeyCode::Char('C') => self.tabs[idx].editor.change_to_end(),
            KeyCode::Char('Y') => self.tabs[idx].editor.yank_line(),
            KeyCode::Char('u') => self.tabs[idx].editor.undo(),
            // 検索
            KeyCode::Char('/') => self.tabs[idx].editor.search_start(),
            KeyCode::Char('n') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.tabs[idx].editor.search.matches.is_empty()
                    && !self.tabs[idx].editor.search.query.is_empty()
                {
                    let q = self.tabs[idx].editor.search.query.clone();
                    self.status_message = Some(format!("一致なし: {}", q));
                } else {
                    self.tabs[idx].editor.next_match();
                }
            }
            KeyCode::Char('N') => {
                if self.tabs[idx].editor.search.matches.is_empty()
                    && !self.tabs[idx].editor.search.query.is_empty()
                {
                    let q = self.tabs[idx].editor.search.query.clone();
                    self.status_message = Some(format!("一致なし: {}", q));
                } else {
                    self.tabs[idx].editor.prev_match();
                }
            }
            // バッファ全体をフォーマット
            KeyCode::Char('=') => {
                if self.tabs[idx].editor.format_buffer() {
                    self.status_message = Some("クエリをフォーマットしました".to_string());
                } else {
                    self.status_message = Some("フォーマット対象がありません".to_string());
                }
            }
            KeyCode::Esc => {
                // 保留中のチョードをキャンセル
                self.tabs[idx].editor.pending_chord = editor::PendingChord::None;
            }
            _ => {}
        }
    }

    /// PendingChord 状態で次のキーを処理する。完了したらチョードを None にクリアする。
    fn handle_editor_normal_chord(&mut self, pending: editor::PendingChord, key: KeyEvent) {
        let idx = self.active_tab;
        let editor = &mut self.tabs[idx].editor;

        match pending {
            editor::PendingChord::None => {}
            // r{char}: 1文字置換
            editor::PendingChord::Replace => {
                if let KeyCode::Char(ch) = key.code {
                    editor.replace_char(ch);
                }
                editor.pending_chord = editor::PendingChord::None;
            }
            // gg
            editor::PendingChord::GotoG => {
                if let KeyCode::Char('g') = key.code {
                    editor.move_to_top();
                }
                editor.pending_chord = editor::PendingChord::None;
            }
            // d / y / c の 2 段目
            editor::PendingChord::Operator(op) => {
                match key.code {
                    KeyCode::Char(c) if c == op => {
                        // dd / yy / cc
                        match op {
                            'd' => editor.delete_line_yank(),
                            'y' => editor.yank_line(),
                            'c' => editor.substitute_line(),
                            _ => {}
                        }
                        editor.pending_chord = editor::PendingChord::None;
                        if op == 'y' {
                            // yy 後は内部レジスタの内容をクリップボードにも反映
                            if let Some(reg) = editor.register.clone() {
                                let _ = App::copy_to_clipboard(&reg.text);
                            }
                        }
                    }
                    KeyCode::Char('w') => {
                        match op {
                            'd' => editor.delete_word_forward(),
                            'y' => editor.yank_word_forward(),
                            'c' => editor.change_word_forward(),
                            _ => {}
                        }
                        editor.pending_chord = editor::PendingChord::None;
                        if op == 'y' {
                            if let Some(reg) = editor.register.clone() {
                                let _ = App::copy_to_clipboard(&reg.text);
                            }
                        }
                    }
                    KeyCode::Char('i') => {
                        editor.pending_chord = editor::PendingChord::OperatorInner(op);
                    }
                    _ => {
                        // Esc 含めて不明なキーはチョードをキャンセル
                        editor.pending_chord = editor::PendingChord::None;
                    }
                }
            }
            // di / yi / ci の 3 段目（テキストオブジェクト）
            editor::PendingChord::OperatorInner(op) => {
                if let KeyCode::Char('w') = key.code {
                    match op {
                        'd' => editor.delete_inner_word(),
                        'y' => editor.yank_inner_word(),
                        'c' => editor.change_inner_word(),
                        _ => {}
                    }
                    if op == 'y' {
                        if let Some(reg) = editor.register.clone() {
                            let _ = App::copy_to_clipboard(&reg.text);
                        }
                    }
                }
                editor.pending_chord = editor::PendingChord::None;
            }
        }
    }

    /// Visual / VisualLine モードのキーハンドラ。
    fn handle_editor_visual_key(&mut self, key: KeyEvent) {
        let idx = self.active_tab;
        // 共通スクロール・画面移動キー（h/j/k/l/g/G/0/$/PageUp/PageDown/Ctrl+D/Ctrl+U/H/L/Home/End/矢印）
        // を先に dispatch する。Visual モード中も移動で範囲を拡縮する。
        if scrollable::dispatch_scroll_key(&mut self.tabs[idx].editor, &key, 20) {
            return;
        }

        match key.code {
            KeyCode::Esc => self.tabs[idx].editor.enter_normal(),
            KeyCode::Char('v') => {
                // Visual 中に v: Normal へ。VisualLine 中に v: Visual へ
                if self.tabs[idx].editor.mode == editor::EditorMode::Visual {
                    self.tabs[idx].editor.enter_normal();
                } else {
                    self.tabs[idx].editor.mode = editor::EditorMode::Visual;
                }
            }
            KeyCode::Char('V') => {
                if self.tabs[idx].editor.mode == editor::EditorMode::VisualLine {
                    self.tabs[idx].editor.enter_normal();
                } else {
                    self.tabs[idx].editor.mode = editor::EditorMode::VisualLine;
                }
            }
            KeyCode::Char('o') => self.tabs[idx].editor.swap_visual_anchor(),
            // 単語移動
            KeyCode::Char('w') => self.tabs[idx].editor.move_word_forward(),
            KeyCode::Char('b') => self.tabs[idx].editor.move_word_back(),
            KeyCode::Char('e') => self.tabs[idx].editor.move_word_end(),
            KeyCode::Char('^') => self.tabs[idx].editor.move_first_non_blank(),
            // ヤンク
            KeyCode::Char('y') => {
                if let Some((text, kind)) = self.tabs[idx].editor.selection_text() {
                    self.yank_to_register_and_clipboard(text, kind);
                    self.status_message = Some("選択範囲をコピーしました".to_string());
                }
                self.tabs[idx].editor.enter_normal();
            }
            // 削除
            KeyCode::Char('d') | KeyCode::Char('x') => {
                if let Some((text, kind)) = self.tabs[idx].editor.delete_selection() {
                    self.yank_to_register_and_clipboard(text, kind);
                }
            }
            // 削除して Insert
            KeyCode::Char('c') => {
                if let Some((text, kind)) = self.tabs[idx].editor.delete_selection() {
                    self.yank_to_register_and_clipboard(text, kind);
                }
                self.tabs[idx].editor.mode = editor::EditorMode::Insert;
            }
            // フォーマット（バッファ全体）
            KeyCode::Char('=') => {
                if self.tabs[idx].editor.format_buffer() {
                    self.status_message = Some("クエリをフォーマットしました".to_string());
                }
                self.tabs[idx].editor.enter_normal();
            }
            // 大小反転
            KeyCode::Char('~') => {
                self.tabs[idx].editor.toggle_case_selection();
                self.tabs[idx].editor.enter_normal();
            }
            _ => {}
        }
    }

    fn handle_editor_insert_key(&mut self, key: KeyEvent) {
        let idx = self.active_tab;
        // プルダウン表示中の特殊キー
        if self.tabs[idx].editor.completion.active {
            match key.code {
                KeyCode::Tab | KeyCode::Down => {
                    self.tabs[idx].editor.completion.next();
                    return;
                }
                KeyCode::BackTab | KeyCode::Up => {
                    self.tabs[idx].editor.completion.prev();
                    return;
                }
                KeyCode::Enter => {
                    self.tabs[idx].editor.accept_completion();
                    return;
                }
                KeyCode::Esc => {
                    self.tabs[idx].editor.completion.close();
                    return; // Normal には戻さない
                }
                _ => {
                    // 他のキーは通常処理に fallthrough（プルダウンは更新される）
                }
            }
        }

        match key.code {
            KeyCode::Esc => self.tabs[idx].editor.enter_normal(),
            KeyCode::Char(ch) => self.tabs[idx].editor.insert_char(ch),
            KeyCode::Enter => self.tabs[idx].editor.insert_newline(),
            KeyCode::Backspace => self.tabs[idx].editor.backspace(),
            KeyCode::Delete => self.tabs[idx].editor.delete(),
            KeyCode::Left => self.tabs[idx].editor.move_left(),
            KeyCode::Right => self.tabs[idx].editor.move_right(),
            KeyCode::Up => self.tabs[idx].editor.move_up(),
            KeyCode::Down => self.tabs[idx].editor.move_down(),
            KeyCode::Home => self.tabs[idx].editor.move_home(),
            KeyCode::End => self.tabs[idx].editor.move_end(),
            _ => {}
        }

        // サジェスト更新（文字入力・削除後）
        self.update_editor_completion();
    }

    fn update_editor_completion(&mut self) {
        let idx = self.active_tab;
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
        self.tabs[idx].editor
            .update_completion(&table_names, &table_columns);
    }

    fn handle_results_key(&mut self, key: KeyEvent) {
        let idx = self.active_tab;
        let was_pending = self.tabs[idx].pending_c;
        // まず pending_c をリセット（c の場合だけ下で再設定）
        self.tabs[idx].pending_c = false;
        // 共通スクロール・画面移動キーを先に dispatch する
        if scrollable::dispatch_scroll_key(&mut self.tabs[idx].results, &key, 20) {
            return;
        }
        match key.code {
            KeyCode::Char('c') => {
                if was_pending {
                    self.try_execute_cc(idx);
                } else {
                    self.tabs[idx].pending_c = true;
                }
            }
            KeyCode::Char('y') => {
                if let Some(csv) = self.tabs[idx].results.copy_current_row() {
                    match App::copy_to_clipboard(&csv) {
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

    fn try_execute_cc(&mut self, idx: usize) {
        let tab = &self.tabs[idx];
        let CcEligibility::Ok { table, pk_columns } = &tab.results.cc_eligibility else {
            let msg = tab.results.cc_eligibility.status_reason().to_string();
            self.status_message = Some(msg);
            return;
        };
        let Some(row) = tab.results.rows.get(tab.results.scroll_offset) else {
            self.status_message = Some("対象行がありません".to_string());
            return;
        };
        let sql =
            cc_edit::build_update_statement(table, &tab.results.columns, row, pk_columns);
        self.tabs[idx].editor.append_text(&sql);
        self.status_message = Some("UPDATE 文を Editor に追記しました".to_string());
    }

    fn handle_new_conn_key(&mut self, key: KeyEvent) -> std::ops::ControlFlow<()> {
        let total = self.new_conn_form.total_rows();
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::ConnectionPicker;
            }
            KeyCode::Tab | KeyCode::Down => {
                self.new_conn_form.cursor = (self.new_conn_form.cursor + 1) % total;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.new_conn_form.cursor = if self.new_conn_form.cursor == 0 {
                    total - 1
                } else {
                    self.new_conn_form.cursor - 1
                };
            }
            KeyCode::Char(ch) => {
                // 特殊行（type / db_type / bool トグルフィールド）では h/l/Space で切替・トグル
                let on_type_row = self.new_conn_form.cursor == 0;
                let on_db_type_row = self.new_conn_form.cursor == 1;
                let on_bool_row = self.new_conn_form.is_current_bool_toggle();

                match ch {
                    'h' if on_type_row => {
                        self.new_conn_form.conn_type = self.new_conn_form.conn_type.cycle_prev();
                        self.new_conn_form.rebuild_fields();
                    }
                    'l' if on_type_row => {
                        self.new_conn_form.conn_type = self.new_conn_form.conn_type.cycle_next();
                        self.new_conn_form.rebuild_fields();
                    }
                    'h' if on_db_type_row => {
                        self.new_conn_form.db_type = self.new_conn_form.db_type.cycle_prev();
                        self.new_conn_form.rebuild_fields();
                        self.new_conn_form.cursor = 1;
                    }
                    'l' if on_db_type_row => {
                        self.new_conn_form.db_type = self.new_conn_form.db_type.cycle_next();
                        self.new_conn_form.rebuild_fields();
                        self.new_conn_form.cursor = 1;
                    }
                    'h' | 'l' | ' ' if on_bool_row => {
                        self.new_conn_form.toggle_current_bool();
                    }
                    _ => {
                        if on_type_row || on_db_type_row || on_bool_row {
                            // 特殊行では通常の文字入力は無視
                        } else if let Some(val) = self.new_conn_form.current_field_mut() {
                            val.push(ch);
                        }
                    }
                }
            }
            KeyCode::Backspace
                if self.new_conn_form.cursor >= 2 && !self.new_conn_form.is_current_bool_toggle() =>
            {
                if let Some(val) = self.new_conn_form.current_field_mut() {
                    val.pop();
                }
            }
            KeyCode::Left => {
                match self.new_conn_form.cursor {
                    0 => {
                        self.new_conn_form.conn_type = self.new_conn_form.conn_type.cycle_prev();
                        self.new_conn_form.rebuild_fields();
                    }
                    1 => {
                        self.new_conn_form.db_type = self.new_conn_form.db_type.cycle_prev();
                        self.new_conn_form.rebuild_fields();
                        self.new_conn_form.cursor = 1; // db_type 行に留まる
                    }
                    _ => {
                        if self.new_conn_form.is_current_bool_toggle() {
                            self.new_conn_form.toggle_current_bool();
                        }
                    }
                }
            }
            KeyCode::Right => {
                match self.new_conn_form.cursor {
                    0 => {
                        self.new_conn_form.conn_type = self.new_conn_form.conn_type.cycle_next();
                        self.new_conn_form.rebuild_fields();
                    }
                    1 => {
                        self.new_conn_form.db_type = self.new_conn_form.db_type.cycle_next();
                        self.new_conn_form.rebuild_fields();
                        self.new_conn_form.cursor = 1;
                    }
                    _ => {
                        if self.new_conn_form.is_current_bool_toggle() {
                            self.new_conn_form.toggle_current_bool();
                        }
                    }
                }
            }
            KeyCode::Enter => {
                // selector 行なら次のフィールドへ移動
                if self.new_conn_form.cursor < 2 {
                    self.new_conn_form.cursor += 1;
                    return std::ops::ControlFlow::Continue(());
                }

                // 接続設定を構築
                match self.build_connection_from_form() {
                    Ok(conn) => {
                        use crate::config::connections::{save_all_connections, save_connection};

                        let form_mode = self.new_conn_form.form_mode.clone();
                        match form_mode {
                            FormMode::Edit(idx) => {
                                if idx >= self.connections.len() {
                                    self.status_message =
                                        Some("編集対象が見つかりませんでした".to_string());
                                    return std::ops::ControlFlow::Continue(());
                                }
                                self.connections[idx] = conn.clone();
                                if let Err(e) = save_all_connections(&self.connections) {
                                    self.status_message = Some(format!("保存エラー: {}", e));
                                    return std::ops::ControlFlow::Continue(());
                                }
                                self.picker_cursor = idx;
                                self.mode = AppMode::ConnectionPicker;
                                self.status_message = Some(format!(
                                    "接続を更新しました: {} (.bak にバックアップ済み)",
                                    conn.name()
                                ));
                            }
                            FormMode::New | FormMode::Duplicate => {
                                if let Err(e) = save_connection(&conn) {
                                    self.status_message = Some(format!("保存エラー: {}", e));
                                    return std::ops::ControlFlow::Continue(());
                                }
                                self.connections.push(conn.clone());
                                self.picker_cursor = self.connections.len() - 1;
                                self.resolved_password = conn.resolve_password().ok().flatten();
                                self.active_connection = Some(ActiveConnectionInfo {
                                    name: conn.name().to_string(),
                                    label: conn.label().map(String::from),
                                    readonly: conn.is_readonly(),
                                    is_redis: matches!(conn.db_type(), DbType::Redis),
                                });
                                self.mode = AppMode::Normal;
                                // 接続切り替え時: 全タブの results をクリア（editor は保持）
                                self.tabs.iter_mut().for_each(|t| t.results.clear());
                                self.schema = SchemaState::new();
                                self.schema.loading = true;

                                match &conn {
                                    ConnectionConfig::Direct(_) | ConnectionConfig::Sqlite(_) => {
                                        self.status_message =
                                            Some(format!("接続中: {}...", conn.name()));
                                        spawn_fetch_tables(
                                            &conn,
                                            self.resolved_password.clone(),
                                            self.tx.clone(),
                                        );
                                    }
                                    ConnectionConfig::Ssh(_) | ConnectionConfig::Ssm(_) => {
                                        self.status_message =
                                            Some(format!("トンネル確立中: {}...", conn.name()));
                                        spawn_tunnel(conn, self.tx.clone());
                                    }
                                }
                            }
                        }
                    }
                    Err(msg) => {
                        self.status_message = Some(msg);
                    }
                }
            }
            _ => {}
        }
        std::ops::ControlFlow::Continue(())
    }

    /// フォームから ConnectionConfig を構築する
    fn build_connection_from_form(&self) -> std::result::Result<ConnectionConfig, String> {
        let form = &self.new_conn_form;
        let db_type = form.db_type.to_db_type();

        let name = form.get("name").to_string();

        if name.is_empty() {
            return Err("name は必須です".to_string());
        }

        let label = {
            let v = form.get("label").to_string();
            if v.is_empty() { None } else { Some(v) }
        };
        let readonly = matches!(form.get("readonly").to_lowercase().as_str(), "true" | "yes" | "1");

        // SQLite は path のみの専用バリアントを構築して早期 return
        if form.db_type.is_sqlite() {
            use crate::config::connections::SqliteConfig;
            let path = form.get("path").to_string();
            if path.is_empty() {
                return Err("path は必須です".to_string());
            }
            return Ok(ConnectionConfig::Sqlite(SqliteConfig {
                name,
                label,
                readonly,
                path,
            }));
        }

        let database = form.get("database").to_string();
        if database.is_empty() {
            return Err("database は必須です".to_string());
        }

        let user = form.get("user").to_string();
        let password_raw = form.get("password").to_string();

        // パスワードの決定ロジック
        // - 入力あり: keychain に保存して `keychain:NAME` を設定
        // - 入力空かつ編集モード: 既存接続の password フィールドを維持
        // - 入力空かつ新規/複製: None
        let password_field = if !password_raw.is_empty() {
            use crate::config::connections::set_keychain_password;
            if let Err(e) = set_keychain_password(&name, &password_raw) {
                return Err(format!("キーチェーン保存エラー: {}", e));
            }
            Some(format!("keychain:{}", name))
        } else if let FormMode::Edit(idx) = &form.form_mode {
            self.connections
                .get(*idx)
                .and_then(|c| c.password_field().map(String::from))
        } else {
            None
        };

        use crate::config::connections::{DirectConfig, SshConfig, SsmConfig};

        match form.conn_type {
            ConnFormType::Direct => {
                let port: u16 = form.get("port").parse().unwrap_or(db_type.default_port());
                let host = form.get("host").to_string();

                Ok(ConnectionConfig::Direct(DirectConfig {
                    name,
                    label,
                    readonly,
                    db_type,
                    host,
                    port,
                    database,
                    user,
                    password: password_field,
                }))
            }
            ConnFormType::Ssh => {
                let ssh_host = form.get("ssh_host").to_string();
                if ssh_host.is_empty() {
                    return Err("ssh_host は必須です".to_string());
                }
                let ssh_user = {
                    let v = form.get("ssh_user").to_string();
                    if v.is_empty() { None } else { Some(v) }
                };
                let remote_db_host = form.get("remote_db_host").to_string();
                if remote_db_host.is_empty() {
                    return Err("remote_db_host は必須です".to_string());
                }
                let remote_db_port: u16 = form.get("remote_db_port").parse().unwrap_or(db_type.default_port());
                let local_port: u16 = form.get("local_port").parse().unwrap_or(0);
                if local_port == 0 {
                    return Err("local_port は必須です".to_string());
                }

                Ok(ConnectionConfig::Ssh(SshConfig {
                    name,
                    label,
                    readonly,
                    db_type,
                    ssh_host,
                    ssh_user,
                    remote_db_host,
                    remote_db_port,
                    local_port,
                    database,
                    user,
                    password: password_field,
                }))
            }
            ConnFormType::Ssm => {
                let instance_id = form.get("instance_id").to_string();
                if instance_id.is_empty() {
                    return Err("instance_id は必須です".to_string());
                }
                let ssh_user = form.get("ssh_user").to_string();
                if ssh_user.is_empty() {
                    return Err("ssh_user は必須です".to_string());
                }
                let ssh_key = {
                    let v = form.get("ssh_key").to_string();
                    if v.is_empty() { None } else { Some(v) }
                };
                let aws_profile = {
                    let v = form.get("aws_profile").to_string();
                    if v.is_empty() { None } else { Some(v) }
                };
                let remote_db_host = form.get("remote_db_host").to_string();
                if remote_db_host.is_empty() {
                    return Err("remote_db_host は必須です".to_string());
                }
                let remote_db_port: u16 = form.get("remote_db_port").parse().unwrap_or(db_type.default_port());
                let local_port: u16 = form.get("local_port").parse().unwrap_or(0);
                if local_port == 0 {
                    return Err("local_port は必須です".to_string());
                }

                Ok(ConnectionConfig::Ssm(SsmConfig {
                    name,
                    label,
                    readonly,
                    db_type,
                    instance_id,
                    ssh_user,
                    ssh_key,
                    aws_profile,
                    remote_db_host,
                    remote_db_port,
                    local_port,
                    database,
                    user,
                    password: password_field,
                }))
            }
        }
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
            KeyCode::Down | KeyCode::Tab if !self.history_entries.is_empty() => {
                self.history_cursor = (self.history_cursor + 1) % self.history_entries.len();
            }
            KeyCode::Up | KeyCode::BackTab if !self.history_entries.is_empty() => {
                self.history_cursor = if self.history_cursor == 0 {
                    self.history_entries.len() - 1
                } else {
                    self.history_cursor - 1
                };
            }
            KeyCode::Enter => {
                if let Some(entry) = self.history_entries.get(self.history_cursor) {
                    let idx = self.active_tab;
                    self.tabs[idx].editor.set_content(&entry.query);
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
        let idx = self.active_tab;
        if self.tabs[idx].results.result.is_none() {
            self.status_message = Some("エクスポートするクエリ結果がありません".to_string());
            return;
        }
        self.export_cursor = 0;
        self.mode = AppMode::ExportFormatPicker;
    }

    fn handle_export_format_key(&mut self, key: KeyEvent) -> std::ops::ControlFlow<()> {
        // Format picker のエントリ数（CSV / JSON / Clipboard）
        const FORMAT_COUNT: usize = 3;
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.export_cursor = (self.export_cursor + 1) % FORMAT_COUNT;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.export_cursor = if self.export_cursor == 0 {
                    FORMAT_COUNT - 1
                } else {
                    self.export_cursor - 1
                };
            }
            KeyCode::Enter => {
                let format = match self.export_cursor {
                    0 => ExportFormat::Csv,
                    1 => ExportFormat::Json,
                    _ => ExportFormat::Clipboard,
                };

                // Clipboard はファイル保存を経由せず、結果テーブルをそのままコピーする。
                if format == ExportFormat::Clipboard {
                    let idx = self.active_tab;
                    if let Some(ref qr) = self.tabs[idx].results.result {
                        let text = export::to_table(qr);
                        self.status_message = Some(match App::copy_to_clipboard(&text) {
                            Ok(_) => format!("クリップボードにコピーしました ({} 行)", qr.rows.len()),
                            Err(e) => format!("クリップボードへのコピーに失敗しました: {}", e),
                        });
                    } else {
                        self.status_message = Some("エクスポートするクエリ結果がありません".to_string());
                    }
                    self.mode = AppMode::Normal;
                    return std::ops::ControlFlow::Continue(());
                }

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
                let idx = self.active_tab;
                if let (Some(ref qr), Some(format)) = (&self.tabs[idx].results.result, self.export_format) {
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
        let idx = self.active_tab;
        let tab_id = self.tabs[idx].id;

        if self.tabs[idx].editor.executing {
            return;
        }

        let query = match self.tabs[idx].editor.get_query_at_cursor() {
            Some(q) => q,
            None => {
                self.status_message = Some("実行するクエリがありません".to_string());
                return;
            }
        };

        // readonly チェック（Redis は別ルールで判定）
        let is_redis = self
            .active_connection
            .as_ref()
            .map(|c| c.is_redis)
            .unwrap_or(false);
        if let Some(ref conn_info) = self.active_connection {
            if conn_info.readonly {
                let check_result = if is_redis {
                    RedisReadonlyChecker.check(&query)
                } else {
                    ReadonlyChecker.check(&query)
                };
                if let Err(e) = check_result {
                    self.tabs[idx].results.set_error(format!("{}", e));
                    self.status_message = Some(format!("{}", e));
                    return;
                }
            }
        }

        // LIMIT 付与（Redis は SQL ではないため自動 LIMIT をスキップ）
        let (final_query, auto_limited) = if is_redis {
            (query.clone(), false)
        } else {
            let applier = LimitApplier {
                default_limit: self.config.default_limit,
            };
            applier.apply(&query)
        };

        // 接続設定取得
        if let Some(conn) = self.connections.get(self.picker_cursor).cloned() {
            self.tabs[idx].editor.executing = true;
            self.status_message = Some("クエリ実行中...".to_string());
            spawn_execute_query(&conn, &final_query, auto_limited, &query, tab_id, self.resolved_password.clone(), self.tx.clone());
        } else {
            self.status_message = Some("接続が選択されていません".to_string());
        }
    }
}

// ── 非同期スキーマ取得 ──

/// DB 種別に応じて識別子をクォートする
fn quote_identifier(name: &str, db_type: &DbType) -> String {
    db_type.quote_identifier(name)
}

fn build_adapter(conn: &ConnectionConfig, password: Option<String>) -> Option<AnyAdapter> {
    // SQLite は host/port/user/password を持たない別経路で構築
    if let ConnectionConfig::Sqlite(c) = conn {
        return Some(AnyAdapter::Sqlite(SqliteAdapter::new(
            c.path.clone(),
            c.readonly,
        )));
    }

    let (host, port, database, user, db_type) = match conn {
        ConnectionConfig::Direct(c) => (c.host.clone(), c.port, c.database.clone(), c.user.clone(), &c.db_type),
        ConnectionConfig::Ssh(c) => ("127.0.0.1".to_string(), c.local_port, c.database.clone(), c.user.clone(), &c.db_type),
        ConnectionConfig::Ssm(c) => ("127.0.0.1".to_string(), c.local_port, c.database.clone(), c.user.clone(), &c.db_type),
        ConnectionConfig::Sqlite(_) => unreachable!("SQLite is handled above"),
    };

    match db_type {
        DbType::Postgresql => Some(AnyAdapter::Postgres(PostgresAdapter::new(host, port, database, user, password))),
        DbType::Mysql => Some(AnyAdapter::Mysql(MysqlAdapter::new(host, port, database, user, password))),
        DbType::Redis => Some(AnyAdapter::Redis(RedisAdapter::new(host, port, database, user, password))),
        DbType::Sqlite => unreachable!("SQLite is handled above"),
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
    if let Some(mut adapter) = build_adapter(conn, password) {
        tokio::spawn(async move {
            let result = match adapter.connect().await {
                Ok(()) => adapter.fetch_tables().await,
                Err(e) => Err(e),
            };
            let _ = tx.send(AppEvent::TablesLoaded(result)).await;
        });
    }
}

fn spawn_fetch_columns(conn: &ConnectionConfig, table_name: &str, password: Option<String>, tx: mpsc::Sender<AppEvent>) {
    if let Some(mut adapter) = build_adapter(conn, password) {
        let table = table_name.to_string();
        tokio::spawn(async move {
            let result = match adapter.connect().await {
                Ok(()) => adapter.fetch_columns(&table).await,
                Err(e) => Err(e),
            };
            let _ = tx.send(AppEvent::ColumnsLoaded(table, result)).await;
        });
    }
}

fn spawn_execute_query(
    conn: &ConnectionConfig,
    query: &str,
    auto_limited: bool,
    original_query: &str,
    tab_id: usize,
    password: Option<String>,
    tx: mpsc::Sender<AppEvent>,
) {
    if let Some(mut adapter) = build_adapter(conn, password) {
        let query = query.to_string();
        let original = original_query.to_string();
        tokio::spawn(async move {
            let result = match adapter.connect().await {
                Ok(()) => adapter.execute(&query).await,
                Err(e) => Err(e),
            };
            let _ = tx.send(AppEvent::QueryCompleted(result, auto_limited, original, tab_id)).await;
        });
    }
}

// ── 描画 ──

fn render(f: &mut Frame, app: &mut App) {
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
        AppMode::Help => help::render(f, size, &mut app.help_scroll),
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
    let active_editor = app.active_editor();
    let (panel_label, mode_span) = match app.active_panel {
        Panel::Schema => ("Schema".to_string(), None),
        Panel::Editor => {
            let mode_str = match active_editor.mode {
                editor::EditorMode::Normal => "NORMAL",
                editor::EditorMode::Insert => "INSERT",
                editor::EditorMode::Visual => "VISUAL",
                editor::EditorMode::VisualLine => "V-LINE",
            };
            let mode_color = match active_editor.mode {
                editor::EditorMode::Normal => Color::DarkGray,
                editor::EditorMode::Insert => Color::Green,
                editor::EditorMode::Visual => Color::Magenta,
                editor::EditorMode::VisualLine => Color::LightMagenta,
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

    let spinner = if active_editor.executing || app.schema.loading {
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
        let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // ターミナルセットアップ
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;

    // App + イベントチャネル
    let (tx, mut rx) = mpsc::channel::<AppEvent>(100);
    let mut app = App::new(connections, config.clone(), tx.clone());

    // 接続別セッションをインメモリへロード。
    // 実際のタブ展開は「接続が決まったタイミング」で行うため、ここでは状態だけ持っておく。
    let session_store = crate::session::SessionStore::new();
    app.session_state = session_store.load();

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
            // 自動接続の場合も保存済みタブを展開する（初回接続なので prev は None）
            app.switch_connection_session(None, conn.name());
            app.active_connection = Some(ActiveConnectionInfo {
                name: conn.name().to_string(),
                label: conn.label().map(String::from),
                readonly: conn.is_readonly(),
                is_redis: matches!(conn.db_type(), DbType::Redis),
            });
            app.mode = AppMode::Normal;
            app.schema = SchemaState::new();
            app.schema.loading = true;
            match &conn {
                ConnectionConfig::Direct(_) | ConnectionConfig::Sqlite(_) => {
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
                Ok(Some(Event::Paste(text))) => {
                    if key_tx.send(AppEvent::Paste(text)).await.is_err() {
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
        let idx = app.active_tab;
        // Editor の可視幅を計算: 右70% - 外枠2 - 行番号桁 - 区切り1
        let line_num_width = format!("{}", app.tabs[idx].editor.lines.len()).len().max(2);
        let editor_inner_width = (term_width * 70 / 100)
            .saturating_sub(2)
            .saturating_sub(line_num_width + 1);
        app.tabs[idx]
            .editor
            .adjust_scroll(editor_height, editor_inner_width);
        // Results パネルの表示幅を更新（右70% - ボーダー2）
        app.tabs[idx].results.visible_width = (term_width * 70 / 100).saturating_sub(2);

        terminal.draw(|f| render(f, &mut app))?;

        if let Some(event) = rx.recv().await {
            if app.handle_event(event).is_break() {
                break;
            }
        } else {
            break;
        }
    }

    // 現在の接続のタブ群を取り込んでからファイルへ書き出す。
    // active_connection が無い場合（一度も接続せず終了）はファイル更新だけ行う。
    if let Some(name) = app.active_connection.as_ref().map(|c| c.name.clone()) {
        let snap = app.capture_connection_session();
        app.session_state.set(name, snap);
    }
    if let Err(err) = session_store.save(&app.session_state) {
        eprintln!("session の保存に失敗しました: {}", err);
    }

    // トンネルのクリーンアップ
    if let Some(mut tunnel) = app.active_tunnel.take() {
        tunnel.kill().await;
    }

    // ターミナル復元
    crossterm::terminal::disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;

    Ok(())
}

#[cfg(test)]
mod tab_tests;
