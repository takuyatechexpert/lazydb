#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabSnapshot {
    pub name: String,
    pub content: String,
    #[serde(default)]
    pub cursor_row: usize,
    #[serde(default)]
    pub cursor_col: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectionSession {
    pub tabs: Vec<TabSnapshot>,
    #[serde(default)]
    pub active_tab: usize,
}

/// セッションファイルのルート構造。
/// 接続名をキーにしたタブ群を保持する（TablePlus 流の per-connection 永続化）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionState {
    /// 接続名 -> その接続で開いていたタブ群
    #[serde(default)]
    pub connections: BTreeMap<String, ConnectionSession>,
}

impl SessionState {
    pub fn get(&self, connection: &str) -> Option<&ConnectionSession> {
        self.connections.get(connection)
    }

    pub fn set(&mut self, connection: String, session: ConnectionSession) {
        self.connections.insert(connection, session);
    }
}

/// クエリタブの内容を `~/.local/share/lazydb/session.json` に永続化するストア。
pub struct SessionStore {
    path: PathBuf,
}

impl SessionStore {
    pub fn new() -> Self {
        let path = dirs_or_default().join("session.json");
        Self { path }
    }

    #[cfg(test)]
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    /// 保存済みセッションを読み込む。ファイル無し・破損時は空の SessionState を返す。
    pub fn load(&self) -> SessionState {
        if !self.path.exists() {
            return SessionState::default();
        }
        let content = match std::fs::read_to_string(&self.path) {
            Ok(c) => c,
            Err(_) => return SessionState::default(),
        };
        serde_json::from_str(&content).unwrap_or_default()
    }

    pub fn save(&self, state: &SessionState) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("セッションディレクトリを作成できません: {:?}", parent))?;
        }
        let content = serde_json::to_string_pretty(state)
            .context("セッションのシリアライズに失敗しました")?;
        std::fs::write(&self.path, content)
            .with_context(|| format!("セッションを書き込めません: {:?}", self.path))?;
        ensure_secure_permissions(&self.path);
        Ok(())
    }
}

#[cfg(unix)]
fn ensure_secure_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(path) {
        let mode = metadata.permissions().mode() & 0o777;
        if mode != 0o600 {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

#[cfg(not(unix))]
fn ensure_secure_permissions(_path: &std::path::Path) {}

fn dirs_or_default() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("lazydb")
}
