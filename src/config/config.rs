use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    /// 起動時に自動接続する接続名
    pub default_connection: Option<String>,

    /// true: default_connection に自動接続
    #[serde(default)]
    pub auto_connect: bool,

    /// SELECT に LIMIT がない場合に付与する件数（0: 無効）
    #[serde(default = "default_limit")]
    pub default_limit: u64,

    /// 接続タイムアウト（秒）
    #[allow(dead_code)]
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout: u64,

    /// 結果の最大表示行数
    #[allow(dead_code)]
    #[serde(default = "default_result_limit")]
    pub result_limit: usize,
}

fn default_limit() -> u64 {
    100
}

fn default_connect_timeout() -> u64 {
    5
}

fn default_result_limit() -> usize {
    500
}

/// config.yml を読み込む（ファイルがなければデフォルト値を返す）
pub fn load_config(config_path: Option<&str>) -> Result<AppConfig> {
    let path: PathBuf = match config_path {
        Some(p) => crate::config::connections::expand_tilde(p),
        None => crate::config::connections::expand_tilde("~/.config/lazydb/config.yml"),
    };

    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("設定ファイルの読み込みに失敗しました: {}", path.display()))?;

    let config: AppConfig = serde_yaml::from_str(&content)
        .with_context(|| format!("設定ファイルのパースに失敗しました: {}", path.display()))?;

    Ok(config)
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_connection: None,
            auto_connect: false,
            default_limit: default_limit(),
            connect_timeout: default_connect_timeout(),
            result_limit: default_result_limit(),
        }
    }
}
