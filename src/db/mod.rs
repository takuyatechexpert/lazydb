pub mod adapter;
pub mod mysql;
pub mod postgres;
pub mod redis;
pub mod sqlite;
mod statement;

#[cfg(test)]
mod tests;

use crate::config::connections::DbType;
use adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};
use anyhow::Result;
use statement::ReadKind;

/// PostgreSQL / MySQL / SQLite / Redis を統一的に扱うアダプター enum
pub enum AnyAdapter {
    Postgres(postgres::PostgresAdapter),
    Mysql(mysql::MysqlAdapter),
    Sqlite(sqlite::SqliteAdapter),
    Redis(redis::RedisAdapter),
}

impl AnyAdapter {
    pub async fn connect(&mut self) -> Result<()> {
        match self {
            AnyAdapter::Postgres(a) => adapter::DbAdapter::connect(a).await,
            AnyAdapter::Mysql(a) => adapter::DbAdapter::connect(a).await,
            AnyAdapter::Sqlite(a) => adapter::DbAdapter::connect(a).await,
            AnyAdapter::Redis(a) => adapter::DbAdapter::connect(a).await,
        }
    }

    pub async fn execute(&self, query: &str) -> Result<QueryResult> {
        match self {
            AnyAdapter::Postgres(a) => a.execute(query).await,
            AnyAdapter::Mysql(a) => a.execute(query).await,
            AnyAdapter::Sqlite(a) => a.execute(query).await,
            AnyAdapter::Redis(a) => a.execute(query).await,
        }
    }

    pub async fn fetch_tables(&self) -> Result<Vec<TableInfo>> {
        match self {
            AnyAdapter::Postgres(a) => a.fetch_tables().await,
            AnyAdapter::Mysql(a) => a.fetch_tables().await,
            AnyAdapter::Sqlite(a) => a.fetch_tables().await,
            AnyAdapter::Redis(a) => a.fetch_tables().await,
        }
    }

    pub async fn fetch_columns(&self, table: &str) -> Result<Vec<ColumnInfo>> {
        match self {
            AnyAdapter::Postgres(a) => a.fetch_columns(table).await,
            AnyAdapter::Mysql(a) => a.fetch_columns(table).await,
            AnyAdapter::Sqlite(a) => a.fetch_columns(table).await,
            AnyAdapter::Redis(a) => a.fetch_columns(table).await,
        }
    }

    /// Redis 等のコマンド指向アダプターかどうか
    /// （LimitApplier / SQL用 ReadonlyChecker をスキップする判定に使う）
    #[allow(dead_code)]
    pub fn is_redis(&self) -> bool {
        matches!(self, AnyAdapter::Redis(_))
    }
}

/// readonly 接続で実行できる Redis コマンド（読み取りだけのもの）
/// 一覧にないコマンドはすべて弾く
const REDIS_READ_COMMANDS: &[&str] = &[
    "EXISTS", "TYPE", "TTL", "PTTL", "EXPIRETIME", "PEXPIRETIME", "KEYS", "SCAN", "RANDOMKEY",
    "DBSIZE", "DUMP", "OBJECT", "TOUCH", "SORT", "SORT_RO",
    "GET", "MGET", "STRLEN", "GETRANGE", "SUBSTR", "LCS", "GETBIT", "BITCOUNT", "BITPOS",
    "BITFIELD_RO",
    "LRANGE", "LLEN", "LINDEX", "LPOS",
    "SMEMBERS", "SISMEMBER", "SMISMEMBER", "SCARD", "SRANDMEMBER", "SINTER", "SINTERCARD",
    "SUNION", "SDIFF", "SSCAN",
    "HGET", "HMGET", "HGETALL", "HKEYS", "HVALS", "HLEN", "HEXISTS", "HSTRLEN", "HRANDFIELD",
    "HSCAN", "HTTL", "HPTTL", "HEXPIRETIME", "HPEXPIRETIME",
    "ZRANGE", "ZRANGEBYSCORE", "ZRANGEBYLEX", "ZREVRANGE", "ZREVRANGEBYSCORE", "ZREVRANGEBYLEX",
    "ZSCORE", "ZMSCORE", "ZRANK", "ZREVRANK", "ZCARD", "ZCOUNT", "ZLEXCOUNT", "ZRANDMEMBER",
    "ZSCAN", "ZINTER", "ZUNION", "ZDIFF", "ZINTERCARD",
    "XRANGE", "XREVRANGE", "XLEN", "XREAD", "XINFO", "XPENDING",
    "PFCOUNT",
    "GEOPOS", "GEODIST", "GEOHASH", "GEOSEARCH", "GEORADIUS", "GEORADIUSBYMEMBER",
    "GEORADIUS_RO", "GEORADIUSBYMEMBER_RO",
    "PING", "ECHO", "SELECT", "INFO", "TIME", "LASTSAVE", "ROLE", "COMMAND", "MEMORY", "PUBSUB",
];

/// 引数に STORE / STOREDIST を付けると、結果をキーに書き込む読み取りコマンド
const REDIS_STORE_OPTION_COMMANDS: &[&str] = &["SORT", "GEORADIUS", "GEORADIUSBYMEMBER"];

/// SELECT 文に LIMIT がない場合に自動付与する
pub struct LimitApplier {
    pub default_limit: u64,
}

impl LimitApplier {
    /// クエリに LIMIT を付与する。付与した場合は true を返す
    pub fn apply(&self, query: &str, db_type: &DbType) -> (String, bool) {
        if self.default_limit == 0 {
            return (query.to_string(), false);
        }

        // SELECT の文 1 つだけが対象（WITH の本体が SELECT のものを含む）。
        // WITH ... DELETE に付けると、PostgreSQL では構文エラー、MySQL では削除件数の制限になる
        if statement::classify(query, db_type) != Ok(vec![ReadKind::Select]) {
            return (query.to_string(), false);
        }

        let trimmed = query.trim();
        let upper = trimmed.to_uppercase();

        // フォーマット済みクエリ（改行を含む）でも検出できるよう、
        // 連続する空白類をすべて単一スペースに正規化してから判定する
        let normalized = format!(" {} ", upper.split_whitespace().collect::<Vec<_>>().join(" "));

        // すでに LIMIT / FETCH FIRST がある場合はスキップ
        if normalized.contains(" LIMIT ") || normalized.contains(" FETCH FIRST ") {
            return (query.to_string(), false);
        }

        // 末尾のセミコロンを除去して LIMIT を付与
        let without_semi = trimmed.trim_end_matches(';').trim_end();
        (format!("{} LIMIT {}", without_semi, self.default_limit), true)
    }
}

/// readonly 接続で、読み取り以外の文をブロックする
///
/// 読み取りの文だけを通す許可リスト方式。
/// 本当のガードは DB 側の読み取り専用化（各アダプター）で、ここは実行前に分かりやすいエラーを出すための判定。
pub struct ReadonlyChecker;

impl ReadonlyChecker {
    pub fn check(&self, query: &str, db_type: &DbType) -> Result<()> {
        if let Err(keyword) = statement::classify(query, db_type) {
            anyhow::bail!("読み取り専用接続では '{}' を実行できません", keyword);
        }
        Ok(())
    }
}

/// Redis 用の readonly チェッカー
///
/// 読み取りコマンドの許可リスト方式。
/// コマンド名は実行時と同じトークナイザで取り出す（`"DEL" foo` もクォートが外れて DEL として実行されるため）。
pub struct RedisReadonlyChecker;

impl RedisReadonlyChecker {
    pub fn check(&self, query: &str) -> Result<()> {
        let tokens = self::redis::tokenize(query)?;
        let Some(command) = tokens.first() else {
            return Ok(());
        };
        let command = command.to_uppercase();

        if !REDIS_READ_COMMANDS.contains(&command.as_str()) {
            anyhow::bail!(
                "読み取り専用接続では Redis コマンド '{}' を実行できません",
                command
            );
        }
        if REDIS_STORE_OPTION_COMMANDS.contains(&command.as_str())
            && tokens[1..].iter().any(|arg| {
                arg.eq_ignore_ascii_case("STORE") || arg.eq_ignore_ascii_case("STOREDIST")
            })
        {
            anyhow::bail!(
                "読み取り専用接続では Redis コマンド '{} ... STORE' を実行できません",
                command
            );
        }
        Ok(())
    }
}
