use super::*;

// ── LimitApplier ──

#[test]
fn limit_applier_adds_limit_to_select() {
    // Arrange
    let applier = LimitApplier { default_limit: 100 };

    // Act
    let (query, applied) = applier.apply("SELECT * FROM users");

    // Assert
    assert_eq!(query, "SELECT * FROM users LIMIT 100");
    assert!(applied);
}

#[test]
fn limit_applier_adds_limit_to_with_query() {
    let applier = LimitApplier { default_limit: 50 };
    let (query, applied) = applier.apply("WITH cte AS (SELECT 1) SELECT * FROM cte");
    assert_eq!(
        query,
        "WITH cte AS (SELECT 1) SELECT * FROM cte LIMIT 50"
    );
    assert!(applied);
}

#[test]
fn limit_applier_removes_trailing_semicolon() {
    let applier = LimitApplier { default_limit: 100 };
    let (query, applied) = applier.apply("SELECT 1;");
    assert_eq!(query, "SELECT 1 LIMIT 100");
    assert!(applied);
}

#[test]
fn limit_applier_skips_when_limit_exists() {
    let applier = LimitApplier { default_limit: 100 };
    let (query, applied) = applier.apply("SELECT * FROM users LIMIT 10");
    assert_eq!(query, "SELECT * FROM users LIMIT 10");
    assert!(!applied);
}

#[test]
fn limit_applier_skips_when_fetch_first_exists() {
    let applier = LimitApplier { default_limit: 100 };
    let (query, applied) = applier.apply("SELECT * FROM users FETCH FIRST 10 ROWS ONLY");
    assert_eq!(query, "SELECT * FROM users FETCH FIRST 10 ROWS ONLY");
    assert!(!applied);
}

#[test]
fn limit_applier_skips_non_select() {
    let applier = LimitApplier { default_limit: 100 };
    let (query, applied) = applier.apply("INSERT INTO users VALUES (1)");
    assert_eq!(query, "INSERT INTO users VALUES (1)");
    assert!(!applied);
}

#[test]
fn limit_applier_disabled_when_zero() {
    let applier = LimitApplier { default_limit: 0 };
    let (query, applied) = applier.apply("SELECT * FROM users");
    assert_eq!(query, "SELECT * FROM users");
    assert!(!applied);
}

#[test]
fn limit_applier_handles_lowercase_select() {
    let applier = LimitApplier { default_limit: 100 };
    let (query, applied) = applier.apply("select * from users");
    assert_eq!(query, "select * from users LIMIT 100");
    assert!(applied);
}

// ── ReadonlyChecker ──

#[test]
fn readonly_checker_allows_select() {
    assert!(ReadonlyChecker.check("SELECT * FROM users").is_ok());
}

#[test]
fn readonly_checker_blocks_insert() {
    assert!(ReadonlyChecker.check("INSERT INTO users VALUES (1)").is_err());
}

#[test]
fn readonly_checker_blocks_update() {
    assert!(ReadonlyChecker.check("UPDATE users SET name = 'x'").is_err());
}

#[test]
fn readonly_checker_blocks_delete() {
    assert!(ReadonlyChecker.check("DELETE FROM users").is_err());
}

#[test]
fn readonly_checker_blocks_drop() {
    assert!(ReadonlyChecker.check("DROP TABLE users").is_err());
}

#[test]
fn readonly_checker_blocks_truncate() {
    assert!(ReadonlyChecker.check("TRUNCATE users").is_err());
}

#[test]
fn readonly_checker_allows_lowercase_select() {
    assert!(ReadonlyChecker.check("select * from users").is_ok());
}

#[test]
fn readonly_checker_blocks_lowercase_insert() {
    assert!(ReadonlyChecker.check("insert into users values (1)").is_err());
}

#[test]
fn readonly_checker_allows_empty_string() {
    assert!(ReadonlyChecker.check("").is_ok());
}
