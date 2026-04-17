use messagehub_core::ai::UserProfile;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_load_reads_file_content() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("user-profile.md");
    fs::write(
        &path,
        "# About me\nLanguages: EN, FR, DE\nRole: freelancer\n",
    )
    .unwrap();

    let profile = UserProfile::load(&path).unwrap();
    assert!(profile.content.contains("EN, FR, DE"));
    assert!(profile.content.contains("freelancer"));
}

#[test]
fn test_load_truncates_long_files_at_char_budget() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("user-profile.md");
    // 10_000 chars of 'x' — far exceeds the 4_000 char budget
    let big = "x".repeat(10_000);
    fs::write(&path, &big).unwrap();

    let profile = UserProfile::load(&path).unwrap();
    assert!(profile.content.len() <= 4_000);
    // A truncation marker should be present so downstream consumers
    // (and the LLM) know the profile was cut.
    assert!(profile.content.ends_with("[truncated]"));
}

#[test]
fn test_load_returns_empty_profile_when_missing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("does-not-exist.md");
    let profile = UserProfile::load(&path).unwrap();
    // Missing profile is not an error — the pipeline runs without it.
    assert!(profile.content.is_empty());
    assert!(!profile.has_content());
}

#[test]
fn test_has_content_distinguishes_empty_and_populated() {
    let empty = UserProfile { content: String::new() };
    let full = UserProfile {
        content: "something".to_string(),
    };
    assert!(!empty.has_content());
    assert!(full.has_content());
}
