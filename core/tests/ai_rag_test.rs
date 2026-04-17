use messagehub_core::ai::rag::build_rag_context;
use messagehub_core::ai::UserProfile;
use messagehub_core::knowledge::parse_markdown_file;
use messagehub_core::store::Store;
use messagehub_core::store::knowledge::IndexedFile;
use messagehub_core::types::Channel;

/// Build a store with a known 05-People person and a topic chunk,
/// using hand-crafted embeddings so no model download is needed.
fn seed_store(store: &Store) {
    let person_parsed = parse_markdown_file(
        "---\nname: Alice Example\nrole: Client\nemail: alice@example.com\n---\n## Role\nConsultant working on project X.",
    )
    .unwrap();
    let person_embedding: Vec<f32> = (0..384).map(|i| (i as f32) * 0.001).collect();
    let person = messagehub_core::knowledge::extract_person(
        "05-People/Alice Example.md",
        person_parsed.frontmatter.as_ref().unwrap(),
    )
    .unwrap()
    .unwrap();
    let person_file = IndexedFile {
        path: "05-People/Alice Example.md",
        mtime_secs: 0,
        para_folder: Some("05-People"),
        parsed: &person_parsed,
        chunk_embeddings: &[person_embedding.clone()],
        person: Some(&person),
    };
    store.upsert_indexed_file(&person_file).unwrap();

    let proj_parsed = parse_markdown_file("## Notes\nProject X planning milestones.").unwrap();
    let proj_file = IndexedFile {
        path: "01-Projects/Project X.md",
        mtime_secs: 0,
        para_folder: Some("01-Projects"),
        parsed: &proj_parsed,
        chunk_embeddings: &[person_embedding],
        person: None,
    };
    store.upsert_indexed_file(&proj_file).unwrap();
}

#[test]
fn test_build_rag_context_with_known_sender_and_no_retriever() {
    let store = Store::open_in_memory().unwrap();
    seed_store(&store);

    let profile = UserProfile {
        content: "Languages: EN, FR. Role: consultant.".to_string(),
    };
    let ctx = build_rag_context(
        &store,
        None, // no retriever -> no topic chunks
        &profile,
        Channel::Email,
        "alice@example.com",
        "About project X",
        "Can we sync tomorrow on milestones?",
    )
    .unwrap();

    assert_eq!(ctx.sender_name.as_deref(), Some("Alice Example"));
    assert_eq!(
        ctx.sender_vault_path.as_deref(),
        Some("05-People/Alice Example.md")
    );
    assert!(ctx.topic_chunks.is_empty());
    assert!(ctx.user_profile_content.contains("consultant"));
}

#[test]
fn test_build_rag_context_with_unknown_sender() {
    let store = Store::open_in_memory().unwrap();
    seed_store(&store);

    let profile = UserProfile {
        content: String::new(),
    };
    let ctx = build_rag_context(
        &store,
        None,
        &profile,
        Channel::Email,
        "stranger@other.com",
        "subject",
        "body",
    )
    .unwrap();

    assert!(ctx.sender_name.is_none());
    assert!(ctx.sender_vault_path.is_none());
    assert!(ctx.topic_chunks.is_empty());
}

#[test]
fn test_rag_context_to_prompt_section_formats_sender_and_chunks() {
    let ctx = messagehub_core::ai::rag::RagContext {
        sender_name: Some("Alice Example".to_string()),
        sender_vault_path: Some("05-People/Alice Example.md".to_string()),
        topic_chunks: vec![messagehub_core::ai::rag::ContextChunk {
            file_path: "01-Projects/Project X.md".to_string(),
            heading: Some("Notes".to_string()),
            content: "Project X planning milestones.".to_string(),
        }],
        user_profile_content: "Languages: EN, FR".to_string(),
    };
    let text = ctx.to_prompt_section();
    assert!(text.contains("Alice Example"));
    assert!(text.contains("05-People/Alice Example.md"));
    assert!(text.contains("Project X planning milestones"));
    assert!(text.contains("Languages: EN, FR"));
}

#[test]
fn test_rag_context_to_prompt_section_handles_empty_fields() {
    let ctx = messagehub_core::ai::rag::RagContext {
        sender_name: None,
        sender_vault_path: None,
        topic_chunks: vec![],
        user_profile_content: String::new(),
    };
    let text = ctx.to_prompt_section();
    // Should not panic, and should emit "unknown sender" so the LLM
    // knows the vault had no match (rather than silently omitting the section).
    assert!(text.to_lowercase().contains("unknown"));
}
