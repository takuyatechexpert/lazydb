#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

const MAX_ENTRIES: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub query: String,
    pub connection: String,
    pub executed_at: DateTime<Utc>,
    pub rows: usize,
    pub duration_ms: u64,
}

pub struct HistoryStore {
    path: PathBuf,
}

impl HistoryStore {
    pub fn new() -> Self {
        let path = dirs_or_default().join("history.ndjson");
        Self { path }
    }

    pub fn append(&self, query: &str, connection: &str, rows: usize, duration_ms: u64) -> Result<()> {
        let entry = HistoryEntry {
            id: Uuid::new_v4().to_string(),
            query: query.to_string(),
            connection: connection.to_string(),
            executed_at: Utc::now(),
            rows,
            duration_ms,
        };

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("履歴ディレクトリを作成できません: {:?}", parent))?;
        }

        let mut entries = self.load_all().unwrap_or_default();
        entries.push(entry);

        if entries.len() > MAX_ENTRIES {
            entries = entries.split_off(entries.len() - MAX_ENTRIES);
        }

        let content: String = entries
            .iter()
            .map(|e| serde_json::to_string(e).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&self.path, content + "\n")
            .with_context(|| format!("履歴ファイルに書き込めません: {:?}", self.path))?;

        ensure_secure_permissions(&self.path);

        Ok(())
    }

    pub fn load_all(&self) -> Result<Vec<HistoryEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(&self.path)
            .with_context(|| format!("履歴ファイルを読み込めません: {:?}", self.path))?;

        let entries: Vec<HistoryEntry> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();

        Ok(entries)
    }

    pub fn search(&self, filter: &str) -> Result<Vec<HistoryEntry>> {
        let entries = self.load_all()?;
        let filter_lower = filter.to_lowercase();

        let filtered: Vec<HistoryEntry> = entries
            .into_iter()
            .rev()
            .filter(|e| e.query.to_lowercase().contains(&filter_lower))
            .collect();

        Ok(filtered)
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
