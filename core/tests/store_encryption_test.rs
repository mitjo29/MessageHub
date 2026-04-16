use rusqlite::Connection;
use tempfile::NamedTempFile;

#[test]
fn test_sqlcipher_encryption_roundtrip() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap();
    let password = "test-master-password";

    // Create encrypted database
    {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "key", password).unwrap();
        conn.execute_batch(
            "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT);
             INSERT INTO test VALUES (1, 'secret data');"
        ).unwrap();
    }

    // Read with correct password
    {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "key", password).unwrap();
        let value: String = conn
            .query_row("SELECT value FROM test WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(value, "secret data");
    }

    // Fail with wrong password
    {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "key", "wrong-password").unwrap();
        let result = conn.query_row("SELECT value FROM test WHERE id = 1", [], |r| r.get::<_, String>(0));
        assert!(result.is_err(), "should fail with wrong password");
    }
}

#[test]
fn test_wal_mode_enabled() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap();

    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "key", "test-password").unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();

    let mode: String = conn
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}
