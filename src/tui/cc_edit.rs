use crate::tui::schema::SchemaState;
use ratatui::style::Color;

/// cc 機能の利用可否
#[derive(Debug, Clone, PartialEq)]
pub enum CcEligibility {
    /// 利用可能。UPDATE 文を生成できる
    Ok {
        table: String,
        pk_columns: Vec<String>,
    },
    /// 空クエリまたは SELECT ではない
    NotSelect,
    /// JOIN を含む（カンマ結合も含む）
    MultipleTables,
    /// サブクエリを含む
    HasSubquery,
    /// 式カラム・関数呼び出しを含む
    HasExpression,
    /// 対象テーブルに PK がない
    NoPrimaryKey,
    /// スキーマキャッシュに未読込（Schema パネルで展開していない）
    ColumnsNotLoaded,
}

impl CcEligibility {
    /// フッター表示ラベル
    pub fn label(&self) -> &'static str {
        match self {
            CcEligibility::Ok { .. } => "[cc OK]",
            CcEligibility::NotSelect => "[cc:SELECT限定]",
            CcEligibility::MultipleTables => "[cc:JOIN不可]",
            CcEligibility::HasSubquery => "[cc:サブクエリ不可]",
            CcEligibility::HasExpression => "[cc:式カラム不可]",
            CcEligibility::NoPrimaryKey => "[cc:PKなし]",
            CcEligibility::ColumnsNotLoaded => "[cc:スキーマ未読込]",
        }
    }

    /// ラベル色
    pub fn label_color(&self) -> Color {
        match self {
            CcEligibility::Ok { .. } => Color::Green,
            CcEligibility::NotSelect | CcEligibility::ColumnsNotLoaded => Color::DarkGray,
            CcEligibility::MultipleTables
            | CcEligibility::HasSubquery
            | CcEligibility::HasExpression => Color::Yellow,
            CcEligibility::NoPrimaryKey => Color::Red,
        }
    }

    /// 編集可能か
    #[allow(dead_code)]
    pub fn is_ok(&self) -> bool {
        matches!(self, CcEligibility::Ok { .. })
    }

    /// ステータスメッセージ用（詳細理由）
    pub fn status_reason(&self) -> &'static str {
        match self {
            CcEligibility::Ok { .. } => "",
            CcEligibility::NotSelect => "cc は単一テーブルの SELECT 結果でのみ使用できます",
            CcEligibility::MultipleTables => "cc は JOIN を含むクエリでは使用できません",
            CcEligibility::HasSubquery => "cc はサブクエリを含むクエリでは使用できません",
            CcEligibility::HasExpression => "cc は式カラムを含む SELECT では使用できません",
            CcEligibility::NoPrimaryKey => "対象テーブルに PK がないため cc は使用できません",
            CcEligibility::ColumnsNotLoaded => {
                "Schema パネルでテーブルを展開してから再度実行してください"
            }
        }
    }
}

/// クエリの簡易解析結果
#[derive(Debug, Clone, PartialEq)]
pub struct CcAnalysis {
    pub is_select: bool,
    pub table: Option<String>,
    pub has_join: bool,
    pub has_subquery: bool,
    pub has_expression: bool,
}

impl CcAnalysis {
    pub fn from_query(query: &str) -> Self {
        let trimmed = query.trim().trim_end_matches(';').trim();
        if trimmed.is_empty() {
            return Self::empty();
        }

        let upper = trimmed.to_ascii_uppercase();
        let is_select = upper.starts_with("SELECT") || upper.starts_with("SELECT\t");
        if !is_select {
            // "SELECT" で始まらない場合も厳密にチェック（先頭が "SELECT" + ws）
            let mut chars = upper.chars();
            let starts_select = upper.starts_with("SELECT")
                && chars
                    .nth(6)
                    .map(|c| c.is_whitespace() || c == '*')
                    .unwrap_or(false);
            if !starts_select {
                return Self::empty();
            }
        }

        // has_subquery: 全体に `(\s*SELECT\b` が現れるか
        let has_subquery = detect_has_subquery(trimmed);

        // 括弧内を空白に置換したマスク版（文字位置を保存）
        let masked = mask_parenthesized_preserve_len(trimmed);
        let masked_upper = masked.to_ascii_uppercase();

        // has_join: マスク後に JOIN キーワードがあるか / FROM <table> [AS a] の後にカンマ
        let has_join = detect_has_join(&masked, &masked_upper);

        // FROM 句の直後のテーブル抽出（マスク後対象。テーブル名自体は括弧内ではない前提）
        let table = if has_join {
            None
        } else {
            extract_table(&masked, &masked_upper)
        };

        // has_expression: SELECT と FROM の間を見る。FROM 位置はマスク後、
        // カラムリストは元文字列（括弧が残っている）で判定する
        let has_expression = detect_has_expression(trimmed, &masked_upper);

        Self {
            is_select: true,
            table,
            has_join,
            has_subquery,
            has_expression,
        }
    }

    fn empty() -> Self {
        Self {
            is_select: false,
            table: None,
            has_join: false,
            has_subquery: false,
            has_expression: false,
        }
    }
}

/// サブクエリ括弧内だけを空白に置換し、**バイト長**を保つ（JOIN 検出の誤検知回避用）。
/// 括弧自体も空白にする。マルチバイト文字は同じバイト数の ASCII 空白で置換することで、
/// source 側のバイトインデックスがそのまま使えるようにする。
fn mask_parenthesized_preserve_len(s: &str) -> String {
    let mut depth: i32 = 0;
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let n = c.len_utf8();
        match c {
            '(' => {
                depth += 1;
                // 括弧自体は 1 バイト
                out.push(' ');
            }
            ')' => {
                if depth > 0 {
                    depth -= 1;
                }
                out.push(' ');
            }
            _ => {
                if depth == 0 {
                    out.push(c);
                } else {
                    // 括弧内はバイト長を保つために同じバイト数の空白で置換
                    for _ in 0..n {
                        out.push(' ');
                    }
                }
            }
        }
    }
    out
}

fn detect_has_subquery(s: &str) -> bool {
    let upper = s.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            // 括弧の後の空白をスキップして SELECT
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                j += 1;
            }
            if j + 6 <= bytes.len() && &bytes[j..j + 6] == b"SELECT" {
                // SELECT の後ろが境界（空白 or 非識別子）
                let next = if j + 6 < bytes.len() {
                    bytes[j + 6] as char
                } else {
                    ' '
                };
                if !is_identifier_char(next) {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

fn is_identifier_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn detect_has_join(outer: &str, outer_upper: &str) -> bool {
    // `\bJOIN\b` 検出
    if contains_keyword(outer_upper, "JOIN") {
        return true;
    }
    // FROM <table> [AS a] の後にカンマがあるか
    if let Some(from_idx) = find_keyword(outer_upper, "FROM") {
        let after_from = &outer[from_idx + 4..];
        let after_from_upper = &outer_upper[from_idx + 4..];
        // FROM の次のキーワードまでの範囲を取る
        let end = next_clause_start(after_from_upper);
        let from_clause = &after_from[..end];
        // from_clause 内にカンマがあれば複数テーブル結合
        if from_clause.contains(',') {
            return true;
        }
    }
    false
}

fn contains_keyword(upper: &str, kw: &str) -> bool {
    find_keyword(upper, kw).is_some()
}

/// `\bKW\b` の最初の位置を返す（単語境界ベース）
fn find_keyword(upper: &str, kw: &str) -> Option<usize> {
    let bytes = upper.as_bytes();
    let kw_bytes = kw.as_bytes();
    if kw_bytes.is_empty() || bytes.len() < kw_bytes.len() {
        return None;
    }
    let mut i = 0;
    while i + kw_bytes.len() <= bytes.len() {
        if &bytes[i..i + kw_bytes.len()] == kw_bytes {
            let before_ok = if i == 0 {
                true
            } else {
                !is_identifier_char(bytes[i - 1] as char)
            };
            let after_ok = if i + kw_bytes.len() == bytes.len() {
                true
            } else {
                !is_identifier_char(bytes[i + kw_bytes.len()] as char)
            };
            if before_ok && after_ok {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// FROM 句以降における次の節（WHERE/GROUP/ORDER/HAVING/LIMIT/;/end）の開始位置
fn next_clause_start(upper: &str) -> usize {
    let candidates = ["WHERE", "GROUP", "ORDER", "HAVING", "LIMIT"];
    let mut end = upper.len();
    for kw in &candidates {
        if let Some(pos) = find_keyword(upper, kw) {
            if pos < end {
                end = pos;
            }
        }
    }
    // セミコロン
    if let Some(pos) = upper.find(';') {
        if pos < end {
            end = pos;
        }
    }
    end
}

/// 識別子を囲う引用符（バッククォート・ダブルクォート・シングルクォート）を剥がす
fn strip_quotes(s: &str) -> &str {
    s.trim_matches('`').trim_matches('"').trim_matches('\'')
}

fn extract_table(outer: &str, outer_upper: &str) -> Option<String> {
    let from_idx = find_keyword(outer_upper, "FROM")?;
    let after = &outer[from_idx + 4..];
    let after_upper = &outer_upper[from_idx + 4..];
    let end = next_clause_start(after_upper);
    let from_clause = after[..end].trim();
    if from_clause.is_empty() {
        return None;
    }
    // カンマがあれば複数テーブル
    if from_clause.contains(',') {
        return None;
    }
    // 最初のトークンを取得（空白区切り）
    let token_raw = from_clause.split_whitespace().next()?;
    let token = strip_quotes(token_raw);
    if token.is_empty() {
        return None;
    }
    // スキーマ修飾: ドット区切りなら最後を table として採用
    let table_part = match token.rfind('.') {
        Some(pos) => &token[pos + 1..],
        None => token,
    };
    let table_part = strip_quotes(table_part);
    if table_part.is_empty() {
        return None;
    }
    Some(table_part.to_ascii_lowercase())
}

/// カラムリスト判定。`source` は元クエリ（括弧が残っている）、
/// `masked_upper` は括弧を空白に置換した大文字化クエリ（FROM 位置特定に利用）。
fn detect_has_expression(source: &str, masked_upper: &str) -> bool {
    // SELECT と FROM の間をマスク後の位置から特定
    let select_pos = match find_keyword(masked_upper, "SELECT") {
        Some(p) => p,
        None => return false,
    };
    let after_start = select_pos + 6;
    if after_start > source.len() {
        return false;
    }
    let after_select = &source[after_start..];
    let after_masked_upper = &masked_upper[after_start..];
    let from_rel = match find_keyword(after_masked_upper, "FROM") {
        Some(p) => p,
        None => after_select.len(),
    };
    let cols_part = after_select[..from_rel].trim();
    if cols_part.is_empty() {
        return false;
    }
    if cols_part == "*" {
        return false;
    }
    // 関数呼び出し、四則演算子、CASE、CAST、AS エイリアス
    if cols_part.contains('(') || cols_part.contains(')') {
        return true;
    }
    for op in ['+', '-', '*', '/', '%'] {
        // '*' は単独 select * を弾いた後なので、カラムリストで '*' が現れる場合は
        // `a.*` / `a * 2` のような expression とみなす
        if cols_part.contains(op) {
            // ただし `a.*` 形式は許容したい場合もあるが MVP では expression 扱いにする
            return true;
        }
    }
    let upper = cols_part.to_ascii_uppercase();
    if contains_keyword(&upper, "CASE")
        || contains_keyword(&upper, "CAST")
        || contains_keyword(&upper, "AS")
    {
        return true;
    }
    // カンマ区切りの各トークンが `[<schema>.]<col>` 形式か確認
    for tok in cols_part.split(',') {
        let t = tok.trim().trim_matches('`').trim_matches('"');
        if t.is_empty() {
            return true;
        }
        // スキーマ.col を許容
        for part in t.split('.') {
            for c in part.chars() {
                if !(is_identifier_char(c) || c == '`' || c == '"') {
                    return true;
                }
            }
        }
    }
    false
}

/// CcAnalysis + SchemaState から最終的な可否を判定
pub fn compute_eligibility(analysis: &CcAnalysis, schema: &SchemaState) -> CcEligibility {
    if !analysis.is_select {
        return CcEligibility::NotSelect;
    }
    if analysis.has_subquery {
        return CcEligibility::HasSubquery;
    }
    if analysis.has_join {
        return CcEligibility::MultipleTables;
    }
    if analysis.has_expression {
        return CcEligibility::HasExpression;
    }
    let table = match &analysis.table {
        Some(t) => t,
        None => return CcEligibility::NotSelect,
    };
    if !schema.columns_loaded(table) {
        return CcEligibility::ColumnsNotLoaded;
    }
    let pk = match schema.primary_keys_for(table) {
        Some(p) => p,
        None => return CcEligibility::ColumnsNotLoaded,
    };
    if pk.is_empty() {
        return CcEligibility::NoPrimaryKey;
    }
    CcEligibility::Ok {
        table: table.clone(),
        pk_columns: pk,
    }
}

/// UPDATE 文を組み立てる
pub fn build_update_statement(
    table: &str,
    columns: &[String],
    row: &[String],
    pk_columns: &[String],
) -> String {
    let set_parts: Vec<String> = columns
        .iter()
        .zip(row.iter())
        .map(|(c, v)| format!("{}='{}'", c, escape_single_quote(v)))
        .collect();
    let where_parts: Vec<String> = pk_columns
        .iter()
        .map(|pk| {
            let idx = columns.iter().position(|c| c == pk);
            let v = idx.and_then(|i| row.get(i)).map(|s| s.as_str()).unwrap_or("");
            format!("{}='{}'", pk, escape_single_quote(v))
        })
        .collect();
    format!(
        "UPDATE {} SET {} WHERE {};",
        table,
        set_parts.join(", "),
        where_parts.join(" AND ")
    )
}

fn escape_single_quote(s: &str) -> String {
    s.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::schema::{ColumnEntry, SchemaState, TableEntry};

    // ── ヘルパー ──

    fn make_schema_with_table(
        table_name: &str,
        columns: Vec<(&str, &str, bool)>,
        loaded: bool,
    ) -> SchemaState {
        let mut schema = SchemaState::new();
        let col_entries: Vec<ColumnEntry> = columns
            .into_iter()
            .map(|(n, t, pk)| ColumnEntry {
                name: n.to_string(),
                col_type: t.to_string(),
                is_primary_key: pk,
            })
            .collect();
        schema.tables.push(TableEntry {
            name: table_name.to_string(),
            expanded: false,
            columns: col_entries,
            columns_loaded: loaded,
            columns_loading: false,
        });
        schema
    }

    // ── CcAnalysis::from_query ──

    /// 単純な SELECT * FROM users は単一テーブル SELECT として認識される
    #[test]
    fn from_query_select_star() {
        let a = CcAnalysis::from_query("SELECT * FROM users");
        assert!(a.is_select);
        assert_eq!(a.table.as_deref(), Some("users"));
        assert!(!a.has_join);
        assert!(!a.has_subquery);
        assert!(!a.has_expression);
    }

    /// SELECT id, name FROM users は複数カラムでも式なしの単一テーブル SELECT
    #[test]
    fn from_query_multiple_columns_no_expression() {
        let a = CcAnalysis::from_query("SELECT id, name FROM users");
        assert!(a.is_select);
        assert_eq!(a.table.as_deref(), Some("users"));
        assert!(!a.has_join);
        assert!(!a.has_subquery);
        assert!(!a.has_expression);
    }

    /// 小文字 select・大文字テーブル名・前後空白に対して、テーブル名が小文字化される
    #[test]
    fn from_query_lowercase_and_whitespace() {
        let a = CcAnalysis::from_query("  select * from USERS  ");
        assert!(a.is_select);
        assert_eq!(a.table.as_deref(), Some("users"));
        assert!(!a.has_join);
        assert!(!a.has_subquery);
        assert!(!a.has_expression);
    }

    /// スキーマ修飾 public.users はテーブル部分のみが採用される
    #[test]
    fn from_query_schema_qualified() {
        let a = CcAnalysis::from_query("SELECT id FROM public.users");
        assert!(a.is_select);
        assert_eq!(a.table.as_deref(), Some("users"));
        assert!(!a.has_join);
        assert!(!a.has_subquery);
        assert!(!a.has_expression);
    }

    /// FROM users AS u の AS エイリアスは除去される
    #[test]
    fn from_query_table_alias_with_as() {
        let a = CcAnalysis::from_query("SELECT * FROM users AS u");
        assert!(a.is_select);
        assert_eq!(a.table.as_deref(), Some("users"));
        assert!(!a.has_join);
    }

    /// FROM users u の空白区切りエイリアスも除去される
    #[test]
    fn from_query_table_alias_without_as() {
        let a = CcAnalysis::from_query("SELECT * FROM users u");
        assert!(a.is_select);
        assert_eq!(a.table.as_deref(), Some("users"));
        assert!(!a.has_join);
    }

    /// JOIN を含むクエリは has_join=true、table=None
    #[test]
    fn from_query_inner_join() {
        let a = CcAnalysis::from_query("SELECT * FROM users JOIN orders ON u.id=o.uid");
        assert!(a.is_select);
        assert_eq!(a.table, None);
        assert!(a.has_join);
        assert!(!a.has_subquery);
    }

    /// LEFT JOIN も has_join=true として検出される
    #[test]
    fn from_query_left_join() {
        let a = CcAnalysis::from_query("SELECT * FROM users LEFT JOIN orders ON u.id=o.uid");
        assert!(a.is_select);
        assert!(a.has_join);
        assert_eq!(a.table, None);
    }

    /// カンマ結合も has_join=true として扱う
    #[test]
    fn from_query_comma_join() {
        let a = CcAnalysis::from_query("SELECT * FROM users, orders");
        assert!(a.is_select);
        assert!(a.has_join);
        assert_eq!(a.table, None);
    }

    /// WHERE 句内のサブクエリは has_subquery=true、括弧内除去後の外側クエリに JOIN は現れない
    /// impl-2 の strip_parenthesized 実装により [I3] 注記ケースがテスト可能と確認済み
    #[test]
    fn from_query_where_in_subquery_has_subquery() {
        let a = CcAnalysis::from_query("SELECT * FROM users WHERE id IN (SELECT id FROM t)");
        assert!(a.is_select);
        assert!(a.has_subquery);
        assert!(!a.has_join);
        assert_eq!(a.table.as_deref(), Some("users"));
    }

    /// COUNT(*) などの関数呼び出しは has_expression=true として検出される
    #[test]
    fn from_query_aggregate_function() {
        let a = CcAnalysis::from_query("SELECT COUNT(*) FROM users");
        assert!(a.is_select);
        assert!(a.has_expression);
        assert_eq!(a.table.as_deref(), Some("users"));
    }

    /// 四則演算式 id + 1 は has_expression=true
    #[test]
    fn from_query_arithmetic_expression() {
        let a = CcAnalysis::from_query("SELECT id + 1 FROM users");
        assert!(a.is_select);
        assert!(a.has_expression);
        assert_eq!(a.table.as_deref(), Some("users"));
    }

    /// UPPER(name) のようなカラムへの関数適用は has_expression=true
    #[test]
    fn from_query_function_on_column() {
        let a = CcAnalysis::from_query("SELECT id, UPPER(name) FROM users");
        assert!(a.is_select);
        assert!(a.has_expression);
        assert_eq!(a.table.as_deref(), Some("users"));
    }

    /// UPDATE 文は is_select=false・table=None
    #[test]
    fn from_query_update_is_not_select() {
        let a = CcAnalysis::from_query("UPDATE users SET x=1");
        assert!(!a.is_select);
        assert_eq!(a.table, None);
        assert!(!a.has_join);
        assert!(!a.has_subquery);
        assert!(!a.has_expression);
    }

    /// 空クエリは is_select=false・table=None
    #[test]
    fn from_query_empty() {
        let a = CcAnalysis::from_query("");
        assert!(!a.is_select);
        assert_eq!(a.table, None);
        assert!(!a.has_join);
        assert!(!a.has_subquery);
        assert!(!a.has_expression);
    }

    // ── build_update_statement ──

    /// 通常ケース: 2 カラム、PK 1 つで UPDATE 文が組まれる
    #[test]
    fn build_update_basic() {
        let sql = build_update_statement(
            "users",
            &["id".to_string(), "name".to_string()],
            &["1".to_string(), "alice".to_string()],
            &["id".to_string()],
        );
        assert_eq!(sql, "UPDATE users SET id='1', name='alice' WHERE id='1';");
    }

    /// 複合 PK: WHERE 句が AND で連結される
    #[test]
    fn build_update_composite_pk() {
        let sql = build_update_statement(
            "t",
            &["a".to_string(), "b".to_string()],
            &["x".to_string(), "y".to_string()],
            &["a".to_string(), "b".to_string()],
        );
        assert_eq!(sql, "UPDATE t SET a='x', b='y' WHERE a='x' AND b='y';");
    }

    /// シングルクォートを含む値は '' にエスケープされる
    #[test]
    fn build_update_escapes_single_quote() {
        let sql = build_update_statement(
            "t",
            &["n".to_string()],
            &["O'Brien".to_string()],
            &["n".to_string()],
        );
        assert_eq!(sql, "UPDATE t SET n='O''Brien' WHERE n='O''Brien';");
    }

    /// 空文字の値は '' として出力される
    #[test]
    fn build_update_empty_value() {
        let sql = build_update_statement(
            "t",
            &["n".to_string()],
            &["".to_string()],
            &["n".to_string()],
        );
        assert_eq!(sql, "UPDATE t SET n='' WHERE n='';");
    }

    // ── compute_eligibility の各バリアント網羅 ──

    /// 単一テーブル SELECT + スキーマ読込済 + PK あり → Ok
    #[test]
    fn eligibility_ok_single_table_with_pk() {
        let schema = make_schema_with_table(
            "users",
            vec![("id", "int", true), ("name", "varchar", false)],
            true,
        );
        let analysis = CcAnalysis::from_query("SELECT * FROM users");
        let result = compute_eligibility(&analysis, &schema);
        assert!(matches!(result, CcEligibility::Ok { .. }));
        if let CcEligibility::Ok { table, pk_columns } = result {
            assert_eq!(table, "users");
            assert_eq!(pk_columns, vec!["id".to_string()]);
        }
    }

    /// SELECT 以外のクエリ（UPDATE 等）→ NotSelect
    #[test]
    fn eligibility_not_select_for_update_query() {
        let schema = SchemaState::new();
        let analysis = CcAnalysis::from_query("UPDATE users SET x=1");
        let result = compute_eligibility(&analysis, &schema);
        assert_eq!(result, CcEligibility::NotSelect);
    }

    /// 空クエリ → NotSelect
    #[test]
    fn eligibility_not_select_for_empty_query() {
        let schema = SchemaState::new();
        let analysis = CcAnalysis::from_query("");
        let result = compute_eligibility(&analysis, &schema);
        assert_eq!(result, CcEligibility::NotSelect);
    }

    /// JOIN を含む SELECT → MultipleTables
    #[test]
    fn eligibility_multiple_tables_for_join() {
        let schema = SchemaState::new();
        let analysis = CcAnalysis::from_query("SELECT * FROM users JOIN orders ON u.id=o.uid");
        let result = compute_eligibility(&analysis, &schema);
        assert_eq!(result, CcEligibility::MultipleTables);
    }

    /// サブクエリを含む SELECT → HasSubquery
    #[test]
    fn eligibility_has_subquery() {
        let schema = SchemaState::new();
        let analysis = CcAnalysis::from_query("SELECT * FROM (SELECT id FROM t) sub");
        let result = compute_eligibility(&analysis, &schema);
        assert_eq!(result, CcEligibility::HasSubquery);
    }

    /// 関数呼び出し等の式カラムを含む SELECT → HasExpression
    #[test]
    fn eligibility_has_expression() {
        let schema = make_schema_with_table("users", vec![("id", "int", true)], true);
        let analysis = CcAnalysis::from_query("SELECT COUNT(*) FROM users");
        let result = compute_eligibility(&analysis, &schema);
        assert_eq!(result, CcEligibility::HasExpression);
    }

    /// スキーマが未読込（columns_loaded=false）→ ColumnsNotLoaded
    #[test]
    fn eligibility_columns_not_loaded() {
        let schema = make_schema_with_table("users", vec![("id", "int", true)], false);
        let analysis = CcAnalysis::from_query("SELECT * FROM users");
        let result = compute_eligibility(&analysis, &schema);
        assert_eq!(result, CcEligibility::ColumnsNotLoaded);
    }

    /// スキーマに存在しないテーブル → ColumnsNotLoaded
    #[test]
    fn eligibility_columns_not_loaded_for_unknown_table() {
        let schema = SchemaState::new();
        let analysis = CcAnalysis::from_query("SELECT * FROM users");
        let result = compute_eligibility(&analysis, &schema);
        assert_eq!(result, CcEligibility::ColumnsNotLoaded);
    }

    /// スキーマ読込済だが PK カラムがない → NoPrimaryKey
    #[test]
    fn eligibility_no_primary_key() {
        let schema = make_schema_with_table(
            "users",
            vec![("id", "int", false), ("name", "varchar", false)],
            true,
        );
        let analysis = CcAnalysis::from_query("SELECT * FROM users");
        let result = compute_eligibility(&analysis, &schema);
        assert_eq!(result, CcEligibility::NoPrimaryKey);
    }

    // ── CcEligibility の補助メソッド ──

    /// label() は各バリアントに対応する固定文字列を返す
    #[test]
    fn eligibility_labels() {
        assert_eq!(
            CcEligibility::Ok {
                table: "t".to_string(),
                pk_columns: vec!["id".to_string()],
            }
            .label(),
            "[cc OK]"
        );
        assert_eq!(CcEligibility::NotSelect.label(), "[cc:SELECT限定]");
        assert_eq!(CcEligibility::MultipleTables.label(), "[cc:JOIN不可]");
        assert_eq!(CcEligibility::HasSubquery.label(), "[cc:サブクエリ不可]");
        assert_eq!(CcEligibility::HasExpression.label(), "[cc:式カラム不可]");
        assert_eq!(CcEligibility::NoPrimaryKey.label(), "[cc:PKなし]");
        assert_eq!(
            CcEligibility::ColumnsNotLoaded.label(),
            "[cc:スキーマ未読込]"
        );
    }

    /// label_color() は契約書 §3.5 の対応表どおりの色を返す
    #[test]
    fn eligibility_label_colors() {
        assert_eq!(
            CcEligibility::Ok {
                table: "t".to_string(),
                pk_columns: vec!["id".to_string()],
            }
            .label_color(),
            Color::Green
        );
        assert_eq!(CcEligibility::NotSelect.label_color(), Color::DarkGray);
        assert_eq!(
            CcEligibility::ColumnsNotLoaded.label_color(),
            Color::DarkGray
        );
        assert_eq!(CcEligibility::MultipleTables.label_color(), Color::Yellow);
        assert_eq!(CcEligibility::HasSubquery.label_color(), Color::Yellow);
        assert_eq!(CcEligibility::HasExpression.label_color(), Color::Yellow);
        assert_eq!(CcEligibility::NoPrimaryKey.label_color(), Color::Red);
    }

    /// is_ok() は Ok バリアントでのみ true を返す
    #[test]
    fn eligibility_is_ok_only_for_ok() {
        assert!(CcEligibility::Ok {
            table: "t".to_string(),
            pk_columns: vec!["id".to_string()],
        }
        .is_ok());
        assert!(!CcEligibility::NotSelect.is_ok());
        assert!(!CcEligibility::MultipleTables.is_ok());
        assert!(!CcEligibility::HasSubquery.is_ok());
        assert!(!CcEligibility::HasExpression.is_ok());
        assert!(!CcEligibility::NoPrimaryKey.is_ok());
        assert!(!CcEligibility::ColumnsNotLoaded.is_ok());
    }

    /// status_reason() は契約書どおりの日本語メッセージを返す
    #[test]
    fn eligibility_status_reasons() {
        assert_eq!(
            CcEligibility::NotSelect.status_reason(),
            "cc は単一テーブルの SELECT 結果でのみ使用できます"
        );
        assert_eq!(
            CcEligibility::MultipleTables.status_reason(),
            "cc は JOIN を含むクエリでは使用できません"
        );
        assert_eq!(
            CcEligibility::HasSubquery.status_reason(),
            "cc はサブクエリを含むクエリでは使用できません"
        );
        assert_eq!(
            CcEligibility::HasExpression.status_reason(),
            "cc は式カラムを含む SELECT では使用できません"
        );
        assert_eq!(
            CcEligibility::NoPrimaryKey.status_reason(),
            "対象テーブルに PK がないため cc は使用できません"
        );
        assert_eq!(
            CcEligibility::ColumnsNotLoaded.status_reason(),
            "Schema パネルでテーブルを展開してから再度実行してください"
        );
    }

    // ── マルチバイト文字境界（reviewer-c [M1] 再発防止、[M2] ケース） ──

    /// 括弧内にマルチバイト文字を含むカラム式でも panic せず、has_expression=true として解析される
    #[test]
    fn from_query_multibyte_in_expression_parens() {
        let a = CcAnalysis::from_query("SELECT (ああ) FROM t");
        assert!(a.is_select);
        assert!(a.has_expression);
        assert_eq!(a.table.as_deref(), Some("t"));
        assert!(!a.has_join);
        assert!(!a.has_subquery);
    }

    /// WHERE 句のリテラルにマルチバイト文字を含んでも panic せず、table 抽出される
    #[test]
    fn from_query_multibyte_in_where_literal() {
        let a = CcAnalysis::from_query("SELECT * FROM users WHERE name = '日本語'");
        assert!(a.is_select);
        assert!(!a.has_expression);
        assert_eq!(a.table.as_deref(), Some("users"));
        assert!(!a.has_subquery);
        assert!(!a.has_join);
    }

    /// サブクエリ内にマルチバイトリテラルがあっても panic せず、has_subquery=true・has_join=false
    #[test]
    fn from_query_multibyte_in_subquery_literal() {
        let a = CcAnalysis::from_query(
            "SELECT * FROM users WHERE x IN (SELECT id FROM t WHERE name = 'あい')",
        );
        assert!(a.is_select);
        assert!(a.has_subquery);
        assert!(!a.has_join);
        assert_eq!(a.table.as_deref(), Some("users"));
    }
}
