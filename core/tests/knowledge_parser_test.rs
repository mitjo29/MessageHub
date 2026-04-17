use messagehub_core::knowledge::parse_markdown_file;

const PERSON_FILE: &str = r#"---
type: person
name: "Alix Moreau"
role: "Daughter (youngest)"
tags: [person, family, children]
last-contact: "2026-04-12"
---

# Alix Moreau

## About
Jocelyn's youngest child. Born September 24, 2012.

## Personal
- **Date of birth**: September 24, 2012
- **Location**: Mertingen, Germany

## Notes
- Interested in becoming an architect.
"#;

const EMAIL_FILE: &str = r#"---
type: email-action
date: 2026-04-14
from: "School <mail@school.de>"
subject: "Elternbrief"
tags: [email, action-required, famille, alix]
priority: medium
priority-score: 4
---

# Rappel — École d'Alix

**From**: School
**Date**: 2026-04-14

## Contenu
Rappel pour les parents d'Alix.

## Actions To Do
- [ ] Lire l'Elternbrief
"#;

const PLAIN_FILE: &str = r#"# Just a title

Some paragraph text without frontmatter.

## A section
More text here.
"#;

#[test]
fn test_parse_person_file_extracts_frontmatter() {
    let parsed = parse_markdown_file(PERSON_FILE).unwrap();
    assert!(parsed.frontmatter.is_some());
    let fm = parsed.frontmatter.unwrap();
    assert_eq!(fm["name"].as_str().unwrap(), "Alix Moreau");
    assert_eq!(fm["role"].as_str().unwrap(), "Daughter (youngest)");
    let tags = fm["tags"].as_sequence().unwrap();
    assert_eq!(tags.len(), 3);
}

#[test]
fn test_parse_person_file_splits_sections() {
    let parsed = parse_markdown_file(PERSON_FILE).unwrap();
    // Sections: "Alix Moreau" (H1), "About", "Personal", "Notes"
    // The H1 is the preamble before any H2 — we keep it as a top-level section.
    assert!(parsed.sections.len() >= 3);
    let headings: Vec<&str> = parsed.sections.iter()
        .filter_map(|s| s.heading.as_deref())
        .collect();
    assert!(headings.contains(&"About"));
    assert!(headings.contains(&"Personal"));
    assert!(headings.contains(&"Notes"));
}

#[test]
fn test_parse_email_file() {
    let parsed = parse_markdown_file(EMAIL_FILE).unwrap();
    let fm = parsed.frontmatter.unwrap();
    assert_eq!(fm["priority-score"].as_i64().unwrap(), 4);
    assert_eq!(fm["type"].as_str().unwrap(), "email-action");
}

#[test]
fn test_parse_plain_file_no_frontmatter() {
    let parsed = parse_markdown_file(PLAIN_FILE).unwrap();
    assert!(parsed.frontmatter.is_none());
    assert!(!parsed.sections.is_empty());
}

#[test]
fn test_section_content_is_self_contained() {
    let parsed = parse_markdown_file(PERSON_FILE).unwrap();
    let about = parsed.sections.iter()
        .find(|s| s.heading.as_deref() == Some("About"))
        .expect("About section should exist");
    assert!(about.content.contains("youngest child"));
    // Should NOT contain content from later sections
    assert!(!about.content.contains("architect"));
}

#[test]
fn test_content_hash_is_deterministic() {
    let parsed1 = parse_markdown_file(PERSON_FILE).unwrap();
    let parsed2 = parse_markdown_file(PERSON_FILE).unwrap();
    assert_eq!(parsed1.content_hash, parsed2.content_hash);
}

#[test]
fn test_content_hash_changes_with_content() {
    let parsed1 = parse_markdown_file(PERSON_FILE).unwrap();
    let modified = PERSON_FILE.replace("youngest child", "YOUNGEST child");
    let parsed2 = parse_markdown_file(&modified).unwrap();
    assert_ne!(parsed1.content_hash, parsed2.content_hash);
}
