use anyhow::Result;

/// クエリ結果
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub duration_ms: u64,
}

/// テーブル情報
#[derive(Debug, Clone)]
pub struct TableInfo {
    pub name: String,
}

/// カラム情報
#[derive(Debug, Clone)]
pub struct ColumnInfo {
    pub name: String,
    pub col_type: String,
}

/// DB アダプタートレイト
pub trait DbAdapter: Send + Sync {
    fn connect(&mut self) -> impl std::future::Future<Output = Result<()>> + Send;
    fn execute(&self, query: &str) -> impl std::future::Future<Output = Result<QueryResult>> + Send;
    fn fetch_tables(&self) -> impl std::future::Future<Output = Result<Vec<TableInfo>>> + Send;
    fn fetch_columns(
        &self,
        table: &str,
    ) -> impl std::future::Future<Output = Result<Vec<ColumnInfo>>> + Send;
}
