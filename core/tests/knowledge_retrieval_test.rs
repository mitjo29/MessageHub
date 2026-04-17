use std::sync::Arc;

use messagehub_core::knowledge::{Embedder, Indexer, RetrievalFilters, Retriever};
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
fn test_retrieval_finds_relevant_chunk() {
    let vault = TempDir::new().unwrap();
    write(
        vault.path(),
        "05-People/Alice.md",
        "---\nname: Alice\n---\n# Alice\n## Role\nAlice is a software engineer focused on AI systems.\n",
    );
    write(
        vault.path(),
        "05-People/Bob.md",
        "---\nname: Bob\n---\n# Bob\n## Role\nBob is a chef who specializes in Italian cuisine.\n",
    );
    write(
        vault.path(),
        "01-Projects/ai-feature.md",
        "---\nname: AI Feature\n---\n# AI Feature\n## Goal\nBuild an AI-powered assistant.\n",
    );

    let store = Store::open_in_memory().unwrap();
    let embedder = Arc::new(Embedder::new().unwrap());
    Indexer::new(vault.path(), embedder.clone())
        .index_all(&store)
        .unwrap();

    let retriever = Retriever::new(embedder);
    let results = retriever
        .search(
            &store,
            "who works on machine learning?",
            &RetrievalFilters::default(),
        )
        .unwrap();
    assert!(!results.is_empty());
    // Alice's chunk should be among the top results (AI/software engineering is
    // closer to ML than cooking).
    let top_paths: Vec<&str> = results.iter().map(|r| r.file_path.as_str()).collect();
    assert!(
        top_paths.iter().any(|p| p.contains("Alice")),
        "expected Alice among top results, got {:?}",
        top_paths
    );
}

#[test]
#[ignore = "requires model download — ~120MB"]
fn test_para_folder_filter() {
    let vault = TempDir::new().unwrap();
    write(
        vault.path(),
        "05-People/Alice.md",
        "---\nname: Alice\n---\n# Alice\n## Role\nSoftware engineer.\n",
    );
    write(
        vault.path(),
        "01-Projects/Software.md",
        "---\nname: Software\n---\n# Software\n## Notes\nSoftware engineering guidelines.\n",
    );

    let store = Store::open_in_memory().unwrap();
    let embedder = Arc::new(Embedder::new().unwrap());
    Indexer::new(vault.path(), embedder.clone())
        .index_all(&store)
        .unwrap();

    let retriever = Retriever::new(embedder);
    let results = retriever
        .search(
            &store,
            "software engineer",
            &RetrievalFilters {
                para_folders: Some(vec!["05-People".to_string()]),
                top_k: Some(5),
            },
        )
        .unwrap();

    assert!(!results.is_empty());
    for r in &results {
        assert_eq!(r.para_folder.as_deref(), Some("05-People"));
    }
}
