use anyhow::Result;

/// クエリ結果
///
/// `rows[r][c]` は NULL を `None`、それ以外を `Some(表示用文字列)` で保持する。
/// 表示・export・cc UPDATE 生成側で NULL を区別して扱えるようにするため。
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
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
    pub is_primary_key: bool,
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
