//! SQL 文の種類の判定
//!
//! readonly 接続のガード（`ReadonlyChecker`）と自動 LIMIT（`LimitApplier`）が同じ判定を使う。
//! 文字列とコメントの区切り方は DB ごとに違うので、方言に合わせて字句に分けてから判定する。
//! 判定できない形は読み取りではないものとして扱う（書き込みを見落とすより、読み取りを拒否する側に倒す）。

use crate::config::connections::DbType;

/// 読み取りの文の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadKind {
    /// SELECT（WITH の本体が SELECT のものを含む）。自動 LIMIT の対象
    Select,
    /// SELECT 以外の読み取り（SHOW / EXPLAIN / DESCRIBE / VALUES / TABLE / PRAGMA）
    Other,
}

/// クエリを `;` で文に区切り、文ごとに読み取りかを判定する。
///
/// すべての文が読み取りなら、空でない文ごとの種類を返す。
/// 読み取りではない文があれば、その文を表す語（`DELETE` など）を返す。
pub fn classify(query: &str, db_type: &DbType) -> Result<Vec<ReadKind>, String> {
    let dialect = Dialect::of(db_type);
    tokenize(query, dialect)
        .split(|t| *t == Token::Semicolon)
        .filter(|statement| !statement.is_empty())
        .map(|statement| classify_statement(statement, dialect))
        .collect()
}

/// 文字列・コメントの区切り方の違い
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Postgres,
    /// 既定の SQL モード（バックスラッシュがエスケープ、`"` が文字列）を前提にする
    Mysql,
    Sqlite,
}

impl Dialect {
    fn of(db_type: &DbType) -> Self {
        match db_type {
            DbType::Postgresql => Dialect::Postgres,
            DbType::Mysql => Dialect::Mysql,
            // Redis は SQL ではないので判定に来ない
            DbType::Sqlite | DbType::Redis => Dialect::Sqlite,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// クォートしていない語。大文字に揃える
    Word(String),
    /// 文字列リテラルとクォートした識別子
    Quoted,
    Open,
    Close,
    Comma,
    Semicolon,
    /// その他の記号
    Symbol(char),
}

impl Token {
    /// エラーメッセージに出す表記
    fn label(&self) -> String {
        match self {
            Token::Word(w) => w.clone(),
            Token::Quoted => "'...'".to_string(),
            Token::Open => "(".to_string(),
            Token::Close => ")".to_string(),
            Token::Comma => ",".to_string(),
            Token::Semicolon => ";".to_string(),
            Token::Symbol(c) => c.to_string(),
        }
    }
}

// ── 字句解析 ──

/// コメントを除いて字句に分ける
fn tokenize(sql: &str, dialect: Dialect) -> Vec<Token> {
    let chars: Vec<char> = sql.chars().collect();
    let mut tokens = Vec::new();
    // MySQL の実行コメント（/*! ... */）の中にいるか。
    // 中身は SQL として実行されるので字句に含め、閉じの */ だけを読み飛ばす
    let mut in_executable_comment = false;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();

        if dialect == Dialect::Postgres && c == '$' {
            if let Some(end) = dollar_quoted_end(&chars, i) {
                tokens.push(Token::Quoted);
                i = end;
                continue;
            }
        }

        match c {
            _ if c.is_whitespace() => i += 1,
            // MySQL の -- は直後に空白が必要（`1--1` は引き算）
            '-' if next == Some('-')
                && (dialect != Dialect::Mysql
                    || chars
                        .get(i + 2)
                        .is_none_or(|c| c.is_whitespace() || c.is_control())) =>
            {
                i = skip_line(&chars, i);
            }
            '#' if dialect == Dialect::Mysql => i = skip_line(&chars, i),
            '/' if next == Some('*') => {
                if dialect == Dialect::Mysql && chars.get(i + 2) == Some(&'!') {
                    // /*!50000 のようにバージョン番号が続く
                    i += 3;
                    while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
                        i += 1;
                    }
                    in_executable_comment = true;
                } else {
                    i = skip_block_comment(&chars, i, dialect == Dialect::Postgres);
                }
            }
            '*' if next == Some('/') && in_executable_comment => {
                in_executable_comment = false;
                i += 2;
            }
            // MySQL だけ、文字列の中のバックスラッシュがエスケープになる
            '\'' => {
                i = skip_quoted(&chars, i, dialect == Dialect::Mysql);
                tokens.push(Token::Quoted);
            }
            // PostgreSQL / SQLite は識別子、MySQL は文字列
            '"' => {
                i = skip_quoted(&chars, i, dialect == Dialect::Mysql);
                tokens.push(Token::Quoted);
            }
            '`' if dialect != Dialect::Postgres => {
                i = skip_quoted(&chars, i, false);
                tokens.push(Token::Quoted);
            }
            // SQLite は [name] も識別子
            '[' if dialect == Dialect::Sqlite => {
                i = chars[i..]
                    .iter()
                    .position(|&c| c == ']')
                    .map_or(chars.len(), |p| i + p + 1);
                tokens.push(Token::Quoted);
            }
            '(' => {
                tokens.push(Token::Open);
                i += 1;
            }
            ')' => {
                tokens.push(Token::Close);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            ';' => {
                tokens.push(Token::Semicolon);
                i += 1;
            }
            _ if is_word_char(c) => {
                let end = chars[i..]
                    .iter()
                    .position(|&c| !is_word_char(c))
                    .map_or(chars.len(), |p| i + p);
                let word = chars[i..end].iter().collect::<String>().to_uppercase();
                i = end;
                // PostgreSQL の E'...' はバックスラッシュがエスケープになる
                if dialect == Dialect::Postgres && word == "E" && chars.get(i) == Some(&'\'') {
                    i = skip_quoted(&chars, i, true);
                    tokens.push(Token::Quoted);
                } else {
                    tokens.push(Token::Word(word));
                }
            }
            _ => {
                tokens.push(Token::Symbol(c));
                i += 1;
            }
        }
    }
    tokens
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// 行末（改行の位置）を返す
fn skip_line(chars: &[char], start: usize) -> usize {
    chars[start..]
        .iter()
        .position(|&c| c == '\n')
        .map_or(chars.len(), |p| start + p)
}

/// ブロックコメントの終わりの次の位置を返す。PostgreSQL は入れ子を数える
fn skip_block_comment(chars: &[char], start: usize, nested: bool) -> usize {
    let mut depth = 0;
    let mut i = start;
    while i + 1 < chars.len() {
        if chars[i] == '/' && chars[i + 1] == '*' && (nested || depth == 0) {
            depth += 1;
            i += 2;
        } else if chars[i] == '*' && chars[i + 1] == '/' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    chars.len()
}

/// クォートの終わりの次の位置を返す。クォートを 2 つ重ねたものはクォート文字そのものとして読む
fn skip_quoted(chars: &[char], start: usize, backslash_escapes: bool) -> usize {
    let quote = chars[start];
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' if backslash_escapes => i += 2,
            c if c == quote && chars.get(i + 1) == Some(&quote) => i += 2,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    chars.len()
}

/// PostgreSQL のドル引用符（`$$...$$` / `$tag$...$tag$`）なら、終わりの次の位置を返す。
/// `$1` のようなパラメータは None
fn dollar_quoted_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start + 1;
    loop {
        let c = *chars.get(i)?;
        if c == '$' {
            break;
        }
        let tag_char = c == '_' || c.is_alphabetic() || (i > start + 1 && c.is_ascii_digit());
        if !tag_char {
            return None;
        }
        i += 1;
    }
    let tag = &chars[start..=i];
    let end = (i + 1..chars.len())
        .find(|&j| chars[j..].starts_with(tag))
        .map_or(chars.len(), |j| j + tag.len());
    Some(end)
}

// ── 文の判定 ──

/// EXPLAIN / DESCRIBE のオプションに使う語
const EXPLAIN_OPTIONS: &[&str] = &[
    "ANALYZE",
    "VERBOSE",
    "FORMAT",
    "TRADITIONAL",
    "JSON",
    "TREE",
    "EXTENDED",
    "PARTITIONS",
];

fn classify_statement(tokens: &[Token], dialect: Dialect) -> Result<ReadKind, String> {
    // (SELECT ...) UNION (SELECT ...) のような括弧始まりは、括弧の中の最初の語で判定する
    let start = tokens
        .iter()
        .position(|t| *t != Token::Open)
        .unwrap_or(tokens.len());
    let tokens = &tokens[start..];

    let keyword = match tokens.first() {
        Some(Token::Word(w)) => w.as_str(),
        Some(other) => return Err(other.label()),
        None => return Err("(".to_string()),
    };
    match keyword {
        "SELECT" => reject_into(tokens).map(|_| ReadKind::Select),
        "VALUES" | "TABLE" => reject_into(tokens).map(|_| ReadKind::Other),
        "SHOW" => Ok(ReadKind::Other),
        // SQLite は接続を読み取り専用で開いているので、PRAGMA での書き込みは DB 側が拒否する
        "PRAGMA" if dialect == Dialect::Sqlite => Ok(ReadKind::Other),
        "EXPLAIN" | "DESCRIBE" | "DESC" => classify_explain(&tokens[1..], dialect),
        "WITH" => classify_with(&tokens[1..], dialect),
        _ => Err(keyword.to_string()),
    }
}

/// SELECT ... INTO は、PostgreSQL では新しいテーブルを作り、MySQL ではファイルや変数に書き出す
fn reject_into(tokens: &[Token]) -> Result<(), String> {
    if tokens.iter().any(|t| is_word(t, "INTO")) {
        return Err("SELECT ... INTO".to_string());
    }
    Ok(())
}

/// EXPLAIN / DESCRIBE は対象の文を実行しない。
/// ただし ANALYZE 付きは実際に実行するので、対象の文も読み取りであることを求める
fn classify_explain(tokens: &[Token], dialect: Dialect) -> Result<ReadKind, String> {
    if !tokens.iter().any(|t| is_word(t, "ANALYZE")) {
        return Ok(ReadKind::Other);
    }
    let malformed = || "EXPLAIN ANALYZE".to_string();

    let mut i = 0;
    // PostgreSQL の EXPLAIN (ANALYZE, BUFFERS) ...
    if tokens.first() == Some(&Token::Open) {
        i = skip_parens(tokens, 0).ok_or_else(malformed)?;
    }
    // ANALYZE VERBOSE / FORMAT = JSON など
    while let Some(t) = tokens.get(i) {
        match t {
            Token::Word(w) if EXPLAIN_OPTIONS.contains(&w.as_str()) => i += 1,
            Token::Symbol('=') => i += 1,
            _ => break,
        }
    }
    if i >= tokens.len() {
        return Err(malformed());
    }
    classify_statement(&tokens[i..], dialect).map(|_| ReadKind::Other)
}

/// WITH は、CTE の中身と本体がすべて読み取りであることを求める。
/// PostgreSQL は CTE の中に DELETE ... RETURNING を書けるので、本体だけでは判定できない
fn classify_with(tokens: &[Token], dialect: Dialect) -> Result<ReadKind, String> {
    let malformed = || "WITH".to_string();

    let mut i = 0;
    if tokens.get(i).is_some_and(|t| is_word(t, "RECURSIVE")) {
        i += 1;
    }
    loop {
        // CTE 名
        match tokens.get(i) {
            Some(Token::Word(_) | Token::Quoted) => i += 1,
            _ => return Err(malformed()),
        }
        // 列名のリスト
        if tokens.get(i) == Some(&Token::Open) {
            i = skip_parens(tokens, i).ok_or_else(malformed)?;
        }
        if !tokens.get(i).is_some_and(|t| is_word(t, "AS")) {
            return Err(malformed());
        }
        i += 1;
        // AS [NOT] MATERIALIZED
        if tokens.get(i).is_some_and(|t| is_word(t, "NOT")) {
            i += 1;
        }
        if tokens.get(i).is_some_and(|t| is_word(t, "MATERIALIZED")) {
            i += 1;
        }
        if tokens.get(i) != Some(&Token::Open) {
            return Err(malformed());
        }
        let end = skip_parens(tokens, i).ok_or_else(malformed)?;
        let body = &tokens[i + 1..end - 1];
        if body.is_empty() {
            return Err(malformed());
        }
        classify_statement(body, dialect)?;
        i = end;

        if tokens.get(i) == Some(&Token::Comma) {
            i += 1;
        } else {
            break;
        }
    }
    if i >= tokens.len() {
        return Err(malformed());
    }
    classify_statement(&tokens[i..], dialect)
}

/// `open` の位置の開き括弧に対応する閉じ括弧の、次の位置を返す。閉じていなければ None
fn skip_parens(tokens: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, t) in tokens[open..].iter().enumerate() {
        match t {
            Token::Open => depth += 1,
            Token::Close => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_word(token: &Token, word: &str) -> bool {
    matches!(token, Token::Word(w) if w == word)
}
