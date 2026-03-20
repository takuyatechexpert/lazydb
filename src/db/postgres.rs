use crate::db::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};
use anyhow::{Context, Result};
use std::time::Instant;
use tokio::process::Command;

pub struct PostgresAdapter {
    host: String,
    port: u16,
    database: String,
    user: String,
    password: Option<String>,
}

impl PostgresAdapter {
    pub fn new(
        host: String,
        port: u16,
        database: String,
        user: String,
        password: Option<String>,
    ) -> Self {
        Self { host, port, database, user, password }
    }

    fn build_command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("psql");
        cmd.arg("-h").arg(&self.host)
            .arg("-p").arg(self.port.to_string())
            .arg("-U").arg(&self.user)
            .arg("-d").arg(&self.database)
            .arg("--no-psqlrc");

        if let Some(ref pw) = self.password {
            cmd.env("PGPASSWORD", pw);
        }

        for arg in args {
            cmd.arg(arg);
        }
        cmd
    }
}

impl DbAdapter for PostgresAdapter {
    async fn connect(&mut self) -> Result<()> {
        // psql で簡単な疎通確認
        let output = self
            .build_command(&["--csv", "-c", "SELECT 1"])
            .output()
            .await
            .context("psql の起動に失敗しました。psql がインストールされているか確認してください")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("接続に失敗しました: {}", stderr.trim());
        }
        Ok(())
    }

    async fn execute(&self, query: &str) -> Result<QueryResult> {
        let start = Instant::now();

        let output = self
            .build_command(&["--csv", "-c", query])
            .output()
            .await
            .context("psql の起動に失敗しました")?;

        let duration_ms = start.elapsed().as_millis() as u64;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("{}", stderr.trim());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_csv_output(&stdout, duration_ms)
    }

    async fn fetch_tables(&self) -> Result<Vec<TableInfo>> {
        let query = "SELECT table_name FROM information_schema.tables \
                     WHERE table_schema = 'public' ORDER BY table_name";
        let result = self.execute(query).await?;
        Ok(result.rows.into_iter().map(|r| TableInfo { name: r[0].clone() }).collect())
    }

    async fn fetch_columns(&self, table: &str) -> Result<Vec<ColumnInfo>> {
        let query = format!(
            "SELECT column_name, data_type FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = '{}' ORDER BY ordinal_position",
            table.replace('\'', "''")
        );
        let result = self.execute(&query).await?;
        Ok(result.rows.into_iter().map(|r| ColumnInfo {
            name: r[0].clone(),
            col_type: r[1].clone(),
        }).collect())
    }
}

fn parse_csv_output(stdout: &str, duration_ms: u64) -> Result<QueryResult> {
    if stdout.trim().is_empty() {
        return Ok(QueryResult { columns: vec![], rows: vec![], duration_ms });
    }

    let mut reader = csv::Reader::from_reader(stdout.as_bytes());

    let columns: Vec<String> = match reader.headers() {
        Ok(h) => h.iter().map(String::from).collect(),
        Err(_) => {
            // DML 結果（例: "INSERT 0 1"）は単一メッセージとして返す
            return Ok(QueryResult {
                columns: vec!["result".to_string()],
                rows: vec![vec![stdout.trim().to_string()]],
                duration_ms,
            });
        }
    };

    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.context("CSV のパースに失敗しました")?;
        rows.push(record.iter().map(String::from).collect());
    }

    Ok(QueryResult { columns, rows, duration_ms })
}
