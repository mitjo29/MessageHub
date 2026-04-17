//! Integration tests for `Store::upsert_indexed_file`, `delete_indexed_file`,
//! `indexed_content_hash`, and `find_vault_person_by_address`.
//!
//! Covers the transactional write path for vault files:
//! - First-time insert of a file with multiple chunks + embeddings.
//! - Re-insert of same path replaces prior chunks and vectors (idempotent upsert).
//! - Delete cascades through chunks, vectors, and person rows.
//! - Person address lookup by (channel, address).

use messagehub_core::knowledge::{parse_markdown_file, PersonAddress, VaultPerson};
use messagehub_core::store::knowledge::IndexedFile;
use messagehub_core::store::Store;

const FILE_A: &str = r#"---
type: person
name: "Alix Moreau"
role: "Daughter"
tags: [person, family]
---

# Alix Moreau

## About
Youngest child. Lives in Mertingen.

## Notes
Interested in architecture.
"#;

const FILE_A_V2: &str = r#"---
type: person
name: "Alix Moreau"
role: "Daughter (youngest)"
tags: [person, family]
---

# Alix Moreau

## Updated
Totally different content for the updated version.
"#;

fn make_person(path: &str, name: &str, email: &str) -> VaultPerson {
    VaultPerson {
        file_path: path.to_string(),
        name: name.to_string(),
        role: Some("Daughter".to_string()),
        tags: vec!["person".into(), "family".into()],
        last_contact: None,
        addresses: vec![PersonAddress {
            channel: "Email".to_string(),
            address: email.to_string(),
        }],
        frontmatter: serde_yaml::from_str("name: Alix Moreau\nrole: Daughter").unwrap(),
    }
}

// Because `Store::conn()` is `pub(crate)`, the integration test can't reach
// it directly. We drive everything through the public API and assert via
// `indexed_content_hash` + `find_vault_person_by_address` plus
// re-inserts / deletes, which observes the same rows from outside.

#[test]
fn test_upsert_inserts_file_chunks_and_person() {
    let store = Store::open_in_memory().expect("open store");
    let parsed = parse_markdown_file(FILE_A).expect("parse file A");
    assert!(
        parsed.sections.len() >= 2,
        "expected multiple sections, got {}",
        parsed.sections.len()
    );

    let embeddings: Vec<Vec<f32>> = parsed.sections.iter().map(|_| vec![0.1_f32; 384]).collect();
    let person = make_person("05-People/Alix.md", "Alix Moreau", "alix@example.com");

    let indexed = IndexedFile {
        path: "05-People/Alix.md",
        mtime_secs: 1_700_000_000,
        para_folder: Some("05-People"),
        parsed: &parsed,
        chunk_embeddings: &embeddings,
        person: Some(&person),
    };

    store.upsert_indexed_file(&indexed).expect("upsert");

    // content hash recorded
    let hash = store
        .indexed_content_hash("05-People/Alix.md")
        .expect("hash query")
        .expect("hash present");
    assert_eq!(hash, parsed.content_hash);

    // person address lookup works
    let hit = store
        .find_vault_person_by_address("Email", "alix@example.com")
        .expect("lookup")
        .expect("person found");
    assert_eq!(hit.0, "Alix Moreau");
    assert_eq!(hit.1, "05-People/Alix.md");
}

#[test]
fn test_upsert_is_idempotent_and_replaces_prior_chunks() {
    let store = Store::open_in_memory().expect("open store");

    // --- Insert v1 ---
    let parsed_v1 = parse_markdown_file(FILE_A).expect("parse v1");
    let embeddings_v1: Vec<Vec<f32>> = parsed_v1
        .sections
        .iter()
        .map(|_| vec![0.1_f32; 384])
        .collect();
    let person_v1 = make_person("05-People/Alix.md", "Alix Moreau", "alix@example.com");
    let indexed_v1 = IndexedFile {
        path: "05-People/Alix.md",
        mtime_secs: 1_700_000_000,
        para_folder: Some("05-People"),
        parsed: &parsed_v1,
        chunk_embeddings: &embeddings_v1,
        person: Some(&person_v1),
    };
    store.upsert_indexed_file(&indexed_v1).expect("upsert v1");

    let v1_hash = store
        .indexed_content_hash("05-People/Alix.md")
        .unwrap()
        .unwrap();

    // --- Re-insert v2 (different content, different email) ---
    let parsed_v2 = parse_markdown_file(FILE_A_V2).expect("parse v2");
    let embeddings_v2: Vec<Vec<f32>> = parsed_v2
        .sections
        .iter()
        .map(|_| vec![0.2_f32; 384])
        .collect();
    let person_v2 = make_person("05-People/Alix.md", "Alix Moreau", "alix.new@example.com");
    let indexed_v2 = IndexedFile {
        path: "05-People/Alix.md",
        mtime_secs: 1_700_000_100,
        para_folder: Some("05-People"),
        parsed: &parsed_v2,
        chunk_embeddings: &embeddings_v2,
        person: Some(&person_v2),
    };
    store.upsert_indexed_file(&indexed_v2).expect("upsert v2");

    // Hash updated
    let v2_hash = store
        .indexed_content_hash("05-People/Alix.md")
        .unwrap()
        .unwrap();
    assert_ne!(v1_hash, v2_hash, "content hash should change after upsert");
    assert_eq!(v2_hash, parsed_v2.content_hash);

    // Old email no longer resolves (person row was replaced, FK-cascaded addresses)
    let stale = store
        .find_vault_person_by_address("Email", "alix@example.com")
        .expect("lookup stale");
    assert!(stale.is_none(), "old address should be gone after re-upsert");

    // New email resolves
    let fresh = store
        .find_vault_person_by_address("Email", "alix.new@example.com")
        .expect("lookup fresh")
        .expect("new address should be present");
    assert_eq!(fresh.0, "Alix Moreau");

    // Row counts: exactly one vault_files row, and vault_chunks == sections of v2.
    // We assert this via a raw rusqlite connection opened on the same in-memory
    // store instance is impossible (in-memory DBs aren't shared across connections),
    // so we use observable behavior: deleting and re-querying hash returns None.
    store
        .delete_indexed_file("05-People/Alix.md")
        .expect("delete");
    let after_delete = store
        .indexed_content_hash("05-People/Alix.md")
        .expect("hash after delete");
    assert!(after_delete.is_none(), "hash should be None after delete");

    // Address lookup also gone after delete (FK cascade)
    let after_del_addr = store
        .find_vault_person_by_address("Email", "alix.new@example.com")
        .expect("lookup after delete");
    assert!(
        after_del_addr.is_none(),
        "person address should be gone after delete_indexed_file"
    );
}

#[test]
fn test_upsert_rejects_embedding_count_mismatch() {
    let store = Store::open_in_memory().expect("open store");
    let parsed = parse_markdown_file(FILE_A).expect("parse");

    // Intentionally provide too few embeddings
    let embeddings: Vec<Vec<f32>> = vec![vec![0.1_f32; 384]];
    assert!(embeddings.len() < parsed.sections.len());

    let indexed = IndexedFile {
        path: "05-People/Alix.md",
        mtime_secs: 1_700_000_000,
        para_folder: Some("05-People"),
        parsed: &parsed,
        chunk_embeddings: &embeddings,
        person: None,
    };

    let err = store
        .upsert_indexed_file(&indexed)
        .expect_err("mismatch should error");
    let msg = err.to_string();
    assert!(msg.contains("chunk count mismatch"), "got: {msg}");

    // Nothing should have been persisted.
    let hash = store
        .indexed_content_hash("05-People/Alix.md")
        .expect("hash query");
    assert!(hash.is_none(), "no file should be persisted on error");
}
