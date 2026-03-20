use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ConnectionConfig {
    Direct(DirectConfig),
    Ssh(SshConfig),
    Ssm(SsmConfig),
}

impl ConnectionConfig {
    pub fn name(&self) -> &str {
        match self {
            ConnectionConfig::Direct(c) => &c.name,
            ConnectionConfig::Ssh(c) => &c.name,
            ConnectionConfig::Ssm(c) => &c.name,
        }
    }

    pub fn label(&self) -> Option<&str> {
        match self {
            ConnectionConfig::Direct(c) => c.label.as_deref(),
            ConnectionConfig::Ssh(c) => c.label.as_deref(),
            ConnectionConfig::Ssm(c) => c.label.as_deref(),
        }
    }

    pub fn is_readonly(&self) -> bool {
        match self {
            ConnectionConfig::Direct(c) => c.readonly,
            ConnectionConfig::Ssh(c) => c.readonly,
            ConnectionConfig::Ssm(c) => c.readonly,
        }
    }

    /// パスワードを解決して返す
    pub fn resolve_password(&self) -> Result<Option<String>> {
        let raw = match self {
            ConnectionConfig::Direct(c) => c.password.as_deref(),
            ConnectionConfig::Ssh(c) => c.password.as_deref(),
            ConnectionConfig::Ssm(c) => c.password.as_deref(),
        };
        resolve_password_value(raw)
    }
}

/// パスワード値を解決する
///   None        → .pgpass に委譲
///   "prompt"    → 対話入力（マスク表示）
///   "env:VAR"   → 環境変数から取得
///   それ以外    → そのまま使用
fn resolve_password_value(value: Option<&str>) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some("prompt") => {
            let password = rpassword::prompt_password("Password: ")
                .context("パスワードの入力に失敗しました")?;
            Ok(Some(password))
        }
        Some(s) if s.starts_with("env:") => {
            let var_name = &s[4..];
            let val = std::env::var(var_name)
                .with_context(|| format!("環境変数 {} が設定されていません", var_name))?;
            Ok(Some(val))
        }
        Some(s) => Ok(Some(s.to_string())),
    }
}

/// connections.yml を読み込む（ファイルがなければ空リストを返す）
pub fn load_connections(config_path: Option<&str>) -> Result<Vec<ConnectionConfig>> {
    let path = match config_path {
        Some(p) => expand_tilde(p),
        None => expand_tilde("~/.config/lazydb/connections.yml"),
    };

    if !path.exists() {
        return Ok(Vec::new());
    }

    // パーミッションが緩い場合は自動修正
    ensure_secure_permissions(&path);

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("接続設定ファイルが見つかりません: {}", path.display()))?;

    let connections: Vec<ConnectionConfig> = serde_yaml::from_str(&content)
        .with_context(|| format!("接続設定ファイルのパースに失敗しました: {}", path.display()))?;

    Ok(connections)
}

/// 接続設定を connections.yml に追記する
pub fn save_connection(conn: &ConnectionConfig) -> Result<()> {
    let path = expand_tilde("~/.config/lazydb/connections.yml");

    // ディレクトリ作成
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("ディレクトリを作成できません: {:?}", parent))?;
    }

    // 既存の接続を読み込み
    let mut connections = if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        serde_yaml::from_str::<Vec<ConnectionConfig>>(&content).unwrap_or_default()
    } else {
        Vec::new()
    };

    connections.push(conn.clone());

    let yaml = serde_yaml::to_string(&connections)
        .context("接続設定のシリアライズに失敗しました")?;
    std::fs::write(&path, yaml)
        .with_context(|| format!("接続設定ファイルに書き込めません: {}", path.display()))?;

    ensure_secure_permissions(&path);

    Ok(())
}

/// ファイルのパーミッションを 600 (owner のみ読み書き) に設定する
#[cfg(unix)]
fn ensure_secure_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(path) {
        let mode = metadata.permissions().mode() & 0o777;
        if mode != 0o600 {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

#[cfg(not(unix))]
fn ensure_secure_permissions(_path: &Path) {
    // Windows では別途対応が必要
}

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(format!("{}/{}", home, rest))
    } else {
        PathBuf::from(path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectConfig {
    pub name: String,
    pub label: Option<String>,
    #[serde(default)]
    pub readonly: bool,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConfig {
    pub name: String,
    pub label: Option<String>,
    #[serde(default)]
    pub readonly: bool,
    /// SSH ホストまたは ~/.ssh/config の Host エイリアス
    pub ssh_host: String,
    /// SSH ユーザー（省略時は ~/.ssh/config の User を使用）
    pub ssh_user: Option<String>,
    pub remote_db_host: String,
    #[serde(default = "default_port")]
    pub remote_db_port: u16,
    pub local_port: u16,
    pub database: String,
    pub user: String,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsmConfig {
    pub name: String,
    pub label: Option<String>,
    #[serde(default)]
    pub readonly: bool,
    pub instance_id: String,
    pub ssh_user: String,
    pub ssh_key: Option<String>,
    pub aws_profile: Option<String>,
    pub remote_db_host: String,
    #[serde(default = "default_port")]
    pub remote_db_port: u16,
    pub local_port: u16,
    pub database: String,
    pub user: String,
    pub password: Option<String>,
}

fn default_port() -> u16 {
    5432
}
