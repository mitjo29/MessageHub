//! Validates that the sqlite-vec extension loads correctly and the
//! knowledge-engine schema created by migration 002 is usable end-to-end.
//!
//! If the `vec0` virtual table type is missing (i.e. sqlite-vec wasn't
//! registered via `sqlite3_auto_extension` before the migration ran),
//! the `CREATE VIRTUAL TABLE vault_chunk_vecs USING vec0(...)` statement
//! in `002_knowledge.sql` would fail — so `Store::open_in_memory()`
//! completing without error already proves extension loading works.
//!
//! This test adds a concrete round-trip: a 384-dim vector is inserted
//! into a `vec0` virtual table and read back byte-for-byte.

use messagehub_core::store::Store;
use rusqlite::Connection;

/// Pack an `f32` slice into the little-endian byte representation that
/// sqlite-vec's `vec0` virtual table expects for a `FLOAT[N]` column.
fn pack_f32_le(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[test]
fn test_sqlite_vec_loads_and_roundtrips_384_dim_vector() {
    // Step 1: creating a Store runs migration 002, which includes
    //   CREATE VIRTUAL TABLE vault_chunk_vecs USING vec0(embedding FLOAT[384]);
    // That DDL only succeeds if sqlite-vec was registered globally
    // via sqlite3_auto_extension *before* the connection was opened.
    let _store = Store::open_in_memory().expect(
        "Store::open_in_memory must succeed — if this fails, sqlite-vec \
         was not registered before migration 002 tried to create a vec0 table.",
    );

    // Step 2: confirm the vec_version() function is globally available.
    // Once sqlite3_auto_extension is registered, *every* subsequent
    // Connection in this process automatically gets sqlite-vec loaded.
    // A fresh ad-hoc connection here is the cleanest way to exercise the
    // virtual table from an integration test without reaching into Store's
    // private conn().
    let conn = Connection::open_in_memory().expect("ad-hoc connection should open");
    let version: String = conn
        .query_row("SELECT vec_version()", [], |r| r.get(0))
        .expect("vec_version() should be callable after Store registered sqlite-vec");
    assert!(version.starts_with('v'), "unexpected vec_version: {version}");

    // Step 3: round-trip a real 384-dim vector through a vec0 table.
    conn.execute_batch("CREATE VIRTUAL TABLE test_vecs USING vec0(embedding FLOAT[384]);")
        .expect("vec0 virtual table should be creatable");

    let vec: Vec<f32> = (0..384).map(|i| (i as f32) * 0.01).collect();
    let blob = pack_f32_le(&vec);

    conn.execute(
        "INSERT INTO test_vecs (rowid, embedding) VALUES (?1, ?2)",
        rusqlite::params![1i64, &blob],
    )
    .expect("insert into vec0 table should succeed");

    let back: Vec<u8> = conn
        .query_row(
            "SELECT embedding FROM test_vecs WHERE rowid = ?1",
            rusqlite::params![1i64],
            |r| r.get(0),
        )
        .expect("select from vec0 table should succeed");

    assert_eq!(back.len(), 384 * 4, "embedding blob should be 384*4 bytes");
    assert_eq!(back, blob, "embedding round-trip should be byte-identical");
}
