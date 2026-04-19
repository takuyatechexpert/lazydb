use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DbType {
    #[default]
    Postgresql,
    Mysql,
}

impl fmt::Display for DbType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbType::Postgresql => write!(f, "postgresql"),
            DbType::Mysql => write!(f, "mysql"),
        }
    }
}

impl DbType {
    pub fn default_port(&self) -> u16 {
        match self {
            DbType::Postgresql => 5432,
            DbType::Mysql => 3306,
        }
    }

    /// 識別子をクォートする（PostgreSQL: "...", MySQL: `...`）
    pub fn quote_identifier(&self, name: &str) -> String {
        match self {
            DbType::Postgresql => {
                let needs_quote = name.is_empty()
                    || name.chars().next().is_none_or(|c| !c.is_ascii_lowercase() && c != '_')
                    || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
                if needs_quote {
                    format!("\"{}\"", name.replace('"', "\"\""))
                } else {
                    name.to_string()
                }
            }
            DbType::Mysql => {
                let needs_quote = name.is_empty()
                    || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if needs_quote {
                    format!("`{}`", name.replace('`', "``"))
                } else {
                    name.to_string()
                }
            }
        }
    }
}

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

    pub fn db_type(&self) -> &DbType {
        match self {
            ConnectionConfig::Direct(c) => &c.db_type,
            ConnectionConfig::Ssh(c) => &c.db_type,
            ConnectionConfig::Ssm(c) => &c.db_type,
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
///   None             → パスワードなしで接続（trust / peer 認証向け）
///   "prompt"         → 対話入力（マスク表示）
///   "env:VAR"        → 環境変数から取得
///   "keychain:NAME"  → OS キーチェーンから取得（macOS Keychain / Linux Secret Service）
///   それ以外         → そのまま使用
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
        Some(s) if s.starts_with("keychain:") => {
            let conn_name = &s[9..];
            get_keychain_password(conn_name)
        }
        Some(s) => Ok(Some(s.to_string())),
    }
}

const KEYCHAIN_SERVICE: &str = "lazydb";

/// OS キーチェーンからパスワードを取得する
fn get_keychain_password(conn_name: &str) -> Result<Option<String>> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, conn_name)
        .context("キーチェーンエントリの作成に失敗しました")?;
    match entry.get_password() {
        Ok(pw) => Ok(Some(pw)),
        Err(keyring::Error::NoEntry) => {
            anyhow::bail!(
                "キーチェーンにパスワードが登録されていません: {}\n\
                 登録コマンド: lazydb set-password {}",
                conn_name,
                conn_name
            )
        }
        Err(e) => anyhow::bail!("キーチェーンからの取得に失敗しました: {}", e),
    }
}

/// OS キーチェーンにパスワードを保存する
pub fn set_keychain_password(conn_name: &str, password: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, conn_name)
        .context("キーチェーンエントリの作成に失敗しました")?;
    entry
        .set_password(password)
        .with_context(|| format!("キーチェーンへの保存に失敗しました: {}", conn_name))?;
    Ok(())
}

/// OS キーチェーンからパスワードを削除する
pub fn delete_keychain_password(conn_name: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, conn_name)
        .context("キーチェーンエントリの作成に失敗しました")?;
    entry
        .delete_credential()
        .with_context(|| format!("キーチェーンからの削除に失敗しました: {}", conn_name))?;
    Ok(())
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

/// 接続設定を connections.yml に追記する（既存内容・コメントを保持）
pub fn save_connection(conn: &ConnectionConfig) -> Result<()> {
    let path = expand_tilde("~/.config/lazydb/connections.yml");

    // ディレクトリ作成
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("ディレクトリを作成できません: {:?}", parent))?;
    }

    // 新しい接続エントリを手書き YAML で生成（serde_yaml で全体を書き直さない）
    let entry_yaml = connection_to_yaml(conn);

    // 既存ファイルの末尾に追記
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("接続設定ファイルを開けません: {}", path.display()))?;

    // 既存ファイルが空でなく改行で終わっていない場合は改行を追加
    if path.exists() {
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if !existing.is_empty() && !existing.ends_with('\n') {
            writeln!(file)?;
        }
    }

    write!(file, "{}", entry_yaml)
        .with_context(|| format!("接続設定ファイルに書き込めません: {}", path.display()))?;

    ensure_secure_permissions(&path);

    Ok(())
}

/// ConnectionConfig を手書き YAML 文字列に変換（コメント保持のため serde を使わない）
fn connection_to_yaml(conn: &ConnectionConfig) -> String {
    match conn {
        ConnectionConfig::Direct(c) => {
            let mut lines = vec![
                format!("\n- type: direct"),
                format!("  name: {}", c.name),
            ];
            if let Some(ref label) = c.label {
                lines.push(format!("  label: {}", label));
            }
            if c.readonly {
                lines.push("  readonly: true".to_string());
            }
            if c.db_type != DbType::Postgresql {
                lines.push(format!("  db_type: {}", c.db_type));
            }
            lines.push(format!("  host: {}", c.host));
            lines.push(format!("  port: {}", c.port));
            lines.push(format!("  database: {}", c.database));
            lines.push(format!("  user: {}", c.user));
            if let Some(ref pw) = c.password {
                lines.push(format!("  password: \"{}\"", pw));
            }
            lines.push(String::new());
            lines.join("\n")
        }
        ConnectionConfig::Ssh(c) => {
            let mut lines = vec![
                format!("\n- type: ssh"),
                format!("  name: {}", c.name),
            ];
            if let Some(ref label) = c.label {
                lines.push(format!("  label: {}", label));
            }
            if c.readonly {
                lines.push("  readonly: true".to_string());
            }
            if c.db_type != DbType::Postgresql {
                lines.push(format!("  db_type: {}", c.db_type));
            }
            lines.push(format!("  ssh_host: {}", c.ssh_host));
            if let Some(ref user) = c.ssh_user {
                lines.push(format!("  ssh_user: {}", user));
            }
            lines.push(format!("  remote_db_host: {}", c.remote_db_host));
            lines.push(format!("  remote_db_port: {}", c.remote_db_port));
            lines.push(format!("  local_port: {}", c.local_port));
            lines.push(format!("  database: {}", c.database));
            lines.push(format!("  user: {}", c.user));
            if let Some(ref pw) = c.password {
                lines.push(format!("  password: \"{}\"", pw));
            }
            lines.push(String::new());
            lines.join("\n")
        }
        ConnectionConfig::Ssm(c) => {
            let mut lines = vec![
                format!("\n- type: ssm"),
                format!("  name: {}", c.name),
            ];
            if let Some(ref label) = c.label {
                lines.push(format!("  label: {}", label));
            }
            if c.readonly {
                lines.push("  readonly: true".to_string());
            }
            if c.db_type != DbType::Postgresql {
                lines.push(format!("  db_type: {}", c.db_type));
            }
            lines.push(format!("  instance_id: {}", c.instance_id));
            lines.push(format!("  ssh_user: {}", c.ssh_user));
            if let Some(ref key) = c.ssh_key {
                lines.push(format!("  ssh_key: {}", key));
            }
            if let Some(ref profile) = c.aws_profile {
                lines.push(format!("  aws_profile: {}", profile));
            }
            lines.push(format!("  remote_db_host: {}", c.remote_db_host));
            lines.push(format!("  remote_db_port: {}", c.remote_db_port));
            lines.push(format!("  local_port: {}", c.local_port));
            lines.push(format!("  database: {}", c.database));
            lines.push(format!("  user: {}", c.user));
            if let Some(ref pw) = c.password {
                lines.push(format!("  password: \"{}\"", pw));
            }
            lines.push(String::new());
            lines.join("\n")
        }
    }
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
    #[serde(default)]
    pub db_type: DbType,
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
    #[serde(default)]
    pub db_type: DbType,
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
    #[serde(default)]
    pub db_type: DbType,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── DbType ──

    #[test]
    fn db_type_default_is_postgresql() {
        assert_eq!(DbType::default(), DbType::Postgresql);
    }

    #[test]
    fn db_type_default_ports() {
        assert_eq!(DbType::Postgresql.default_port(), 5432);
        assert_eq!(DbType::Mysql.default_port(), 3306);
    }

    #[test]
    fn db_type_display() {
        assert_eq!(format!("{}", DbType::Postgresql), "postgresql");
        assert_eq!(format!("{}", DbType::Mysql), "mysql");
    }

    // ── quote_identifier PostgreSQL ──

    #[test]
    fn pg_quote_lowercase_no_quote() {
        assert_eq!(DbType::Postgresql.quote_identifier("users"), "users");
    }

    #[test]
    fn pg_quote_uppercase_needs_quote() {
        assert_eq!(DbType::Postgresql.quote_identifier("Users"), "\"Users\"");
    }

    #[test]
    fn pg_quote_special_chars() {
        assert_eq!(DbType::Postgresql.quote_identifier("my-table"), "\"my-table\"");
    }

    #[test]
    fn pg_quote_escapes_double_quotes() {
        assert_eq!(DbType::Postgresql.quote_identifier("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn pg_quote_empty() {
        assert_eq!(DbType::Postgresql.quote_identifier(""), "\"\"");
    }

    // ── quote_identifier MySQL ──

    #[test]
    fn mysql_quote_simple_no_quote() {
        assert_eq!(DbType::Mysql.quote_identifier("users"), "users");
    }

    #[test]
    fn mysql_quote_uppercase_no_quote() {
        // MySQL はケース非依存なのでクォート不要
        assert_eq!(DbType::Mysql.quote_identifier("Users"), "Users");
    }

    #[test]
    fn mysql_quote_special_chars() {
        assert_eq!(DbType::Mysql.quote_identifier("my-table"), "`my-table`");
    }

    #[test]
    fn mysql_quote_escapes_backticks() {
        assert_eq!(DbType::Mysql.quote_identifier("a`b"), "`a``b`");
    }

    #[test]
    fn mysql_quote_empty() {
        assert_eq!(DbType::Mysql.quote_identifier(""), "``");
    }

    // ── YAML デシリアライズ後方互換性 ──

    #[test]
    fn deserialize_without_db_type_defaults_to_postgresql() {
        let yaml = r#"
- type: direct
  name: test
  host: localhost
  port: 5432
  database: testdb
  user: postgres
"#;
        let conns: Vec<ConnectionConfig> = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(conns[0].db_type(), &DbType::Postgresql);
    }

    #[test]
    fn deserialize_with_db_type_mysql() {
        let yaml = r#"
- type: direct
  name: test
  db_type: mysql
  host: localhost
  port: 3306
  database: testdb
  user: root
"#;
        let conns: Vec<ConnectionConfig> = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(conns[0].db_type(), &DbType::Mysql);
    }
}
