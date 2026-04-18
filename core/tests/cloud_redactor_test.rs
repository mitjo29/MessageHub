use messagehub_core::ai::cloud::{Redactor, ReverseMap};

fn build_standalone_redactor(people: &[&str]) -> Redactor {
    // Bypasses the vault; lets us test the trie logic without seeding a store.
    Redactor::from_names(people.iter().map(|s| s.to_string()).collect())
}

#[test]
fn test_redact_replaces_vault_name() {
    let r = build_standalone_redactor(&["Alice Example"]);
    let (out, map) = r.redact("Hi Alice Example, here's the update.");
    assert!(out.contains("[PERSON_1]"));
    assert!(!out.contains("Alice Example"));
    assert_eq!(map.get("[PERSON_1]"), Some(&"Alice Example".to_string()));
}

#[test]
fn test_redact_longest_match_wins_for_overlapping_names() {
    // "Alice Example" should win over "Alice".
    let r = build_standalone_redactor(&["Alice", "Alice Example"]);
    let (out, _map) = r.redact("Hi Alice Example!");
    // One token, not two. "Alice" standalone shouldn't also fire.
    let tokens: Vec<&str> = out.matches("[PERSON_").collect();
    assert_eq!(tokens.len(), 1);
}

#[test]
fn test_redact_is_case_insensitive_for_vault_names() {
    let r = build_standalone_redactor(&["Alice Example"]);
    let (out, map) = r.redact("spoke with ALICE EXAMPLE today");
    assert!(out.contains("[PERSON_1]"));
    // Reverse map preserves the *original* spelling from the input so
    // un_redact restores what the user saw.
    assert_eq!(map.get("[PERSON_1]"), Some(&"ALICE EXAMPLE".to_string()));
}

#[test]
fn test_redact_replaces_email_address() {
    let r = build_standalone_redactor(&[]);
    let (out, map) = r.redact("email me at alice@example.com please");
    assert!(out.contains("[EMAIL_1]"));
    assert!(!out.contains("alice@example.com"));
    assert_eq!(map.get("[EMAIL_1]"), Some(&"alice@example.com".to_string()));
}

#[test]
fn test_redact_same_email_gets_stable_token_in_one_call() {
    let r = build_standalone_redactor(&[]);
    let (out, _map) = r.redact("mail alice@example.com then alice@example.com again");
    // Two occurrences, one token reused.
    let count = out.matches("[EMAIL_1]").count();
    assert_eq!(count, 2);
    assert!(!out.contains("[EMAIL_2]"));
}

#[test]
fn test_redact_replaces_phone_number() {
    let r = build_standalone_redactor(&[]);
    let (out, map) = r.redact("call me at +41 79 123 45 67 anytime");
    assert!(out.contains("[PHONE_1]"));
    assert!(map.get("[PHONE_1]").unwrap().contains("79"));
}

#[test]
fn test_redact_leaves_short_number_sequences_alone() {
    // "order 12345" is 5 chars, below the phone regex minimum.
    let r = build_standalone_redactor(&[]);
    let (out, _map) = r.redact("see order 12345 in dashboard");
    assert!(!out.contains("[PHONE_"));
    assert!(out.contains("12345"));
}

#[test]
fn test_un_redact_round_trips() {
    let r = build_standalone_redactor(&["Alice Example"]);
    let (redacted, map) = r.redact("Hi Alice Example, reach me at alice@example.com");
    assert!(redacted.contains("[PERSON_1]"));
    assert!(redacted.contains("[EMAIL_1]"));
    let restored = Redactor::un_redact(&redacted, &map);
    assert_eq!(restored, "Hi Alice Example, reach me at alice@example.com");
}

#[test]
fn test_un_redact_passthrough_when_map_empty() {
    let empty: ReverseMap = ReverseMap::new();
    let out = Redactor::un_redact("no tokens here", &empty);
    assert_eq!(out, "no tokens here");
}
