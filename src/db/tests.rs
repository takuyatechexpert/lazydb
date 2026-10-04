use super::*;

const PG: &DbType = &DbType::Postgresql;
const MYSQL: &DbType = &DbType::Mysql;
const SQLITE: &DbType = &DbType::Sqlite;

// ── LimitApplier ──

#[test]
fn limit_applier_adds_limit_to_select() {
    // Arrange
    let applier = LimitApplier { default_limit: 100 };

    // Act
    let (query, applied) = applier.apply("SELECT * FROM users", PG);

    // Assert
    assert_eq!(query, "SELECT * FROM users LIMIT 100");
    assert!(applied);
}

#[test]
fn limit_applier_adds_limit_to_with_query() {
    let applier = LimitApplier { default_limit: 50 };
    let (query, applied) = applier.apply("WITH cte AS (SELECT 1) SELECT * FROM cte", PG);
    assert_eq!(
        query,
        "WITH cte AS (SELECT 1) SELECT * FROM cte LIMIT 50"
    );
    assert!(applied);
}

#[test]
fn limit_applier_removes_trailing_semicolon() {
    let applier = LimitApplier { default_limit: 100 };
    let (query, applied) = applier.apply("SELECT 1;", PG);
    assert_eq!(query, "SELECT 1 LIMIT 100");
    assert!(applied);
}

#[test]
fn limit_applier_skips_when_limit_exists() {
    let applier = LimitApplier { default_limit: 100 };
    let (query, applied) = applier.apply("SELECT * FROM users LIMIT 10", PG);
    assert_eq!(query, "SELECT * FROM users LIMIT 10");
    assert!(!applied);
}

#[test]
fn limit_applier_skips_when_fetch_first_exists() {
    let applier = LimitApplier { default_limit: 100 };
    let (query, applied) = applier.apply("SELECT * FROM users FETCH FIRST 10 ROWS ONLY", PG);
    assert_eq!(query, "SELECT * FROM users FETCH FIRST 10 ROWS ONLY");
    assert!(!applied);
}

#[test]
fn limit_applier_skips_non_select() {
    let applier = LimitApplier { default_limit: 100 };
    let (query, applied) = applier.apply("INSERT INTO users VALUES (1)", PG);
    assert_eq!(query, "INSERT INTO users VALUES (1)");
    assert!(!applied);
}

#[test]
fn limit_applier_disabled_when_zero() {
    let applier = LimitApplier { default_limit: 0 };
    let (query, applied) = applier.apply("SELECT * FROM users", PG);
    assert_eq!(query, "SELECT * FROM users");
    assert!(!applied);
}

#[test]
fn limit_applier_skips_when_limit_exists_in_formatted_query() {
    // sqlformat で整形すると LIMIT が改行で囲まれるため、
    // " LIMIT " / "\nLIMIT " のいずれにも一致せず誤って二重付与される回帰防止
    let applier = LimitApplier { default_limit: 100 };
    let formatted = "SELECT\n  *\nFROM\n  users\nLIMIT\n  10";
    let (query, applied) = applier.apply(formatted, PG);
    assert_eq!(query, formatted);
    assert!(!applied);
}

#[test]
fn limit_applier_skips_when_fetch_first_exists_in_formatted_query() {
    let applier = LimitApplier { default_limit: 100 };
    let formatted = "SELECT\n  *\nFROM\n  users\nFETCH FIRST 10 ROWS ONLY";
    let (query, applied) = applier.apply(formatted, PG);
    assert_eq!(query, formatted);
    assert!(!applied);
}

#[test]
fn limit_applier_handles_lowercase_select() {
    let applier = LimitApplier { default_limit: 100 };
    let (query, applied) = applier.apply("select * from users", PG);
    assert_eq!(query, "select * from users LIMIT 100");
    assert!(applied);
}

#[test]
fn limit_applier_skips_dml_after_with() {
    // WITH ... DELETE に LIMIT を付けると、PostgreSQL では構文エラー、MySQL では削除件数の制限になる
    let applier = LimitApplier { default_limit: 100 };
    let query = "WITH x AS (SELECT 1) DELETE FROM users";
    assert_eq!(applier.apply(query, MYSQL), (query.to_string(), false));
}

#[test]
fn limit_applier_adds_limit_when_column_is_named_top() {
    // 対応している DB に TOP 構文はないので、top という列名で付与をやめない
    let applier = LimitApplier { default_limit: 100 };
    let (query, applied) = applier.apply("SELECT top FROM ranking", PG);
    assert_eq!(query, "SELECT top FROM ranking LIMIT 100");
    assert!(applied);
}

// ── ReadonlyChecker ──

#[test]
fn readonly_checker_allows_select() {
    assert!(ReadonlyChecker.check("SELECT * FROM users", PG).is_ok());
}

#[test]
fn readonly_checker_blocks_insert() {
    assert!(ReadonlyChecker.check("INSERT INTO users VALUES (1)", PG).is_err());
}

#[test]
fn readonly_checker_blocks_update() {
    assert!(ReadonlyChecker.check("UPDATE users SET name = 'x'", PG).is_err());
}

#[test]
fn readonly_checker_blocks_delete() {
    assert!(ReadonlyChecker.check("DELETE FROM users", PG).is_err());
}

#[test]
fn readonly_checker_blocks_drop() {
    assert!(ReadonlyChecker.check("DROP TABLE users", PG).is_err());
}

#[test]
fn readonly_checker_blocks_truncate() {
    assert!(ReadonlyChecker.check("TRUNCATE users", PG).is_err());
}

#[test]
fn readonly_checker_allows_lowercase_select() {
    assert!(ReadonlyChecker.check("select * from users", PG).is_ok());
}

#[test]
fn readonly_checker_blocks_lowercase_insert() {
    assert!(ReadonlyChecker.check("insert into users values (1)", PG).is_err());
}

#[test]
fn readonly_checker_allows_empty_string() {
    assert!(ReadonlyChecker.check("", PG).is_ok());
}

#[test]
fn readonly_checker_blocks_dml_after_leading_comment() {
    for query in ["/* memo */ DELETE FROM users", "-- memo\nDELETE FROM users"] {
        assert!(ReadonlyChecker.check(query, PG).is_err(), "{query}");
    }
    assert!(ReadonlyChecker.check("-- memo\nSELECT * FROM users", PG).is_ok());
}

#[test]
fn readonly_checker_blocks_dml_in_with() {
    // 本体の DML と、CTE の中の DML（PostgreSQL は DELETE ... RETURNING を書ける）
    for query in [
        "WITH x AS (SELECT 1) DELETE FROM users",
        "WITH d AS (DELETE FROM users RETURNING *) SELECT * FROM d",
    ] {
        assert!(ReadonlyChecker.check(query, PG).is_err(), "{query}");
    }
    let recursive =
        "WITH RECURSIVE x(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM x) SELECT * FROM x";
    assert!(ReadonlyChecker.check(recursive, PG).is_ok());
}

#[test]
fn readonly_checker_allows_only_read_statements() {
    for query in [
        "GRANT SELECT ON users TO bob",
        "SET default_transaction_read_only = off",
        "CALL purge_users()",
        "COPY users FROM '/tmp/users.csv'",
        "VACUUM users",
    ] {
        assert!(ReadonlyChecker.check(query, PG).is_err(), "{query}");
    }
    for query in ["SHOW search_path", "VALUES (1)", "TABLE users", "DESCRIBE users"] {
        assert!(ReadonlyChecker.check(query, PG).is_ok(), "{query}");
    }
}

#[test]
fn readonly_checker_blocks_explain_analyze_of_write() {
    // ANALYZE 付きの EXPLAIN は対象の文を実際に実行する
    for query in [
        "EXPLAIN ANALYZE DELETE FROM users",
        "EXPLAIN (ANALYZE, BUFFERS) DELETE FROM users",
    ] {
        assert!(ReadonlyChecker.check(query, PG).is_err(), "{query}");
    }
    for query in ["EXPLAIN ANALYZE SELECT * FROM users", "EXPLAIN DELETE FROM users"] {
        assert!(ReadonlyChecker.check(query, PG).is_ok(), "{query}");
    }
}

#[test]
fn readonly_checker_blocks_select_into() {
    // PostgreSQL は新しいテーブルを作り、MySQL はファイルに書き出す
    assert!(ReadonlyChecker.check("SELECT * INTO users_copy FROM users", PG).is_err());
    assert!(ReadonlyChecker
        .check("SELECT * FROM users INTO OUTFILE '/tmp/users.txt'", MYSQL)
        .is_err());
}

#[test]
fn readonly_checker_reads_strings_and_comments_per_dialect() {
    // MySQL は文字列の中のバックスラッシュがエスケープなので、'...' 全体が 1 つの文字列になる。
    // PostgreSQL は 'a\' で文字列が終わり、続く DELETE が別の文になる
    let query = "SELECT 'a\\'; DELETE FROM users; -- '";
    assert!(ReadonlyChecker.check(query, MYSQL).is_ok());
    assert!(ReadonlyChecker.check(query, PG).is_err());
    // MySQL の実行コメントの中身は実行される
    assert!(ReadonlyChecker.check("/*!50000 DELETE FROM users */", MYSQL).is_err());
}

#[test]
fn readonly_checker_allows_pragma_on_sqlite() {
    // SQLite は接続を読み取り専用で開くので、PRAGMA での書き込みは DB 側が拒否する
    assert!(ReadonlyChecker.check("PRAGMA table_info(users)", SQLITE).is_ok());
}

// ── RedisReadonlyChecker ──

#[test]
fn redis_readonly_allows_get() {
    assert!(RedisReadonlyChecker.check("GET foo").is_ok());
}

#[test]
fn redis_readonly_allows_keys() {
    assert!(RedisReadonlyChecker.check("KEYS user:*").is_ok());
}

#[test]
fn redis_readonly_allows_hgetall() {
    assert!(RedisReadonlyChecker.check("HGETALL myhash").is_ok());
}

#[test]
fn redis_readonly_allows_ping() {
    assert!(RedisReadonlyChecker.check("PING").is_ok());
}

#[test]
fn redis_readonly_allows_select_db() {
    // SELECT は Redis では DB 切替コマンドで書き込みではない
    assert!(RedisReadonlyChecker.check("SELECT 0").is_ok());
}

#[test]
fn redis_readonly_blocks_set() {
    assert!(RedisReadonlyChecker.check("SET key val").is_err());
}

#[test]
fn redis_readonly_blocks_del() {
    assert!(RedisReadonlyChecker.check("DEL foo bar").is_err());
}

#[test]
fn redis_readonly_blocks_flushdb() {
    assert!(RedisReadonlyChecker.check("FLUSHDB").is_err());
}

#[test]
fn redis_readonly_blocks_flushall() {
    assert!(RedisReadonlyChecker.check("FLUSHALL").is_err());
}

#[test]
fn redis_readonly_blocks_hset() {
    assert!(RedisReadonlyChecker.check("HSET myhash field value").is_err());
}

#[test]
fn redis_readonly_blocks_lowercase_del() {
    assert!(RedisReadonlyChecker.check("del foo").is_err());
}

#[test]
fn redis_readonly_handles_trailing_semicolon() {
    // セミコロン付きの SET も書き込みとして弾く
    assert!(RedisReadonlyChecker.check("SET; key val").is_err());
}

#[test]
fn redis_readonly_allows_empty_string() {
    assert!(RedisReadonlyChecker.check("").is_ok());
}

#[test]
fn redis_readonly_blocks_commands_outside_allow_list() {
    // 拒否リストに漏れていた書き込みと、クォートしたコマンド名（実行時はクォートが外れて DEL になる）
    for query in [
        "GETDEL foo",
        "SWAPDB 0 1",
        "FCALL myfunc 0",
        "XREADGROUP GROUP g c STREAMS s >",
        "\"DEL\" foo",
    ] {
        assert!(RedisReadonlyChecker.check(query).is_err(), "{query}");
    }
}

#[test]
fn redis_readonly_blocks_sort_with_store() {
    assert!(RedisReadonlyChecker.check("SORT mylist STORE dest").is_err());
    assert!(RedisReadonlyChecker.check("SORT mylist ALPHA").is_ok());
}
