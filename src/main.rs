mod config;
mod db;
mod export;
mod history;
mod tunnel;
mod tui;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use config::{config::load_config, connections::load_connections};
use config::connections::DbType;
use db::{
    adapter::QueryResult,
    mysql::MysqlAdapter,
    postgres::PostgresAdapter,
    AnyAdapter, LimitApplier, ReadonlyChecker,
};
use unicode_width::UnicodeWidthStr;

#[derive(Parser)]
#[command(name = "lazydb")]
#[command(version, about = "A standalone TUI SQL client")]
struct Cli {
    /// 接続名を指定して起動
    #[arg(short, long)]
    connection: Option<String>,

    /// config.yml のパスを指定
    #[arg(long)]
    config: Option<String>,

    /// connections.yml のパスを指定
    #[arg(long)]
    connections: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// 接続一覧を表示して終了
    ListConnections,

    /// 非インタラクティブ実行（スクリプト用）
    Exec {
        /// 接続名（必須）
        #[arg(short, long)]
        connection: String,

        /// SQL 文字列
        #[arg(short, long)]
        query: Option<String>,

        /// SQL ファイルパス
        #[arg(short, long)]
        file: Option<String>,

        /// 出力形式: table（デフォルト）/ csv / json
        #[arg(long, default_value = "table")]
        format: String,

        /// 自動 LIMIT を無効化
        #[arg(long)]
        no_limit: bool,

        /// LIMIT 件数を指定
        #[arg(long)]
        limit: Option<u64>,
    },

    /// パスワードを OS キーチェーンに保存
    SetPassword {
        /// 接続名
        connection: String,
    },

    /// パスワードを OS キーチェーンから削除
    DeletePassword {
        /// 接続名
        connection: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::ListConnections) => {
            cmd_list_connections(cli.connections.as_deref()).await?;
        }
        Some(Commands::Exec { connection, query, file, format, no_limit, limit }) => {
            cmd_exec(
                cli.config.as_deref(),
                cli.connections.as_deref(),
                &connection,
                query.as_deref(),
                file.as_deref(),
                &format,
                no_limit,
                limit,
            )
            .await?;
        }
        Some(Commands::SetPassword { connection }) => {
            cmd_set_password(&connection)?;
        }
        Some(Commands::DeletePassword { connection }) => {
            cmd_delete_password(&connection)?;
        }
        None => {
            let app_config = load_config(cli.config.as_deref())?;
            let connections = load_connections(cli.connections.as_deref())?;
            tui::run(connections, app_config, cli.connection.as_deref()).await?;
        }
    }

    Ok(())
}

fn cmd_set_password(connection: &str) -> Result<()> {
    let password = rpassword::prompt_password(format!("Password for '{}': ", connection))
        .context("パスワードの入力に失敗しました")?;
    config::connections::set_keychain_password(connection, &password)?;
    println!("キーチェーンに保存しました: {}", connection);
    println!("connections.yml で password: \"keychain:{}\" を設定してください", connection);
    Ok(())
}

fn cmd_delete_password(connection: &str) -> Result<()> {
    config::connections::delete_keychain_password(connection)?;
    println!("キーチェーンから削除しました: {}", connection);
    Ok(())
}

async fn cmd_list_connections(connections_path: Option<&str>) -> Result<()> {
    let connections = load_connections(connections_path)?;
    println!("{:<20} {:<8} {:<10} {:<12}", "NAME", "LABEL", "TYPE", "DB");
    println!("{}", "-".repeat(54));
    for conn in &connections {
        let label = conn.label().unwrap_or("-");
        let type_str = match conn {
            config::connections::ConnectionConfig::Direct(_) => "direct",
            config::connections::ConnectionConfig::Ssh(_) => "ssh",
            config::connections::ConnectionConfig::Ssm(_) => "ssm",
        };
        println!("{:<20} {:<8} {:<10} {:<12}", conn.name(), label, type_str, conn.db_type());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_exec(
    config_path: Option<&str>,
    connections_path: Option<&str>,
    connection_name: &str,
    query: Option<&str>,
    file: Option<&str>,
    format: &str,
    no_limit: bool,
    limit_override: Option<u64>,
) -> Result<()> {
    // クエリの取得
    let raw_query = match (query, file) {
        (Some(q), _) => q.to_string(),
        (None, Some(f)) => {
            std::fs::read_to_string(f)
                .with_context(|| format!("SQL ファイルを読み込めません: {}", f))?
        }
        (None, None) => anyhow::bail!("--query または --file を指定してください"),
    };

    // 設定読み込み
    let app_config = load_config(config_path)?;
    let connections = load_connections(connections_path)?;

    // 接続設定を検索
    let conn_config = connections
        .iter()
        .find(|c| c.name() == connection_name)
        .with_context(|| format!("接続 '{}' が見つかりません", connection_name))?;

    // readonly チェック
    if conn_config.is_readonly() {
        ReadonlyChecker.check(&raw_query)?;
    }

    // LIMIT 付与
    let effective_limit = if no_limit {
        0
    } else {
        limit_override.unwrap_or(app_config.default_limit)
    };
    let applier = LimitApplier { default_limit: effective_limit };
    let (final_query, auto_limited) = applier.apply(&raw_query);

    // トンネル起動（SSH/SSM の場合）
    let mut _tunnel: Option<Box<dyn std::any::Any>> = None;
    let password = conn_config.resolve_password()?;

    let (host, port, database, user, db_type) = match conn_config {
        config::connections::ConnectionConfig::Direct(c) => {
            (c.host.clone(), c.port, c.database.clone(), c.user.clone(), &c.db_type)
        }
        config::connections::ConnectionConfig::Ssh(c) => {
            use tunnel::ssh::SshTunnel;
            let tunnel = SshTunnel::start(
                &c.ssh_host,
                c.ssh_user.as_deref(),
                &c.remote_db_host,
                c.remote_db_port,
                c.local_port,
            )
            .await?;
            _tunnel = Some(Box::new(tunnel));
            ("127.0.0.1".to_string(), c.local_port, c.database.clone(), c.user.clone(), &c.db_type)
        }
        config::connections::ConnectionConfig::Ssm(c) => {
            use tunnel::ssm::SsmTunnel;
            let tunnel = SsmTunnel::start(
                &c.instance_id,
                &c.ssh_user,
                c.ssh_key.as_deref(),
                c.aws_profile.as_deref(),
                &c.remote_db_host,
                c.remote_db_port,
                c.local_port,
            )
            .await?;
            _tunnel = Some(Box::new(tunnel));
            ("127.0.0.1".to_string(), c.local_port, c.database.clone(), c.user.clone(), &c.db_type)
        }
    };

    let mut adapter = match db_type {
        DbType::Postgresql => AnyAdapter::Postgres(PostgresAdapter::new(host, port, database, user, password)),
        DbType::Mysql => AnyAdapter::Mysql(MysqlAdapter::new(host, port, database, user, password)),
    };

    // 接続確認
    adapter.connect().await?;

    // クエリ実行
    let result = adapter.execute(&final_query).await?;

    // 出力
    print_result(&result, format, auto_limited)?;

    // _tunnel は drop 時に kill_on_drop で自動終了
    Ok(())
}

fn print_result(result: &QueryResult, format: &str, auto_limited: bool) -> Result<()> {
    match format {
        "csv" => print_csv(result),
        "json" => print_json(result),
        _ => print_table(result, auto_limited),
    }
}

fn print_table(result: &QueryResult, auto_limited: bool) -> Result<()> {
    if result.columns.is_empty() {
        println!("(0 rows)");
        return Ok(());
    }

    // カラム幅計算（ヘッダーと各セルの最大幅）
    let mut widths: Vec<usize> = result.columns.iter().map(|c| UnicodeWidthStr::width(c.as_str())).collect();
    for row in &result.rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(UnicodeWidthStr::width(cell.as_str()));
            }
        }
    }

    // ヘッダー行
    let header: Vec<String> = result.columns.iter().enumerate()
        .map(|(i, col)| pad_right(col, widths[i]))
        .collect();
    println!(" {} ", header.join(" | "));

    // 区切り線
    let sep: Vec<String> = widths.iter().map(|w| "-".repeat(w + 2)).collect();
    println!("{}", sep.join("+"));

    // データ行
    for row in &result.rows {
        let cells: Vec<String> = row.iter().enumerate()
            .map(|(i, cell)| pad_right(cell, widths.get(i).copied().unwrap_or(0)))
            .collect();
        println!(" {} ", cells.join(" | "));
    }

    // フッター
    let rows_count = result.rows.len();
    let auto_limit_notice = if auto_limited { "  [auto LIMIT]" } else { "" };
    println!("({} rows)  ({:.3}s){}", rows_count, result.duration_ms as f64 / 1000.0, auto_limit_notice);

    Ok(())
}

fn print_csv(result: &QueryResult) -> Result<()> {
    let mut wtr = csv::Writer::from_writer(std::io::stdout());
    wtr.write_record(&result.columns)?;
    for row in &result.rows {
        wtr.write_record(row)?;
    }
    wtr.flush()?;
    Ok(())
}

fn print_json(result: &QueryResult) -> Result<()> {
    let records: Vec<serde_json::Value> = result.rows.iter().map(|row| {
        let obj: serde_json::Map<String, serde_json::Value> = result.columns.iter()
            .zip(row.iter())
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        serde_json::Value::Object(obj)
    }).collect();

    println!("{}", serde_json::to_string_pretty(&records)?);
    Ok(())
}

/// Unicode 幅を考慮して右パディング
fn pad_right(s: &str, width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}
