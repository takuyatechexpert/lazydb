use crate::db::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};
use anyhow::{Context, Result};
use redis::aio::MultiplexedConnection;
use redis::{Client, Value};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// Redis アダプター
///
/// SQL ではなく Redis コマンド文字列をそのまま `execute()` に渡す。
/// 例: `GET foo` / `KEYS user:*` / `HGETALL myhash` / `SELECT 0`
pub struct RedisAdapter {
    connection_url: String,
    /// SELECT n でセッション中に切り替えられた現在の DB 番号
    /// （fetch_columns 等のサンプル取得時に同じ DB を使うため保持）
    conn: Option<Arc<Mutex<MultiplexedConnection>>>,
}

impl RedisAdapter {
    pub fn new(
        host: String,
        port: u16,
        database: String,
        _user: String,
        password: Option<String>,
    ) -> Self {
        // database は数字（0〜15 など）として解釈。空または非数値なら 0。
        let db_index: u32 = database.trim().parse().unwrap_or(0);
        let auth = match password.as_deref() {
            Some(pw) if !pw.is_empty() => format!(":{}@", urlencoding::encode(pw)),
            _ => String::new(),
        };
        let connection_url = format!("redis://{}{}:{}/{}", auth, host, port, db_index);
        Self {
            connection_url,
            conn: None,
        }
    }
}

impl DbAdapter for RedisAdapter {
    async fn connect(&mut self) -> Result<()> {
        let client =
            Client::open(self.connection_url.as_str()).context("Redis URL のパースに失敗しました")?;
        let conn = client
            .get_multiplexed_async_connection()
            .await
            .context("Redis への接続に失敗しました")?;

        // 疎通確認
        let mut conn_check = conn.clone();
        let _: String = redis::cmd("PING")
            .query_async(&mut conn_check)
            .await
            .context("Redis 疎通確認 (PING) に失敗しました")?;

        self.conn = Some(Arc::new(Mutex::new(conn)));
        Ok(())
    }

    async fn execute(&self, query: &str) -> Result<QueryResult> {
        let conn = self
            .conn
            .as_ref()
            .context("Redis に接続されていません")?
            .clone();

        let tokens = tokenize(query)?;
        if tokens.is_empty() {
            anyhow::bail!("空のコマンドです");
        }

        let cmd_name = tokens[0].to_uppercase();
        let mut cmd = redis::cmd(&cmd_name);
        for arg in &tokens[1..] {
            cmd.arg(arg.as_str());
        }

        let start = Instant::now();
        let value: Value = {
            let mut guard = conn.lock().await;
            cmd.query_async(&mut *guard)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?
        };
        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(format_value(&cmd_name, value, duration_ms))
    }

    async fn fetch_tables(&self) -> Result<Vec<TableInfo>> {
        // Redis では論理 DB 番号 0〜15 を「テーブル」相当として扱う
        // CONFIG GET databases で実際の DB 数を取得できるが、未認可ユーザーで失敗するため
        // まず CONFIG GET を試み、失敗時は 16 個（デフォルト）にフォールバックする
        let conn = self
            .conn
            .as_ref()
            .context("Redis に接続されていません")?
            .clone();

        let db_count: u32 = {
            let mut guard = conn.lock().await;
            let res: Result<Vec<String>, _> = redis::cmd("CONFIG")
                .arg("GET")
                .arg("databases")
                .query_async(&mut *guard)
                .await;
            match res {
                Ok(v) if v.len() >= 2 => v[1].parse().unwrap_or(16),
                _ => 16,
            }
        };

        Ok((0..db_count)
            .map(|i| TableInfo {
                name: format!("db{}", i),
            })
            .collect())
    }

    async fn fetch_columns(&self, table: &str) -> Result<Vec<ColumnInfo>> {
        // テーブル名は "db<N>" 形式。SCAN でサンプルキーを取得して列扱いで返す
        let db_index: u32 = table
            .trim_start_matches("db")
            .parse()
            .with_context(|| format!("不正な DB 名: {}", table))?;

        let conn = self
            .conn
            .as_ref()
            .context("Redis に接続されていません")?
            .clone();

        let mut guard = conn.lock().await;

        // SELECT で対象 DB に切り替え（このコネクションのみに影響）
        let _: redis::RedisResult<String> = redis::cmd("SELECT")
            .arg(db_index)
            .query_async(&mut *guard)
            .await;

        // SCAN でサンプルキーを最大 50 件取得
        let scan_res: redis::RedisResult<(String, Vec<String>)> = redis::cmd("SCAN")
            .arg(0)
            .arg("COUNT")
            .arg(50)
            .query_async(&mut *guard)
            .await;

        let keys = scan_res.map(|(_, k)| k).unwrap_or_default();

        // 型情報を TYPE コマンドで補完（取れなければ "unknown"）
        let mut columns = Vec::with_capacity(keys.len());
        for key in keys.iter().take(50) {
            let type_res: redis::RedisResult<String> = redis::cmd("TYPE")
                .arg(key)
                .query_async(&mut *guard)
                .await;
            columns.push(ColumnInfo {
                name: key.clone(),
                col_type: type_res.unwrap_or_else(|_| "unknown".to_string()),
                is_primary_key: false,
            });
        }

        Ok(columns)
    }
}

/// Redis コマンド文字列を引数列にトークン化する
///
/// シェル風のクォート（`"..."` / `'...'`）に最低限対応する。
/// 例: `SET greeting "hello world"` → ["SET", "greeting", "hello world"]
pub fn tokenize(input: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            '\\' if in_double => {
                if let Some(&next) = chars.peek() {
                    chars.next();
                    current.push(next);
                }
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }

    if in_single || in_double {
        anyhow::bail!("クォートが閉じていません");
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    // 末尾のセミコロンを許容（SQL クライアント由来の癖を受け入れる）
    if let Some(last) = tokens.last_mut() {
        if last.ends_with(';') {
            last.pop();
            if last.is_empty() {
                tokens.pop();
            }
        }
    }

    Ok(tokens)
}

/// Redis の Value を QueryResult に整形する
///
/// - Status / Okay / Int / BulkString / Nil → 1 行 1 列（"value"）
/// - Array → idx + value の 2 列。HGETALL 等は field/value のペア表に整形
fn format_value(cmd: &str, value: Value, duration_ms: u64) -> QueryResult {
    // HGETALL / CONFIG GET など、要素ペアを field/value 表として表示するコマンド
    let is_pair_cmd = matches!(
        cmd.to_uppercase().as_str(),
        "HGETALL" | "HMGET" | "CONFIG"
    );

    match value {
        Value::Nil => QueryResult {
            columns: vec!["value".to_string()],
            rows: vec![vec![None]],
            duration_ms,
        },
        Value::Int(n) => QueryResult {
            columns: vec!["value".to_string()],
            rows: vec![vec![Some(n.to_string())]],
            duration_ms,
        },
        Value::BulkString(bytes) => QueryResult {
            columns: vec!["value".to_string()],
            rows: vec![vec![Some(String::from_utf8_lossy(&bytes).to_string())]],
            duration_ms,
        },
        Value::SimpleString(s) => QueryResult {
            columns: vec!["status".to_string()],
            rows: vec![vec![Some(s)]],
            duration_ms,
        },
        Value::Okay => QueryResult {
            columns: vec!["status".to_string()],
            rows: vec![vec![Some("OK".to_string())]],
            duration_ms,
        },
        Value::Array(items) => {
            if is_pair_cmd && items.len() % 2 == 0 && !items.is_empty() {
                let mut rows = Vec::with_capacity(items.len() / 2);
                let mut iter = items.into_iter();
                while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
                    rows.push(vec![Some(value_to_string(k)), Some(value_to_string(v))]);
                }
                QueryResult {
                    columns: vec!["field".to_string(), "value".to_string()],
                    rows,
                    duration_ms,
                }
            } else {
                let rows: Vec<Vec<Option<String>>> = items
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| vec![Some(i.to_string()), Some(value_to_string(v))])
                    .collect();
                QueryResult {
                    columns: vec!["idx".to_string(), "value".to_string()],
                    rows,
                    duration_ms,
                }
            }
        }
        // Map / Set / 他の RESP3 系は文字列化して 1 行に詰める
        other => QueryResult {
            columns: vec!["value".to_string()],
            rows: vec![vec![Some(value_to_string(other))]],
            duration_ms,
        },
    }
}

/// redis::Value を表示用文字列へ平坦化する（ネスト時のフォールバック用）
fn value_to_string(v: Value) -> String {
    match v {
        Value::Nil => "(nil)".to_string(),
        Value::Int(n) => n.to_string(),
        Value::BulkString(b) => String::from_utf8_lossy(&b).to_string(),
        Value::SimpleString(s) => s,
        Value::Okay => "OK".to_string(),
        Value::Array(items) => {
            let parts: Vec<String> = items.into_iter().map(value_to_string).collect();
            format!("[{}]", parts.join(", "))
        }
        other => format!("{:?}", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_simple_command() {
        let t = tokenize("GET foo").unwrap();
        assert_eq!(t, vec!["GET", "foo"]);
    }

    #[test]
    fn tokenize_double_quoted_arg() {
        let t = tokenize(r#"SET greeting "hello world""#).unwrap();
        assert_eq!(t, vec!["SET", "greeting", "hello world"]);
    }

    #[test]
    fn tokenize_single_quoted_arg() {
        let t = tokenize("SET key 'a b c'").unwrap();
        assert_eq!(t, vec!["SET", "key", "a b c"]);
    }

    #[test]
    fn tokenize_escaped_quote_in_double() {
        let t = tokenize(r#"SET key "a\"b""#).unwrap();
        assert_eq!(t, vec!["SET", "key", "a\"b"]);
    }

    #[test]
    fn tokenize_trailing_semicolon_dropped() {
        let t = tokenize("PING;").unwrap();
        assert_eq!(t, vec!["PING"]);
    }

    #[test]
    fn tokenize_unclosed_quote_errors() {
        assert!(tokenize(r#"SET k "unclosed"#).is_err());
    }

    #[test]
    fn format_int_value() {
        let r = format_value("DBSIZE", Value::Int(42), 1);
        assert_eq!(r.columns, vec!["value"]);
        assert_eq!(r.rows, vec![vec![Some("42".to_string())]]);
    }

    #[test]
    fn format_bulk_string_value() {
        let r = format_value(
            "GET",
            Value::BulkString(b"hello".to_vec()),
            1,
        );
        assert_eq!(r.rows, vec![vec![Some("hello".to_string())]]);
    }

    #[test]
    fn format_nil_value_as_none() {
        let r = format_value("GET", Value::Nil, 1);
        assert_eq!(r.rows, vec![vec![None]]);
    }

    #[test]
    fn format_okay_value() {
        let r = format_value("SET", Value::Okay, 1);
        assert_eq!(r.columns, vec!["status"]);
        assert_eq!(r.rows, vec![vec![Some("OK".to_string())]]);
    }

    #[test]
    fn format_array_keys_as_idx_value() {
        let r = format_value(
            "KEYS",
            Value::Array(vec![
                Value::BulkString(b"foo".to_vec()),
                Value::BulkString(b"bar".to_vec()),
            ]),
            1,
        );
        assert_eq!(r.columns, vec!["idx", "value"]);
        assert_eq!(r.rows.len(), 2);
        assert_eq!(r.rows[0], vec![Some("0".into()), Some("foo".into())]);
        assert_eq!(r.rows[1], vec![Some("1".into()), Some("bar".into())]);
    }

    #[test]
    fn format_hgetall_as_field_value_pairs() {
        let r = format_value(
            "HGETALL",
            Value::Array(vec![
                Value::BulkString(b"name".to_vec()),
                Value::BulkString(b"sora".to_vec()),
                Value::BulkString(b"role".to_vec()),
                Value::BulkString(b"ai".to_vec()),
            ]),
            1,
        );
        assert_eq!(r.columns, vec!["field", "value"]);
        assert_eq!(r.rows.len(), 2);
        assert_eq!(r.rows[0], vec![Some("name".into()), Some("sora".into())]);
        assert_eq!(r.rows[1], vec![Some("role".into()), Some("ai".into())]);
    }
}
