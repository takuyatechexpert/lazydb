use crate::db::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};
use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Column, PgPool, Row};
use std::time::Instant;

pub struct PostgresAdapter {
    pool: Option<PgPool>,
    connection_url: String,
    /// 接続時の current_schema。未接続時は "public" を既定値として保持。
    schema: String,
}

impl PostgresAdapter {
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
            "postgres://{}:{}@{}:{}/{}?sslmode=prefer",
            user, encoded_pw, host, port, database
        );
        Self {
            pool: None,
            connection_url,
            schema: "public".to_string(),
        }
    }
}

impl DbAdapter for PostgresAdapter {
    async fn connect(&mut self) -> Result<()> {
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&self.connection_url)
            .await
            .context("PostgreSQL への接続に失敗しました")?;

        // 疎通確認
        sqlx::query("SELECT 1")
            .execute(&pool)
            .await
            .context("PostgreSQL 疎通確認に失敗しました")?;

        // search_path 先頭のスキーマを取得。search_path が空等で NULL が返る
        // ケースに備え、取得失敗時は "public" にフォールバックする。
        let current: Option<String> = sqlx::query_scalar("SELECT current_schema()::text")
            .fetch_one(&pool)
            .await
            .context("current_schema の取得に失敗しました")?;
        self.schema = current.unwrap_or_else(|| "public".to_string());

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
            // DML の場合や結果が空の場合
            // カラム情報が取れるか試す
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

        // 行データを文字列に変換
        let result_rows: Vec<Vec<String>> = rows
            .iter()
            .map(|row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, _)| get_pg_value_as_string(row, i))
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
             WHERE table_schema = $1 ORDER BY table_name",
        )
        .bind(&self.schema)
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
            "SELECT \
               c.column_name, \
               c.data_type, \
               CASE WHEN EXISTS ( \
                 SELECT 1 \
                 FROM pg_index i \
                 JOIN pg_class cls ON cls.oid = i.indrelid \
                 JOIN pg_namespace ns ON ns.oid = cls.relnamespace \
                 JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey) \
                 WHERE i.indisprimary \
                   AND ns.nspname = c.table_schema \
                   AND cls.relname = c.table_name \
                   AND a.attname = c.column_name \
               ) THEN TRUE ELSE FALSE END AS is_primary_key \
             FROM information_schema.columns c \
             WHERE c.table_schema = $1 AND c.table_name = $2 \
             ORDER BY c.ordinal_position",
        )
        .bind(&self.schema)
        .bind(table)
        .fetch_all(pool)
        .await
        .context("カラム情報の取得に失敗しました")?;

        Ok(rows
            .iter()
            .map(|r| ColumnInfo {
                name: r.get::<String, _>("column_name"),
                col_type: r.get::<String, _>("data_type"),
                is_primary_key: r.get::<bool, _>("is_primary_key"),
            })
            .collect())
    }
}

/// PgRow から文字列として値を取得する
fn get_pg_value_as_string(row: &sqlx::postgres::PgRow, index: usize) -> String {
    use sqlx::TypeInfo;
    use sqlx::ValueRef;

    let value_ref = row.try_get_raw(index).unwrap();
    if value_ref.is_null() {
        return String::new();
    }

    let type_info = value_ref.type_info();
    let type_name = type_info.name();

    match type_name {
        "BOOL" => row
            .try_get::<bool, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "INT2" => row
            .try_get::<i16, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "INT4" => row
            .try_get::<i32, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "INT8" => row
            .try_get::<i64, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "FLOAT4" => row
            .try_get::<f32, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "FLOAT8" | "NUMERIC" => row
            .try_get::<f64, _>(index)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        _ => row
            .try_get::<String, _>(index)
            .unwrap_or_default(),
    }
}
