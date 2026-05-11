use crate::config::connections::expand_tilde;
use crate::db::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};
use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Column, Row, SqlitePool};
use std::str::FromStr;
use std::time::Instant;

pub struct SqliteAdapter {
    pool: Option<SqlitePool>,
    /// 解決済みのファイルパス（チルダ展開・絶対化後）または `:memory:`
    path: String,
    readonly: bool,
}

impl SqliteAdapter {
    pub fn new(path: String, readonly: bool) -> Self {
        // チルダ展開のみ行い、相対パスは現状のまま扱う（sqlx 側で解決される）
        let resolved = if path == ":memory:" {
            path
        } else {
            expand_tilde(&path).to_string_lossy().to_string()
        };
        Self {
            pool: None,
            path: resolved,
            readonly,
        }
    }
}

impl DbAdapter for SqliteAdapter {
    async fn connect(&mut self) -> Result<()> {
        // create_if_missing(false) でファイル不在時は明示エラーにする。
        // SQLite では URI クエリ経由でも設定できるが、OS パス（スペース等）対応のため
        // SqliteConnectOptions を直接組み立てる。
        let mut opts = if self.path == ":memory:" {
            SqliteConnectOptions::from_str("sqlite::memory:")
                .context("SQLite メモリ DB の接続オプションを解析できません")?
        } else {
            SqliteConnectOptions::new()
                .filename(&self.path)
                .create_if_missing(false)
        };
        opts = opts.read_only(self.readonly);

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(opts)
            .await
            .with_context(|| format!("SQLite への接続に失敗しました: {}", self.path))?;

        // 疎通確認
        sqlx::query("SELECT 1")
            .execute(&pool)
            .await
            .context("SQLite 疎通確認に失敗しました")?;

        self.pool = Some(pool);
        Ok(())
    }

    async fn execute(&self, query: &str) -> Result<QueryResult> {
        let pool = self
            .pool
            .as_ref()
            .context("データベースに接続されていません")?;

        let start = Instant::now();

        let rows = sqlx::query(query)
            .fetch_all(pool)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let duration_ms = start.elapsed().as_millis() as u64;

        if rows.is_empty() {
            return Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                duration_ms,
            });
        }

        let columns: Vec<String> = rows[0]
            .columns()
            .iter()
            .map(|c| c.name().to_string())
            .collect();

        let result_rows: Vec<Vec<Option<String>>> = rows
            .iter()
            .map(|row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, _)| get_sqlite_value(row, i))
                    .collect()
            })
            .collect();

        Ok(QueryResult {
            columns,
            rows: result_rows,
            duration_ms,
        })
    }

    async fn fetch_tables(&self) -> Result<Vec<TableInfo>> {
        let pool = self
            .pool
            .as_ref()
            .context("データベースに接続されていません")?;

        // sqlite_% で始まる内部テーブルは除外する
        let rows = sqlx::query(
            "SELECT name FROM sqlite_master \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
             ORDER BY name",
        )
        .fetch_all(pool)
        .await
        .context("テーブル一覧の取得に失敗しました")?;

        Ok(rows
            .iter()
            .map(|r| TableInfo {
                name: r.get::<String, _>("name"),
            })
            .collect())
    }

    async fn fetch_columns(&self, table: &str) -> Result<Vec<ColumnInfo>> {
        let pool = self
            .pool
            .as_ref()
            .context("データベースに接続されていません")?;

        // PRAGMA table_info はバインドパラメータ非対応のため、識別子をリテラル展開する。
        // 安全のためダブルクオートをエスケープして "..." で包む。
        let quoted = format!("\"{}\"", table.replace('"', "\"\""));
        let sql = format!("PRAGMA table_info({})", quoted);

        let rows = sqlx::query(&sql)
            .fetch_all(pool)
            .await
            .context("カラム情報の取得に失敗しました")?;

        Ok(rows
            .iter()
            .map(|r| {
                let pk: i64 = r.try_get::<i64, _>("pk").unwrap_or(0);
                ColumnInfo {
                    name: r.get::<String, _>("name"),
                    col_type: r.get::<String, _>("type"),
                    is_primary_key: pk > 0,
                }
            })
            .collect())
    }
}

/// SqliteRow から表示用の値を取得する。NULL は `None`、それ以外は `Some(文字列)` を返す。
fn get_sqlite_value(row: &sqlx::sqlite::SqliteRow, index: usize) -> Option<String> {
    use sqlx::TypeInfo;
    use sqlx::ValueRef;

    let value_ref = row.try_get_raw(index).unwrap();
    if value_ref.is_null() {
        return None;
    }

    let type_info = value_ref.type_info();
    let type_name = type_info.name();

    // SQLite は動的型付け。代表的な型のみ専用処理し、それ以外は文字列フォールバック。
    let s = match type_name {
        "BOOLEAN" => row
            .try_get::<bool, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "INTEGER" | "INT" | "INT8" => row
            .try_get::<i64, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "REAL" | "DOUBLE" | "FLOAT" => row
            .try_get::<f64, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "BLOB" => row
            .try_get::<Vec<u8>, _>(index)
            .map(|v| format_binary(&v))
            .unwrap_or_default(),
        _ => row.try_get::<String, _>(index).unwrap_or_default(),
    };
    Some(s)
}

/// バイナリデータを 16 進文字列で表示する（先頭 32 byte で打ち切り）
fn format_binary(bytes: &[u8]) -> String {
    const MAX: usize = 32;
    let head: String = bytes
        .iter()
        .take(MAX)
        .map(|b| format!("{:02x}", b))
        .collect();
    if bytes.len() > MAX {
        format!("0x{}…({} bytes)", head, bytes.len())
    } else {
        format!("0x{}", head)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_binary_short_returns_hex_with_prefix() {
        assert_eq!(format_binary(&[0xca, 0xfe]), "0xcafe");
    }

    #[test]
    fn format_binary_empty_returns_prefix_only() {
        assert_eq!(format_binary(&[]), "0x");
    }

    #[test]
    fn format_binary_truncates_long_bytes() {
        let bytes = vec![0x10; 64];
        let s = format_binary(&bytes);
        assert!(s.contains("…(64 bytes)"));
    }

    #[tokio::test]
    async fn memory_db_round_trip() {
        let mut adapter = SqliteAdapter::new(":memory:".to_string(), false);
        adapter.connect().await.unwrap();

        adapter
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .await
            .unwrap();
        adapter
            .execute("INSERT INTO users (id, name) VALUES (1, 'alice'), (2, 'bob')")
            .await
            .unwrap();

        let tables = adapter.fetch_tables().await.unwrap();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "users");

        let columns = adapter.fetch_columns("users").await.unwrap();
        assert_eq!(columns.len(), 2);
        assert!(columns.iter().any(|c| c.name == "id" && c.is_primary_key));
        assert!(columns.iter().any(|c| c.name == "name" && !c.is_primary_key));

        let result = adapter
            .execute("SELECT id, name FROM users ORDER BY id")
            .await
            .unwrap();
        assert_eq!(result.columns, vec!["id".to_string(), "name".to_string()]);
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0][0].as_deref(), Some("1"));
        assert_eq!(result.rows[0][1].as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn null_is_returned_as_none() {
        let mut adapter = SqliteAdapter::new(":memory:".to_string(), false);
        adapter.connect().await.unwrap();
        adapter
            .execute("CREATE TABLE t (a TEXT)")
            .await
            .unwrap();
        adapter.execute("INSERT INTO t (a) VALUES (NULL)").await.unwrap();

        let result = adapter.execute("SELECT a FROM t").await.unwrap();
        assert_eq!(result.rows[0][0], None);
    }
}
