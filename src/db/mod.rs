pub mod adapter;
pub mod mysql;
pub mod postgres;

#[cfg(test)]
mod tests;

use adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};
use anyhow::Result;

/// PostgreSQL / MySQL を統一的に扱うアダプター enum
pub enum AnyAdapter {
    Postgres(postgres::PostgresAdapter),
    Mysql(mysql::MysqlAdapter),
}

impl AnyAdapter {
    pub async fn connect(&mut self) -> Result<()> {
        match self {
            AnyAdapter::Postgres(a) => adapter::DbAdapter::connect(a).await,
            AnyAdapter::Mysql(a) => adapter::DbAdapter::connect(a).await,
        }
    }

    pub async fn execute(&self, query: &str) -> Result<QueryResult> {
        match self {
            AnyAdapter::Postgres(a) => a.execute(query).await,
            AnyAdapter::Mysql(a) => a.execute(query).await,
        }
    }

    pub async fn fetch_tables(&self) -> Result<Vec<TableInfo>> {
        match self {
            AnyAdapter::Postgres(a) => a.fetch_tables().await,
            AnyAdapter::Mysql(a) => a.fetch_tables().await,
        }
    }

    pub async fn fetch_columns(&self, table: &str) -> Result<Vec<ColumnInfo>> {
        match self {
            AnyAdapter::Postgres(a) => a.fetch_columns(table).await,
            AnyAdapter::Mysql(a) => a.fetch_columns(table).await,
        }
    }
}

const WRITE_KEYWORDS: &[&str] = &[
    "INSERT", "UPDATE", "DELETE", "TRUNCATE", "DROP", "CREATE", "ALTER", "RENAME", "REPLACE",
];

/// SELECT 文に LIMIT がない場合に自動付与する
pub struct LimitApplier {
    pub default_limit: u64,
}

impl LimitApplier {
    /// クエリに LIMIT を付与する。付与した場合は true を返す
    pub fn apply(&self, query: &str) -> (String, bool) {
        if self.default_limit == 0 {
            return (query.to_string(), false);
        }

        let trimmed = query.trim();
        let upper = trimmed.to_uppercase();

        // SELECT / WITH で始まるクエリのみ対象
        if !upper.starts_with("SELECT") && !upper.starts_with("WITH") {
            return (query.to_string(), false);
        }

        // フォーマット済みクエリ（改行を含む）でも検出できるよう、
        // 連続する空白類をすべて単一スペースに正規化してから判定する
        let normalized = format!(" {} ", upper.split_whitespace().collect::<Vec<_>>().join(" "));

        // すでに LIMIT / FETCH FIRST / TOP がある場合はスキップ
        if normalized.contains(" LIMIT ")
            || normalized.contains(" FETCH FIRST ")
            || normalized.contains(" TOP ")
        {
            return (query.to_string(), false);
        }

        // 末尾のセミコロンを除去して LIMIT を付与
        let without_semi = trimmed.trim_end_matches(';').trim_end();
        (format!("{} LIMIT {}", without_semi, self.default_limit), true)
    }
}

/// readonly 接続で書き込みクエリをブロックする
pub struct ReadonlyChecker;

impl ReadonlyChecker {
    pub fn check(&self, query: &str) -> Result<()> {
        let first_word = query
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_uppercase();

        if WRITE_KEYWORDS.contains(&first_word.as_str()) {
            anyhow::bail!(
                "読み取り専用接続では '{}' を実行できません",
                first_word
            );
        }
        Ok(())
    }
}
