use crate::db::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};
use anyhow::{Context, Result};
use sqlx::mysql::MySqlPoolOptions;
use sqlx::{Column, MySqlPool, Row};
use std::time::Instant;

pub struct MysqlAdapter {
    pool: Option<MySqlPool>,
    connection_url: String,
}

impl MysqlAdapter {
    pub fn new(
        host: String,
        port: u16,
        database: String,
        user: String,
        password: Option<String>,
    ) -> Self {
        let pw = password.as_deref().unwrap_or("");
        let encoded_pw = urlencoding::encode(pw);
        let connection_url = format!(
            "mysql://{}:{}@{}:{}/{}",
            user, encoded_pw, host, port, database
        );
        Self {
            pool: None,
            connection_url,
        }
    }
}

impl DbAdapter for MysqlAdapter {
    async fn connect(&mut self) -> Result<()> {
        let pool = MySqlPoolOptions::new()
            .max_connections(2)
            .connect(&self.connection_url)
            .await
            .context("MySQL への接続に失敗しました")?;

        // 疎通確認
        sqlx::query("SELECT 1")
            .execute(&pool)
            .await
            .context("MySQL 疎通確認に失敗しました")?;

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

        // カラム名を取得
        let columns: Vec<String> = rows[0]
            .columns()
            .iter()
            .map(|c| c.name().to_string())
            .collect();

        // 行データを文字列に変換（NULL は None として保持）
        let result_rows: Vec<Vec<Option<String>>> = rows
            .iter()
            .map(|row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, _)| get_mysql_value(row, i))
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

        let rows = sqlx::query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = DATABASE() ORDER BY table_name",
        )
        .fetch_all(pool)
        .await
        .context("テーブル一覧の取得に失敗しました")?;

        Ok(rows
            .iter()
            .map(|r| TableInfo {
                name: r.get::<String, _>("table_name"),
            })
            .collect())
    }

    async fn fetch_columns(&self, table: &str) -> Result<Vec<ColumnInfo>> {
        let pool = self
            .pool
            .as_ref()
            .context("データベースに接続されていません")?;

        let rows = sqlx::query(
            "SELECT column_name, column_type, column_key FROM information_schema.columns \
             WHERE table_schema = DATABASE() AND table_name = ? ORDER BY ordinal_position",
        )
        .bind(table)
        .fetch_all(pool)
        .await
        .context("カラム情報の取得に失敗しました")?;

        Ok(rows
            .iter()
            .map(|r| {
                let column_key: String = r.get::<String, _>("column_key");
                ColumnInfo {
                    name: r.get::<String, _>("column_name"),
                    col_type: r.get::<String, _>("column_type"),
                    is_primary_key: column_key.eq_ignore_ascii_case("PRI"),
                }
            })
            .collect())
    }
}

/// MySqlRow から表示用の値を取得する。NULL は `None`、それ以外は `Some(文字列)` を返す。
fn get_mysql_value(row: &sqlx::mysql::MySqlRow, index: usize) -> Option<String> {
    use sqlx::TypeInfo;
    use sqlx::ValueRef;

    let value_ref = row.try_get_raw(index).unwrap();
    if value_ref.is_null() {
        return None;
    }

    let type_info = value_ref.type_info();
    let type_name = type_info.name();

    let s = match type_name {
        "BOOLEAN" | "TINYINT(1)" => row
            .try_get::<bool, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" => row
            .try_get::<i32, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "BIGINT" => row
            .try_get::<i64, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "TINYINT UNSIGNED" | "SMALLINT UNSIGNED" | "MEDIUMINT UNSIGNED" | "INT UNSIGNED" => row
            .try_get::<u32, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "BIGINT UNSIGNED" => row
            .try_get::<u64, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "YEAR" => row
            .try_get::<u16, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "FLOAT" => row
            .try_get::<f32, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "DOUBLE" => row
            .try_get::<f64, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "DECIMAL" | "NUMERIC" => row
            .try_get::<sqlx::types::BigDecimal, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "DATE" => row
            .try_get::<sqlx::types::chrono::NaiveDate, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "TIME" => row
            .try_get::<sqlx::types::chrono::NaiveTime, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "DATETIME" => row
            .try_get::<sqlx::types::chrono::NaiveDateTime, _>(index)
            .map(|v| v.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_default(),
        "TIMESTAMP" => row
            .try_get::<sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>, _>(index)
            .map(|v| v.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_default(),
        "JSON" => row
            .try_get::<sqlx::types::JsonValue, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" | "BINARY" | "VARBINARY" => row
            .try_get::<Vec<u8>, _>(index)
            .map(|v| format_binary(&v))
            .unwrap_or_default(),
        _ => row
            .try_get::<String, _>(index)
            .unwrap_or_default(),
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
        assert_eq!(format_binary(&[0xde, 0xad, 0xbe, 0xef]), "0xdeadbeef");
    }

    #[test]
    fn format_binary_empty_returns_prefix_only() {
        assert_eq!(format_binary(&[]), "0x");
    }

    #[test]
    fn format_binary_truncates_long_bytes() {
        let bytes = vec![0xab; 100];
        let s = format_binary(&bytes);
        assert!(s.starts_with("0x"));
        assert!(s.contains("…(100 bytes)"));
        // 0x + 32 byte * 2 hex = 66 文字までは含まれる
        assert!(s.len() > 66);
    }
}
