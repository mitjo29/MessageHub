use std::sync::Arc;

use messagehub_core::knowledge::{Embedder, Indexer};
use messagehub_core::store::Store;
use tempfile::TempDir;

fn write(dir: &std::path::Path, rel: &str, content: &str) {
    let full = dir.join(rel);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(full, content).unwrap();
}

#[test]
#[ignore = "requires model download — ~120MB"]
fn test_index_small_vault() {
    let vault = TempDir::new().unwrap();

    write(
        vault.path(),
        "05-People/Alice.md",
        r#"---
type: person
name: "Alice Example"
role: "Designer"
tags: [person, work]
email: "alice@example.com"
---

# Alice Example

## About
Alice is a designer working on the new dashboard.

## Projects
Currently leading the onboarding redesign.
"#,
    );

    write(
        vault.path(),
        "01-Projects/Dashboard.md",
        r#"---
type: project
name: "Dashboard Redesign"
status: active
---

# Dashboard Redesign

## Goal
Simplify the main navigation.

## Status
In design review with Alice.
"#,
    );

    write(vault.path(), "notes.md", "# Loose note\n\nJust some text.");

    let store = Store::open_in_memory().unwrap();
    let embedder = Arc::new(Embedder::new().unwrap());
    let indexer = Indexer::new(vault.path(), embedder);

    let report = indexer.index_all(&store).unwrap();
    assert_eq!(report.files_scanned, 3);
    assert_eq!(report.files_indexed, 3);
    assert_eq!(report.files_reindexed, 0);
    assert_eq!(report.people_indexed, 1);

    // Re-run — should skip everything.
    let report2 = indexer.index_all(&store).unwrap();
    assert_eq!(report2.files_skipped, 3);
    assert_eq!(report2.files_reindexed, 0);
}

#[test]
#[ignore = "requires model download — ~120MB"]
fn test_person_address_lookup() {
    let vault = TempDir::new().unwrap();
    write(
        vault.path(),
        "05-People/Alice.md",
        r#"---
name: "Alice"
email: "alice@example.com"
telegram: "@alice_dev"
---

# Alice
## Notes
A test person.
"#,
    );

    let store = Store::open_in_memory().unwrap();
    let embedder = Arc::new(Embedder::new().unwrap());
    Indexer::new(vault.path(), embedder)
        .index_all(&store)
        .unwrap();

    let via_email = store
        .find_vault_person_by_address("Email", "alice@example.com")
        .unwrap();
    assert!(via_email.is_some());
    assert_eq!(via_email.unwrap().0, "Alice");

    let via_telegram = store
        .find_vault_person_by_address("Telegram", "@alice_dev")
        .unwrap();
    assert!(via_telegram.is_some());

    let not_found = store
        .find_vault_person_by_address("Email", "nobody@example.com")
        .unwrap();
    assert!(not_found.is_none());
}
