# Plan 3: Knowledge Engine — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the knowledge layer that ingests an Obsidian vault, parses YAML frontmatter (with special handling for `05-People/` files), chunks markdown by section headings, generates multilingual embeddings via `fastembed`, and stores them in `sqlite-vec` virtual tables for semantic retrieval.

**Architecture:** A `knowledge` module in `core/src/knowledge/` with four components: (1) `parser` extracts frontmatter + sections from markdown files, (2) `embedder` wraps `fastembed` with the E5 prefix convention (`passage:` for stored chunks, `query:` for searches), (3) `indexer` orchestrates file watching and incremental updates, (4) `retrieval` performs semantic search with optional PARA folder and channel filters. People files get dual indexing — structured YAML goes into a `vault_people` table for O(log n) sender lookup, AND the full content is chunked and embedded for semantic retrieval.

**Tech Stack:** `fastembed` 5.x (ONNX Runtime, auto-downloads multilingual-e5-small), `sqlite-vec` 0.1 (vector virtual tables), `serde_yaml` (YAML frontmatter parsing), `notify` (file watcher), `pulldown-cmark` (markdown parsing), `blake3` (file content hashing for incremental updates)

---

## File Structure

```
core/
├── Cargo.toml                          # MODIFY — add fastembed, sqlite-vec, serde_yaml, notify, pulldown-cmark, blake3
├── migrations/
│   └── 002_knowledge.sql               # CREATE — vault_files, vault_chunks (vec0), vault_people tables
├── src/
│   ├── lib.rs                          # MODIFY — add `pub mod knowledge;`
│   ├── error.rs                        # MODIFY — add Knowledge, Embedding, VaultParse variants
│   ├── store/
│   │   └── migrations.rs               # MODIFY — register 002 migration
│   └── knowledge/
│       ├── mod.rs                      # CREATE — module root, public API
│       ├── parser.rs                   # CREATE — markdown + YAML frontmatter parsing, section chunking
│       ├── embedder.rs                 # CREATE — fastembed wrapper with E5 prefix convention
│       ├── people.rs                   # CREATE — structured people profile extraction
│       ├── indexer.rs                  # CREATE — vault ingestion, incremental updates, file watcher
│       └── retrieval.rs                # CREATE — semantic search with filters
└── tests/
    ├── knowledge_parser_test.rs        # CREATE — parser unit tests
    ├── knowledge_indexer_test.rs       # CREATE — full vault ingestion tests
    └── knowledge_retrieval_test.rs     # CREATE — semantic search tests
```

---

### Task 1: Database Schema + Migration

**Files:**
- Create: `core/migrations/002_knowledge.sql`
- Modify: `core/src/store/migrations.rs`
- Test: via integration tests

- [ ] **Step 1: Create `core/migrations/002_knowledge.sql`**

```sql
-- Tracks every indexed markdown file for incremental updates.
CREATE TABLE IF NOT EXISTS vault_files (
    path TEXT PRIMARY KEY,              -- Relative path from vault root, e.g. "05-People/Alix Moreau.md"
    content_hash TEXT NOT NULL,         -- blake3 hash of file content
    mtime_secs INTEGER NOT NULL,        -- File modification time (Unix timestamp)
    frontmatter_json TEXT,              -- Parsed YAML frontmatter as JSON (nullable — files may have none)
    para_folder TEXT,                   -- "00-Inbox", "01-Projects", "05-People", etc. (nullable)
    indexed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_vault_files_para ON vault_files(para_folder);

-- One row per chunk. Joined with the vec virtual table on `id`.
CREATE TABLE IF NOT EXISTS vault_chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL REFERENCES vault_files(path) ON DELETE CASCADE,
    section_heading TEXT,               -- The ## heading that introduced this chunk, if any
    chunk_index INTEGER NOT NULL,       -- Order within the file (0-based)
    content TEXT NOT NULL,              -- The actual chunk text (without E5 prefix)
    token_count INTEGER NOT NULL,       -- Approximate token count for budget calculations
    para_folder TEXT                    -- Denormalized from vault_files for fast filtering
);
CREATE INDEX IF NOT EXISTS idx_vault_chunks_file ON vault_chunks(file_path);
CREATE INDEX IF NOT EXISTS idx_vault_chunks_para ON vault_chunks(para_folder);

-- sqlite-vec virtual table: 384-dim vectors (multilingual-e5-small output dimension).
-- The ROWID here matches vault_chunks.id — we keep them in sync via insert/delete triggers below.
CREATE VIRTUAL TABLE IF NOT EXISTS vault_chunk_vecs USING vec0(
    embedding FLOAT[384]
);

-- Structured people profiles extracted from 05-People/*.md frontmatter.
-- This is the "structured" half of dual-indexing — the chunks themselves
-- also go into vault_chunks for semantic search.
CREATE TABLE IF NOT EXISTS vault_people (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL UNIQUE REFERENCES vault_files(path) ON DELETE CASCADE,
    name TEXT NOT NULL,
    role TEXT,                          -- e.g. "Daughter (youngest)"
    tags_json TEXT,                     -- JSON array: ["person", "family", "children"]
    last_contact TEXT,                  -- ISO date string
    frontmatter_json TEXT NOT NULL      -- Full frontmatter preserved as JSON for flexible queries
);
CREATE INDEX IF NOT EXISTS idx_vault_people_name ON vault_people(name);

-- Contact-to-vault-person link. Populated when a message sender matches a
-- known address from this person's profile (via vault_people.frontmatter_json
-- email/phone fields — parsed at query time).
CREATE TABLE IF NOT EXISTS vault_people_addresses (
    person_id INTEGER NOT NULL REFERENCES vault_people(id) ON DELETE CASCADE,
    channel_type TEXT NOT NULL,
    address TEXT NOT NULL,
    PRIMARY KEY (person_id, channel_type, address)
);
CREATE INDEX IF NOT EXISTS idx_vault_people_addresses_lookup
    ON vault_people_addresses(channel_type, address);
```

- [ ] **Step 2: Register the migration in `core/src/store/migrations.rs`**

Add to the `MIGRATIONS` slice:

```rust
const MIGRATIONS: &[(&str, &str)] = &[
    ("001_initial", include_str!("../../migrations/001_initial.sql")),
    ("002_knowledge", include_str!("../../migrations/002_knowledge.sql")),
];
```

- [ ] **Step 3: The `sqlite-vec` extension must be loaded before the migration runs**

Modify `core/src/store/mod.rs` `Store::open` and `Store::open_in_memory` to load the extension. Replace the contents of `mod.rs`:

```rust
pub mod channels;
pub mod contacts;
pub mod messages;
mod migrations;

use std::path::Path;
use rusqlite::Connection;
use tracing::info;

use crate::error::Result;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path, password: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "key", password)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", "5000")?;

        // Load sqlite-vec extension (must happen before migrations that use vec0)
        unsafe {
            conn.load_extension_enable()?;
            sqlite_vec::sqlite3_vec_init(&conn)?;
            conn.load_extension_disable()?;
        }

        migrations::run_migrations(&conn)?;

        info!(path = %path.display(), "database opened");
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        unsafe {
            conn.load_extension_enable()?;
            sqlite_vec::sqlite3_vec_init(&conn)?;
            conn.load_extension_disable()?;
        }

        migrations::run_migrations(&conn)?;

        Ok(Self { conn })
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
}
```

**Note:** The `sqlite_vec` crate provides a `sqlite3_vec_init` function that registers the extension with a live `Connection`. Check the crate's docs for the exact calling convention — the API may be `sqlite_vec::load(&conn)` or similar. Adapt as needed once you see the actual crate API.

- [ ] **Step 4: Run existing tests — they must still pass**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo test -p messagehub-core
```

- [ ] **Step 5: Commit**

```bash
git add core/migrations/002_knowledge.sql core/src/store/migrations.rs core/src/store/mod.rs core/Cargo.toml Cargo.lock
git commit -m "feat: add knowledge engine schema (vault_files, vault_chunks, vault_people)

Adds sqlite-vec virtual table for 384-dim embeddings, structured people
profile table with address lookup index, and file tracking table for
incremental updates via content hashing.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Markdown + Frontmatter Parser

**Files:**
- Create: `core/src/knowledge/mod.rs`
- Create: `core/src/knowledge/parser.rs`
- Create: `core/tests/knowledge_parser_test.rs`
- Modify: `core/Cargo.toml` — add `serde_yaml`, `pulldown-cmark`, `blake3`
- Modify: `core/src/lib.rs` — add `pub mod knowledge;`
- Modify: `core/src/error.rs` — add `VaultParse` variant

- [ ] **Step 1: Add dependencies to `core/Cargo.toml`**

Add to the `[dependencies]` section:

```toml
fastembed = { version = "5", default-features = false, features = ["hf-hub-native-tls", "ort-download-binaries-native-tls"] }
sqlite-vec = "0.1"
serde_yaml = "0.9"
pulldown-cmark = "0.12"
blake3 = "1"
notify = "7"
```

- [ ] **Step 2: Add error variants to `core/src/error.rs`**

Add to the `CoreError` enum:

```rust
    #[error("vault parse error: {0}")]
    VaultParse(String),

    #[error("embedding error: {0}")]
    Embedding(String),

    #[error("knowledge engine error: {0}")]
    Knowledge(String),
```

- [ ] **Step 3: Add `pub mod knowledge;` to `core/src/lib.rs`**

```rust
pub mod error;
pub mod types;
pub mod store;
pub mod adapters;
pub mod knowledge;
```

- [ ] **Step 4: Create `core/src/knowledge/mod.rs`**

```rust
pub mod parser;
// embedder, people, indexer, retrieval — added in later tasks

pub use parser::{ParsedFile, Section, parse_markdown_file};
```

- [ ] **Step 5: Write failing tests at `core/tests/knowledge_parser_test.rs`**

```rust
use messagehub_core::knowledge::{parse_markdown_file, ParsedFile};

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
```

- [ ] **Step 6: Run tests to verify they fail**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo test --test knowledge_parser_test 2>&1 | head -20
```

Expected: compilation errors because `parse_markdown_file` doesn't exist yet.

- [ ] **Step 7: Implement the parser at `core/src/knowledge/parser.rs`**

```rust
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// Result of parsing a markdown file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedFile {
    /// YAML frontmatter as a generic JSON value (None if file has no frontmatter).
    pub frontmatter: Option<serde_yaml::Value>,
    /// The body split into sections at `#`/`##` headings.
    pub sections: Vec<Section>,
    /// Blake3 hash of the full file content (for incremental-update detection).
    pub content_hash: String,
    /// Approximate total token count (body only — used for budget logging).
    pub total_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    /// The heading that introduced this section (None for content before the first heading).
    pub heading: Option<String>,
    /// The section body text (heading is not included in content).
    pub content: String,
    /// Heading level (1-6). 0 if there's no heading.
    pub level: u8,
    /// Approximate token count for this section.
    pub tokens: usize,
}

/// Parse a markdown file into frontmatter + sections.
///
/// Frontmatter is YAML between `---` delimiters at the top of the file.
/// Sections are split at `#` and `##` heading boundaries. Content before
/// the first heading (e.g., a preamble paragraph after the frontmatter)
/// becomes a section with `heading = None` and `level = 0`.
pub fn parse_markdown_file(content: &str) -> Result<ParsedFile> {
    let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();

    let (frontmatter, body) = split_frontmatter(content)?;
    let sections = split_sections(body);
    let total_tokens = sections.iter().map(|s| s.tokens).sum();

    Ok(ParsedFile {
        frontmatter,
        sections,
        content_hash,
        total_tokens,
    })
}

/// Split a markdown file into (frontmatter, body).
///
/// Returns `(None, full_content)` if the file doesn't start with `---`.
fn split_frontmatter(content: &str) -> Result<(Option<serde_yaml::Value>, &str)> {
    let trimmed = content.trim_start_matches('\u{feff}'); // Strip BOM if present
    if !trimmed.starts_with("---") {
        return Ok((None, trimmed));
    }

    // Find the closing `---` on its own line.
    // The opening `---` is at position 0.
    let after_opening = &trimmed[3..];
    let after_opening = after_opening.strip_prefix('\n').unwrap_or(after_opening);

    let close_pos = find_frontmatter_close(after_opening);
    match close_pos {
        Some(pos) => {
            let yaml_str = &after_opening[..pos];
            let body_start = pos + after_opening[pos..].find('\n').unwrap_or(pos) + 1;
            let body = after_opening.get(body_start.min(after_opening.len())..).unwrap_or("");

            let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str)
                .map_err(|e| CoreError::VaultParse(format!("invalid YAML frontmatter: {}", e)))?;

            Ok((Some(fm), body))
        }
        None => {
            // No closing `---` found; treat whole file as body (unusual but possible).
            Ok((None, trimmed))
        }
    }
}

/// Find the byte offset of a line that is exactly `---` in `s`.
fn find_frontmatter_close(s: &str) -> Option<usize> {
    let mut pos = 0;
    for line in s.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(|c: char| c == '\n' || c == '\r');
        if trimmed == "---" {
            return Some(pos);
        }
        pos += line.len();
    }
    None
}

/// Split a markdown body into sections at heading boundaries.
///
/// Headings of level 1 or 2 start new sections. Deeper headings (###+) stay
/// inside the current section — section boundaries should be coarse enough
/// that each chunk is substantial but small enough for embedding context.
fn split_sections(body: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut current_level: u8 = 0;
    let mut current_content = String::new();

    for line in body.lines() {
        let (heading_level, heading_text) = parse_heading(line);

        if let (Some(level), Some(text)) = (heading_level, heading_text) {
            if level <= 2 {
                // Flush the current section before starting a new one.
                if !current_content.trim().is_empty() || current_heading.is_some() {
                    sections.push(make_section(
                        current_heading.take(),
                        current_level,
                        std::mem::take(&mut current_content),
                    ));
                }
                current_heading = Some(text.to_string());
                current_level = level;
                continue;
            }
        }

        current_content.push_str(line);
        current_content.push('\n');
    }

    if !current_content.trim().is_empty() || current_heading.is_some() {
        sections.push(make_section(current_heading, current_level, current_content));
    }

    sections
}

/// Parse a line and return `(level, text)` if it's an ATX heading, else `(None, None)`.
fn parse_heading(line: &str) -> (Option<u8>, Option<&str>) {
    let trimmed = line.trim_start();
    let mut level = 0u8;
    let mut chars = trimmed.chars();
    while chars.next() == Some('#') {
        level += 1;
    }
    if level == 0 || level > 6 {
        return (None, None);
    }
    // Require a space after the #s (ATX heading rule).
    let after_hashes = &trimmed[(level as usize)..];
    if !after_hashes.starts_with(' ') {
        return (None, None);
    }
    let text = after_hashes.trim();
    if text.is_empty() {
        return (None, None);
    }
    (Some(level), Some(text))
}

fn make_section(heading: Option<String>, level: u8, content: String) -> Section {
    let content = content.trim_end().to_string();
    let tokens = approx_token_count(&content);
    Section {
        heading,
        content,
        level,
        tokens,
    }
}

/// Rough token count heuristic: ~4 characters per token for English/French/German prose.
/// This is not a real tokenizer — it's just for budget planning.
pub fn approx_token_count(s: &str) -> usize {
    (s.chars().count() + 3) / 4
}
```

- [ ] **Step 8: Run parser tests — all 7 must pass**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo test --test knowledge_parser_test
```

- [ ] **Step 9: Commit**

```bash
git add core/src/knowledge/ core/tests/knowledge_parser_test.rs core/src/lib.rs core/src/error.rs core/Cargo.toml Cargo.lock
git commit -m "feat: add markdown + YAML frontmatter parser with section chunking

Parses Obsidian vault files into frontmatter + sections split at
H1/H2 boundaries. Computes blake3 content hash for incremental
update detection. Deeper headings (###+) stay within their parent
section to keep chunks semantically coherent.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Fastembed Embedder with E5 Prefix Convention

**Files:**
- Create: `core/src/knowledge/embedder.rs`
- Modify: `core/src/knowledge/mod.rs`

`★ Why this matters:` The E5 family (including `multilingual-e5-small`) was trained with a specific prefix convention. Stored document chunks get `passage: <text>`, query strings get `query: <text>`. Skipping the prefix drops retrieval quality by 10-20% in benchmarks. This wrapper enforces the convention so the rest of the codebase can ignore it.

- [ ] **Step 1: Implement the embedder at `core/src/knowledge/embedder.rs`**

```rust
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::error::{CoreError, Result};

/// Output dimension of multilingual-e5-small.
pub const EMBEDDING_DIM: usize = 384;

/// Wraps `fastembed::TextEmbedding` with the E5 prefix convention.
///
/// Stored chunks are prefixed with `passage: ` before embedding.
/// Query strings are prefixed with `query: ` before embedding.
/// Callers pass raw text — this struct handles the prefixes.
pub struct Embedder {
    model: TextEmbedding,
}

impl Embedder {
    /// Create an embedder using multilingual-e5-small.
    /// On first use, the model (~120MB) downloads automatically from HuggingFace
    /// and caches under `$HOME/.cache/fastembed/`.
    pub fn new() -> Result<Self> {
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::MultilingualE5Small)
                .with_show_download_progress(false),
        )
        .map_err(|e| CoreError::Embedding(format!("failed to init embedding model: {}", e)))?;

        Ok(Self { model })
    }

    /// Embed a batch of document chunks. Caller should pass raw text — this function adds
    /// the `passage:` prefix required by E5.
    pub fn embed_passages(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let prefixed: Vec<String> = texts.iter().map(|t| format!("passage: {}", t)).collect();
        let refs: Vec<&str> = prefixed.iter().map(|s| s.as_str()).collect();
        self.model
            .embed(refs, None)
            .map_err(|e| CoreError::Embedding(format!("embed_passages failed: {}", e)))
    }

    /// Embed a single query string. Adds the `query:` prefix required by E5.
    pub fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let prefixed = format!("query: {}", text);
        let mut out = self
            .model
            .embed(vec![prefixed.as_str()], None)
            .map_err(|e| CoreError::Embedding(format!("embed_query failed: {}", e)))?;
        out.pop()
            .ok_or_else(|| CoreError::Embedding("empty embedding result".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests require downloading the model (~120MB). They're marked #[ignore]
    // so they don't run by default. Run explicitly with:
    //   cargo test --test knowledge_parser_test -- --ignored
    // (or whichever test file is configured to run them).

    #[test]
    #[ignore = "requires model download — ~120MB"]
    fn test_embed_passages_dims() {
        let embedder = Embedder::new().unwrap();
        let result = embedder.embed_passages(&["hello world", "another passage"]).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].len(), EMBEDDING_DIM);
        assert_eq!(result[1].len(), EMBEDDING_DIM);
    }

    #[test]
    #[ignore = "requires model download — ~120MB"]
    fn test_embed_query_dims() {
        let embedder = Embedder::new().unwrap();
        let result = embedder.embed_query("test query").unwrap();
        assert_eq!(result.len(), EMBEDDING_DIM);
    }

    #[test]
    #[ignore = "requires model download — ~120MB"]
    fn test_similar_texts_have_close_embeddings() {
        let embedder = Embedder::new().unwrap();
        let vecs = embedder
            .embed_passages(&[
                "The dog chased the cat across the yard",
                "A canine pursued a feline through the garden",
                "Quantum mechanics and the Schrodinger equation",
            ])
            .unwrap();

        let sim_ab = cosine(&vecs[0], &vecs[1]);
        let sim_ac = cosine(&vecs[0], &vecs[2]);
        assert!(sim_ab > sim_ac, "paraphrase should be more similar than unrelated text");
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        dot / (mag_a * mag_b)
    }
}
```

- [ ] **Step 2: Update `core/src/knowledge/mod.rs`**

```rust
pub mod embedder;
pub mod parser;

pub use embedder::{Embedder, EMBEDDING_DIM};
pub use parser::{parse_markdown_file, ParsedFile, Section};
```

- [ ] **Step 3: Verify compilation (don't run ignored tests)**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo check -p messagehub-core
```

- [ ] **Step 4: Commit**

```bash
git add core/src/knowledge/embedder.rs core/src/knowledge/mod.rs
git commit -m "feat: add fastembed wrapper with E5 prefix convention

Uses multilingual-e5-small (384 dims) via fastembed's ONNX runtime.
Automatically adds 'passage:' prefix for stored chunks and 'query:'
prefix for search queries, as required by the E5 family of models.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: People Profile Extractor

**Files:**
- Create: `core/src/knowledge/people.rs`
- Modify: `core/src/knowledge/mod.rs`

`★ Why this matters:` The user's 05-People/ folder has a specific frontmatter schema. This task extracts that into a structured `VaultPerson` value that gets persisted to `vault_people` for O(log n) sender lookup. The schema is permissive — we extract what's there and tolerate missing fields rather than failing.

- [ ] **Step 1: Implement at `core/src/knowledge/people.rs`**

```rust
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::error::Result;

/// Structured data extracted from a `05-People/*.md` frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultPerson {
    pub file_path: String,
    pub name: String,
    pub role: Option<String>,
    pub tags: Vec<String>,
    pub last_contact: Option<String>,
    /// Discovered addresses grouped by channel (e.g. email → [a@b.com, c@d.com]).
    pub addresses: Vec<PersonAddress>,
    /// Full frontmatter preserved for any downstream consumer that needs it.
    pub frontmatter: serde_yaml::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonAddress {
    /// Channel identifier: "Email", "Telegram", "WhatsApp", "Sms", "Teams".
    /// Matches `types::Channel::to_db_str()` so a direct lookup join works.
    pub channel: String,
    pub address: String,
}

/// Extract a `VaultPerson` from a parsed 05-People file.
///
/// Returns `None` if the file doesn't look like a person profile
/// (no frontmatter, or `type` != "person" when `type` is present).
/// This gate lets the indexer safely call `extract_person` on every
/// 05-People file without custom routing logic upstream.
pub fn extract_person(file_path: &str, frontmatter: &Value) -> Result<Option<VaultPerson>> {
    // Require a name. If frontmatter lacks a name field, this isn't a person profile.
    let name = match frontmatter.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return Ok(None),
    };

    // If `type` is present, it must equal "person" (or be absent/None).
    if let Some(t) = frontmatter.get("type").and_then(|v| v.as_str()) {
        if t != "person" {
            return Ok(None);
        }
    }

    let role = frontmatter.get("role").and_then(|v| v.as_str()).map(String::from);
    let last_contact = frontmatter
        .get("last-contact")
        .and_then(|v| v.as_str())
        .map(String::from);

    let tags = frontmatter
        .get("tags")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let addresses = extract_addresses(frontmatter);

    Ok(Some(VaultPerson {
        file_path: file_path.to_string(),
        name,
        role,
        tags,
        last_contact,
        addresses,
        frontmatter: frontmatter.clone(),
    }))
}

/// Extract addresses from common frontmatter fields.
///
/// Recognized fields:
/// - `email` / `emails` / `email-accounts[*].address` → Email
/// - `telegram` / `telegram-username` → Telegram
/// - `whatsapp` / `phone` (if whatsapp-enabled flag is true) / `phone-whatsapp` → WhatsApp
/// - `sms` / `phone-sms` → Sms
/// - `teams` / `teams-email` → Teams
fn extract_addresses(frontmatter: &Value) -> Vec<PersonAddress> {
    let mut out = Vec::new();

    // Email
    collect_string_or_list(frontmatter.get("email"), &mut out, "Email");
    collect_string_or_list(frontmatter.get("emails"), &mut out, "Email");
    if let Some(accounts) = frontmatter.get("email-accounts").and_then(|v| v.as_sequence()) {
        for acct in accounts {
            if let Some(addr) = acct.get("address").and_then(|v| v.as_str()) {
                out.push(PersonAddress {
                    channel: "Email".to_string(),
                    address: addr.to_string(),
                });
            }
        }
    }

    // Telegram
    collect_string_or_list(frontmatter.get("telegram"), &mut out, "Telegram");
    collect_string_or_list(frontmatter.get("telegram-username"), &mut out, "Telegram");

    // WhatsApp
    collect_string_or_list(frontmatter.get("whatsapp"), &mut out, "WhatsApp");
    collect_string_or_list(frontmatter.get("phone-whatsapp"), &mut out, "WhatsApp");

    // SMS
    collect_string_or_list(frontmatter.get("sms"), &mut out, "Sms");
    collect_string_or_list(frontmatter.get("phone-sms"), &mut out, "Sms");

    // Teams
    collect_string_or_list(frontmatter.get("teams"), &mut out, "Teams");
    collect_string_or_list(frontmatter.get("teams-email"), &mut out, "Teams");

    // Deduplicate while preserving insertion order.
    let mut seen = std::collections::HashSet::new();
    out.retain(|a| seen.insert((a.channel.clone(), a.address.clone())));
    out
}

fn collect_string_or_list(value: Option<&Value>, out: &mut Vec<PersonAddress>, channel: &str) {
    match value {
        Some(Value::String(s)) => {
            out.push(PersonAddress {
                channel: channel.to_string(),
                address: s.clone(),
            });
        }
        Some(Value::Sequence(seq)) => {
            for v in seq {
                if let Some(s) = v.as_str() {
                    out.push(PersonAddress {
                        channel: channel.to_string(),
                        address: s.to_string(),
                    });
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(s: &str) -> Value {
        serde_yaml::from_str(s).unwrap()
    }

    #[test]
    fn test_extract_basic_person() {
        let fm = yaml(r#"
type: person
name: "Alix Moreau"
role: "Daughter"
tags: [person, family]
last-contact: "2026-04-12"
"#);
        let person = extract_person("05-People/Alix Moreau.md", &fm).unwrap().unwrap();
        assert_eq!(person.name, "Alix Moreau");
        assert_eq!(person.role.as_deref(), Some("Daughter"));
        assert_eq!(person.tags, vec!["person", "family"]);
        assert_eq!(person.last_contact.as_deref(), Some("2026-04-12"));
    }

    #[test]
    fn test_extract_skips_non_person() {
        let fm = yaml(r#"
type: project
name: "Project X"
"#);
        assert!(extract_person("01-Projects/Project X.md", &fm).unwrap().is_none());
    }

    #[test]
    fn test_extract_skips_missing_name() {
        let fm = yaml(r#"
type: person
role: "Unknown"
"#);
        assert!(extract_person("05-People/Unknown.md", &fm).unwrap().is_none());
    }

    #[test]
    fn test_extract_emails_from_string() {
        let fm = yaml(r#"
name: "Test"
email: "test@example.com"
"#);
        let person = extract_person("p.md", &fm).unwrap().unwrap();
        assert_eq!(person.addresses.len(), 1);
        assert_eq!(person.addresses[0].channel, "Email");
        assert_eq!(person.addresses[0].address, "test@example.com");
    }

    #[test]
    fn test_extract_emails_from_list() {
        let fm = yaml(r#"
name: "Test"
emails:
  - "a@example.com"
  - "b@example.com"
"#);
        let person = extract_person("p.md", &fm).unwrap().unwrap();
        assert_eq!(person.addresses.len(), 2);
    }

    #[test]
    fn test_extract_email_accounts_structure() {
        let fm = yaml(r#"
name: "Jocelyn"
email-accounts:
  - address: "a@gmail.com"
    provider: gmail
  - address: "b@company.com"
    provider: ms365
"#);
        let person = extract_person("p.md", &fm).unwrap().unwrap();
        assert_eq!(person.addresses.len(), 2);
        assert!(person.addresses.iter().all(|a| a.channel == "Email"));
    }

    #[test]
    fn test_extract_multiple_channels() {
        let fm = yaml(r#"
name: "Test"
email: "t@example.com"
telegram: "@testuser"
whatsapp: "+491234567"
"#);
        let person = extract_person("p.md", &fm).unwrap().unwrap();
        let channels: Vec<&str> = person.addresses.iter().map(|a| a.channel.as_str()).collect();
        assert!(channels.contains(&"Email"));
        assert!(channels.contains(&"Telegram"));
        assert!(channels.contains(&"WhatsApp"));
    }

    #[test]
    fn test_duplicates_are_deduplicated() {
        let fm = yaml(r#"
name: "Test"
email: "t@example.com"
emails:
  - "t@example.com"
  - "other@example.com"
"#);
        let person = extract_person("p.md", &fm).unwrap().unwrap();
        assert_eq!(person.addresses.len(), 2);
    }
}
```

- [ ] **Step 2: Update `core/src/knowledge/mod.rs`**

```rust
pub mod embedder;
pub mod parser;
pub mod people;

pub use embedder::{Embedder, EMBEDDING_DIM};
pub use parser::{parse_markdown_file, ParsedFile, Section};
pub use people::{extract_person, PersonAddress, VaultPerson};
```

- [ ] **Step 3: Run tests — 8 new tests should pass**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo test knowledge::people
```

- [ ] **Step 4: Commit**

```bash
git add core/src/knowledge/people.rs core/src/knowledge/mod.rs
git commit -m "feat: add structured people profile extractor

Extracts name, role, tags, and multi-channel addresses from the YAML
frontmatter of 05-People/*.md files. Recognizes several conventions
(email/emails, email-accounts structure, telegram, whatsapp, etc.)
and gracefully skips non-person files via the type field check.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Store Methods for Knowledge Tables

**Files:**
- Create: `core/src/store/knowledge.rs`
- Modify: `core/src/store/mod.rs`

`★ Why this matters:` The indexer needs transactional writes for a file's (a) `vault_files` row, (b) N `vault_chunks` rows, (c) N embedding vectors, (d) optional `vault_people` row. If any step fails, none of the partial rows should persist. We wrap all that in a single method with BEGIN/COMMIT.

- [ ] **Step 1: Add `pub mod knowledge;` to `core/src/store/mod.rs`**

```rust
pub mod channels;
pub mod contacts;
pub mod knowledge;
pub mod messages;
mod migrations;
// ...rest unchanged
```

- [ ] **Step 2: Implement at `core/src/store/knowledge.rs`**

```rust
use rusqlite::params;

use crate::error::{CoreError, Result};
use crate::knowledge::{ParsedFile, VaultPerson};
use crate::store::Store;

/// Everything the indexer needs to persist for a single markdown file.
pub struct IndexedFile<'a> {
    pub path: &'a str,
    pub mtime_secs: i64,
    pub para_folder: Option<&'a str>,
    pub parsed: &'a ParsedFile,
    /// One embedding per section (same order as `parsed.sections`).
    pub chunk_embeddings: &'a [Vec<f32>],
    /// If this is a valid person profile, the extracted structured data.
    pub person: Option<&'a VaultPerson>,
}

impl Store {
    /// Insert or replace a file's knowledge representation transactionally.
    ///
    /// If the file already exists in `vault_files`, all its chunks, vectors,
    /// and person row are deleted and re-inserted. This keeps incremental
    /// updates simple: callers just re-call `upsert_indexed_file`.
    pub fn upsert_indexed_file(&self, file: &IndexedFile<'_>) -> Result<()> {
        if file.chunk_embeddings.len() != file.parsed.sections.len() {
            return Err(CoreError::Knowledge(format!(
                "chunk count mismatch: {} sections, {} embeddings",
                file.parsed.sections.len(),
                file.chunk_embeddings.len()
            )));
        }

        self.conn().execute_batch("BEGIN IMMEDIATE;")?;
        let result = self.upsert_indexed_file_inner(file);
        match &result {
            Ok(_) => self.conn().execute_batch("COMMIT;")?,
            Err(_) => {
                let _ = self.conn().execute_batch("ROLLBACK;");
            }
        }
        result
    }

    fn upsert_indexed_file_inner(&self, file: &IndexedFile<'_>) -> Result<()> {
        // Delete existing chunks + vectors for this file (cascades via FK on vault_chunks).
        // We must delete from vault_chunk_vecs explicitly because it's a virtual table
        // and FK cascades don't reach it.
        let existing_ids: Vec<i64> = {
            let mut stmt = self.conn().prepare(
                "SELECT id FROM vault_chunks WHERE file_path = ?1",
            )?;
            let ids: std::result::Result<Vec<i64>, _> = stmt
                .query_map([file.path], |row| row.get(0))?
                .collect();
            ids.map_err(CoreError::Database)?
        };
        for id in &existing_ids {
            self.conn().execute(
                "DELETE FROM vault_chunk_vecs WHERE rowid = ?1",
                [id],
            )?;
        }

        // Delete the vault_files row — cascades to vault_chunks and vault_people.
        self.conn().execute(
            "DELETE FROM vault_files WHERE path = ?1",
            [file.path],
        )?;

        // Insert vault_files.
        let frontmatter_json = file
            .parsed
            .frontmatter
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        self.conn().execute(
            "INSERT INTO vault_files (path, content_hash, mtime_secs, frontmatter_json, para_folder)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                file.path,
                file.parsed.content_hash,
                file.mtime_secs,
                frontmatter_json,
                file.para_folder,
            ],
        )?;

        // Insert chunks + vectors.
        for (idx, (section, embedding)) in file
            .parsed
            .sections
            .iter()
            .zip(file.chunk_embeddings.iter())
            .enumerate()
        {
            self.conn().execute(
                "INSERT INTO vault_chunks (file_path, section_heading, chunk_index, content, token_count, para_folder)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    file.path,
                    section.heading,
                    idx as i64,
                    section.content,
                    section.tokens as i64,
                    file.para_folder,
                ],
            )?;
            let chunk_id = self.conn().last_insert_rowid();

            let bytes = f32_slice_to_bytes(embedding);
            self.conn().execute(
                "INSERT INTO vault_chunk_vecs (rowid, embedding) VALUES (?1, ?2)",
                params![chunk_id, bytes],
            )?;
        }

        // Insert person row + addresses if present.
        if let Some(person) = file.person {
            let tags_json = serde_json::to_string(&person.tags)?;
            let fm_json = serde_json::to_string(&person.frontmatter)?;
            self.conn().execute(
                "INSERT INTO vault_people (file_path, name, role, tags_json, last_contact, frontmatter_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    person.file_path,
                    person.name,
                    person.role,
                    tags_json,
                    person.last_contact,
                    fm_json,
                ],
            )?;
            let person_id = self.conn().last_insert_rowid();

            for addr in &person.addresses {
                self.conn().execute(
                    "INSERT OR IGNORE INTO vault_people_addresses (person_id, channel_type, address)
                     VALUES (?1, ?2, ?3)",
                    params![person_id, addr.channel, addr.address],
                )?;
            }
        }

        Ok(())
    }

    /// Returns the content hash currently indexed for `path`, or None if the file isn't indexed.
    /// Callers compare this against the current file hash to decide whether to re-index.
    pub fn indexed_content_hash(&self, path: &str) -> Result<Option<String>> {
        let result: std::result::Result<String, rusqlite::Error> = self.conn().query_row(
            "SELECT content_hash FROM vault_files WHERE path = ?1",
            [path],
            |row| row.get(0),
        );
        match result {
            Ok(h) => Ok(Some(h)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Database(e)),
        }
    }

    /// Delete a file's knowledge representation (chunks, vectors, person).
    /// Used when a file is deleted from the vault.
    pub fn delete_indexed_file(&self, path: &str) -> Result<()> {
        let existing_ids: Vec<i64> = {
            let mut stmt = self.conn().prepare(
                "SELECT id FROM vault_chunks WHERE file_path = ?1",
            )?;
            let ids: std::result::Result<Vec<i64>, _> = stmt
                .query_map([path], |row| row.get(0))?
                .collect();
            ids.map_err(CoreError::Database)?
        };
        self.conn().execute_batch("BEGIN IMMEDIATE;")?;
        for id in &existing_ids {
            self.conn().execute(
                "DELETE FROM vault_chunk_vecs WHERE rowid = ?1",
                [id],
            )?;
        }
        self.conn().execute(
            "DELETE FROM vault_files WHERE path = ?1",
            [path],
        )?;
        self.conn().execute_batch("COMMIT;")?;
        Ok(())
    }

    /// Lookup a vault person by an address on a specific channel.
    /// Returns the person's name and file path, or None.
    pub fn find_vault_person_by_address(
        &self,
        channel_db_str: &str,
        address: &str,
    ) -> Result<Option<(String, String)>> {
        let result: std::result::Result<(String, String), rusqlite::Error> = self.conn().query_row(
            "SELECT vp.name, vp.file_path
             FROM vault_people_addresses vpa
             JOIN vault_people vp ON vp.id = vpa.person_id
             WHERE vpa.channel_type = ?1 AND vpa.address = ?2
             LIMIT 1",
            params![channel_db_str, address],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok(pair) => Ok(Some(pair)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Database(e)),
        }
    }
}

/// Convert a slice of f32s into the little-endian byte representation that sqlite-vec expects.
fn f32_slice_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}
```

- [ ] **Step 3: Verify compilation**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo check -p messagehub-core
```

- [ ] **Step 4: Commit**

```bash
git add core/src/store/knowledge.rs core/src/store/mod.rs
git commit -m "feat: add transactional store methods for vault indexing

upsert_indexed_file writes vault_files, vault_chunks, vault_chunk_vecs,
and optional vault_people rows in a single transaction with rollback
on error. Delete-and-reinsert semantics keep incremental updates simple.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Vault Indexer

**Files:**
- Create: `core/src/knowledge/indexer.rs`
- Modify: `core/src/knowledge/mod.rs`
- Create: `core/tests/knowledge_indexer_test.rs`

`★ Why this matters:` The indexer is the orchestrator that glues parser + embedder + people extractor + store together. It walks the vault, detects changes via content hashes, and re-embeds only what changed. This is where the bulk of the user's existing vault (~154 files) gets ingested.

- [ ] **Step 1: Implement at `core/src/knowledge/indexer.rs`**

```rust
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use tracing::{info, warn, debug};

use crate::error::{CoreError, Result};
use crate::knowledge::{
    embedder::Embedder,
    parser::parse_markdown_file,
    people::extract_person,
    ParsedFile,
};
use crate::store::knowledge::IndexedFile;
use crate::store::Store;

/// Indexer: reads files from a vault root, parses them, embeds chunks,
/// and persists everything via Store.
pub struct Indexer {
    vault_root: PathBuf,
    embedder: Arc<Embedder>,
}

/// Summary of an indexing run.
#[derive(Debug, Default)]
pub struct IndexingReport {
    pub files_scanned: usize,
    pub files_indexed: usize,     // newly indexed
    pub files_reindexed: usize,   // content changed
    pub files_skipped: usize,     // unchanged
    pub files_failed: usize,
    pub people_indexed: usize,
}

impl Indexer {
    pub fn new(vault_root: impl Into<PathBuf>, embedder: Arc<Embedder>) -> Self {
        Self {
            vault_root: vault_root.into(),
            embedder,
        }
    }

    /// Index every markdown file under the vault root.
    /// Uses content-hash comparison to skip unchanged files.
    pub fn index_all(&self, store: &Store) -> Result<IndexingReport> {
        let mut report = IndexingReport::default();
        for entry in walk_markdown_files(&self.vault_root) {
            report.files_scanned += 1;
            match self.index_one(&entry, store) {
                Ok(IndexOutcome::Indexed { is_new, is_person }) => {
                    if is_new {
                        report.files_indexed += 1;
                    } else {
                        report.files_reindexed += 1;
                    }
                    if is_person {
                        report.people_indexed += 1;
                    }
                }
                Ok(IndexOutcome::Skipped) => {
                    report.files_skipped += 1;
                }
                Err(e) => {
                    report.files_failed += 1;
                    warn!(path = %entry.display(), error = %e, "failed to index file");
                }
            }
        }
        info!(
            scanned = report.files_scanned,
            indexed = report.files_indexed,
            reindexed = report.files_reindexed,
            skipped = report.files_skipped,
            failed = report.files_failed,
            people = report.people_indexed,
            "indexing complete"
        );
        Ok(report)
    }

    /// Index a single file. Used by the file watcher on change events.
    pub fn index_file(&self, abs_path: &Path, store: &Store) -> Result<IndexOutcome> {
        self.index_one(abs_path, store)
    }

    /// Remove a file's indexed data. Used by the watcher on deletion events.
    pub fn remove_file(&self, abs_path: &Path, store: &Store) -> Result<()> {
        let rel = self.relative_path(abs_path)?;
        store.delete_indexed_file(&rel)
    }

    fn index_one(&self, abs_path: &Path, store: &Store) -> Result<IndexOutcome> {
        let rel_path = self.relative_path(abs_path)?;
        let mtime_secs = file_mtime_secs(abs_path)?;
        let content = std::fs::read_to_string(abs_path)
            .map_err(|e| CoreError::Knowledge(format!("read {}: {}", abs_path.display(), e)))?;
        let parsed = parse_markdown_file(&content)?;

        // Incremental-update gate: if the stored hash matches, skip.
        if let Some(existing_hash) = store.indexed_content_hash(&rel_path)? {
            if existing_hash == parsed.content_hash {
                debug!(path = %rel_path, "unchanged, skipping");
                return Ok(IndexOutcome::Skipped);
            }
        }

        let is_new = store.indexed_content_hash(&rel_path)?.is_none();
        let para_folder = detect_para_folder(&rel_path);

        // Collect chunks (one per section). Skip empty sections.
        let section_texts: Vec<&str> = parsed
            .sections
            .iter()
            .map(|s| s.content.as_str())
            .filter(|c| !c.trim().is_empty())
            .collect();

        let chunk_embeddings = if section_texts.is_empty() {
            Vec::new()
        } else {
            self.embedder.embed_passages(&section_texts)?
        };

        // The parsed.sections list may include empty sections (e.g., a heading
        // with no body). We filter those out for embeddings but need the kept
        // sections passed to IndexedFile to match 1:1 with embeddings.
        let kept_sections: Vec<_> = parsed
            .sections
            .iter()
            .filter(|s| !s.content.trim().is_empty())
            .cloned()
            .collect();
        let kept = ParsedFile {
            sections: kept_sections,
            ..parsed.clone()
        };

        // Extract person info for 05-People/ files.
        let person = if para_folder.as_deref() == Some("05-People") {
            if let Some(fm) = &kept.frontmatter {
                extract_person(&rel_path, fm)?
            } else {
                None
            }
        } else {
            None
        };
        let is_person = person.is_some();

        let indexed = IndexedFile {
            path: &rel_path,
            mtime_secs,
            para_folder: para_folder.as_deref(),
            parsed: &kept,
            chunk_embeddings: &chunk_embeddings,
            person: person.as_ref(),
        };
        store.upsert_indexed_file(&indexed)?;

        Ok(IndexOutcome::Indexed { is_new, is_person })
    }

    fn relative_path(&self, abs_path: &Path) -> Result<String> {
        let rel = abs_path
            .strip_prefix(&self.vault_root)
            .map_err(|_| CoreError::Knowledge(format!(
                "{} is not under vault root {}",
                abs_path.display(),
                self.vault_root.display()
            )))?;
        Ok(rel.to_string_lossy().replace('\\', "/"))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum IndexOutcome {
    Indexed { is_new: bool, is_person: bool },
    Skipped,
}

/// Collect every `.md` file under `root`, skipping hidden directories (e.g. `.obsidian`).
/// Eager collection is fine for typical vaults (hundreds to low thousands of files).
fn walk_markdown_files(root: &Path) -> Vec<PathBuf> {
    use std::collections::VecDeque;
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    let mut out: Vec<PathBuf> = Vec::new();
    if root.is_dir() {
        queue.push_back(root.to_path_buf());
    }
    while let Some(dir) = queue.pop_front() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') {
                    continue;
                }
            }
            if path.is_dir() {
                queue.push_back(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                out.push(path);
            }
        }
    }
    out
}

fn file_mtime_secs(path: &Path) -> Result<i64> {
    let meta = std::fs::metadata(path)
        .map_err(|e| CoreError::Knowledge(format!("stat {}: {}", path.display(), e)))?;
    let mtime = meta
        .modified()
        .map_err(|e| CoreError::Knowledge(format!("mtime: {}", e)))?;
    let secs = mtime
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok(secs)
}

/// Detect the PARA folder by looking at the top-level directory of the relative path.
/// Only returns Some(...) for the known PARA folders.
fn detect_para_folder(rel_path: &str) -> Option<String> {
    const PARA_FOLDERS: &[&str] = &[
        "00-Inbox",
        "01-Projects",
        "02-Areas",
        "03-Resources",
        "04-Archive",
        "05-People",
        "06-Meetings",
        "07-Daily",
    ];
    let first = rel_path.split('/').next()?;
    if PARA_FOLDERS.iter().any(|p| *p == first) {
        Some(first.to_string())
    } else {
        None
    }
}
```

- [ ] **Step 2: Update `core/src/knowledge/mod.rs`**

```rust
pub mod embedder;
pub mod indexer;
pub mod parser;
pub mod people;

pub use embedder::{Embedder, EMBEDDING_DIM};
pub use indexer::{Indexer, IndexingReport, IndexOutcome};
pub use parser::{parse_markdown_file, ParsedFile, Section};
pub use people::{extract_person, PersonAddress, VaultPerson};
```

- [ ] **Step 3: Write integration tests at `core/tests/knowledge_indexer_test.rs`**

These tests need the embedder, which downloads the model on first run. We mark them `#[ignore]` by default so CI stays fast.

```rust
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

    write(vault.path(), "05-People/Alice.md", r#"---
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
"#);

    write(vault.path(), "01-Projects/Dashboard.md", r#"---
type: project
name: "Dashboard Redesign"
status: active
---

# Dashboard Redesign

## Goal
Simplify the main navigation.

## Status
In design review with Alice.
"#);

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
    write(vault.path(), "05-People/Alice.md", r#"---
name: "Alice"
email: "alice@example.com"
telegram: "@alice_dev"
---

# Alice
## Notes
A test person.
"#);

    let store = Store::open_in_memory().unwrap();
    let embedder = Arc::new(Embedder::new().unwrap());
    Indexer::new(vault.path(), embedder).index_all(&store).unwrap();

    let via_email = store.find_vault_person_by_address("Email", "alice@example.com").unwrap();
    assert!(via_email.is_some());
    assert_eq!(via_email.unwrap().0, "Alice");

    let via_telegram = store.find_vault_person_by_address("Telegram", "@alice_dev").unwrap();
    assert!(via_telegram.is_some());

    let not_found = store.find_vault_person_by_address("Email", "nobody@example.com").unwrap();
    assert!(not_found.is_none());
}
```

- [ ] **Step 4: Run the non-ignored tests (compile check + parser tests)**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo test -p messagehub-core
```

- [ ] **Step 5: Commit**

```bash
git add core/src/knowledge/indexer.rs core/src/knowledge/mod.rs core/tests/knowledge_indexer_test.rs
git commit -m "feat: add vault indexer with incremental updates

Walks the vault root, parses markdown files, embeds chunks via
fastembed, and upserts via Store::upsert_indexed_file. Uses
blake3 content hashes to skip unchanged files on re-runs.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: Semantic Retrieval

**Files:**
- Create: `core/src/knowledge/retrieval.rs`
- Modify: `core/src/knowledge/mod.rs`
- Create: `core/tests/knowledge_retrieval_test.rs`

`★ Why this matters:` Retrieval is where the knowledge engine pays off. The query "who is Alix?" should find the `05-People/Alix Moreau.md` chunks. "Emails from the school about Alix" should find both the person profile and the archived school emails. This task implements the filtered vector search that the AI pipeline (Plan 4) will call.

- [ ] **Step 1: Implement at `core/src/knowledge/retrieval.rs`**

```rust
use rusqlite::params;
use std::sync::Arc;

use crate::error::{CoreError, Result};
use crate::knowledge::Embedder;
use crate::store::Store;

/// A retrieved chunk with similarity score and provenance.
#[derive(Debug, Clone)]
pub struct RetrievedChunk {
    pub file_path: String,
    pub section_heading: Option<String>,
    pub content: String,
    pub para_folder: Option<String>,
    /// L2 distance — lower is more similar.
    pub distance: f32,
}

/// Optional filters for retrieval.
#[derive(Debug, Clone, Default)]
pub struct RetrievalFilters {
    /// Restrict to specific PARA folders (e.g. ["05-People"]).
    pub para_folders: Option<Vec<String>>,
    /// Maximum chunks to return (default 5 if None).
    pub top_k: Option<usize>,
}

pub struct Retriever {
    embedder: Arc<Embedder>,
}

impl Retriever {
    pub fn new(embedder: Arc<Embedder>) -> Self {
        Self { embedder }
    }

    /// Semantic search over indexed vault chunks.
    pub fn search(
        &self,
        store: &Store,
        query: &str,
        filters: &RetrievalFilters,
    ) -> Result<Vec<RetrievedChunk>> {
        let query_vec = self.embedder.embed_query(query)?;
        let query_bytes = f32_slice_to_bytes(&query_vec);
        let top_k = filters.top_k.unwrap_or(5);

        // We fetch top_k * 4 from the vec index then filter by para_folder,
        // which is a reasonable tradeoff (sqlite-vec's MATCH doesn't support
        // our external filter). If the user has few chunks in the filtered
        // folder, `top_k * 4` will still return enough; if they have many,
        // the caller can always raise top_k.
        let over_fetch = top_k.saturating_mul(4).max(20);

        let para_filter_clause = filters
            .para_folders
            .as_ref()
            .map(|_| " AND vc.para_folder IN (SELECT value FROM json_each(?3))")
            .unwrap_or("");

        let sql = format!(
            "SELECT vc.file_path, vc.section_heading, vc.content, vc.para_folder, v.distance
             FROM vault_chunk_vecs v
             JOIN vault_chunks vc ON vc.id = v.rowid
             WHERE v.embedding MATCH ?1 AND k = ?2{}
             ORDER BY v.distance",
            para_filter_clause
        );

        let mut stmt = store.conn().prepare(&sql)?;
        let rows: Vec<RetrievedChunk> = match &filters.para_folders {
            Some(folders) => {
                let folders_json = serde_json::to_string(folders)?;
                let result: std::result::Result<Vec<RetrievedChunk>, rusqlite::Error> = stmt
                    .query_map(
                        params![query_bytes, over_fetch as i64, folders_json],
                        row_to_chunk,
                    )?
                    .collect();
                result.map_err(CoreError::Database)?
            }
            None => {
                let result: std::result::Result<Vec<RetrievedChunk>, rusqlite::Error> = stmt
                    .query_map(params![query_bytes, over_fetch as i64], row_to_chunk)?
                    .collect();
                result.map_err(CoreError::Database)?
            }
        };

        Ok(rows.into_iter().take(top_k).collect())
    }
}

fn row_to_chunk(row: &rusqlite::Row) -> std::result::Result<RetrievedChunk, rusqlite::Error> {
    Ok(RetrievedChunk {
        file_path: row.get(0)?,
        section_heading: row.get(1)?,
        content: row.get(2)?,
        para_folder: row.get(3)?,
        distance: row.get::<_, f64>(4)? as f32,
    })
}

fn f32_slice_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}
```

- [ ] **Step 2: Update `core/src/knowledge/mod.rs`**

```rust
pub mod embedder;
pub mod indexer;
pub mod parser;
pub mod people;
pub mod retrieval;

pub use embedder::{Embedder, EMBEDDING_DIM};
pub use indexer::{Indexer, IndexingReport, IndexOutcome};
pub use parser::{parse_markdown_file, ParsedFile, Section};
pub use people::{extract_person, PersonAddress, VaultPerson};
pub use retrieval::{RetrievalFilters, RetrievedChunk, Retriever};
```

- [ ] **Step 3: Integration test at `core/tests/knowledge_retrieval_test.rs`**

```rust
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
    write(vault.path(), "05-People/Alice.md", "---\nname: Alice\n---\n# Alice\n## Role\nAlice is a software engineer focused on AI systems.\n");
    write(vault.path(), "05-People/Bob.md", "---\nname: Bob\n---\n# Bob\n## Role\nBob is a chef who specializes in Italian cuisine.\n");
    write(vault.path(), "01-Projects/ai-feature.md", "---\nname: AI Feature\n---\n# AI Feature\n## Goal\nBuild an AI-powered assistant.\n");

    let store = Store::open_in_memory().unwrap();
    let embedder = Arc::new(Embedder::new().unwrap());
    Indexer::new(vault.path(), embedder.clone()).index_all(&store).unwrap();

    let retriever = Retriever::new(embedder);
    let results = retriever
        .search(&store, "who works on machine learning?", &RetrievalFilters::default())
        .unwrap();
    assert!(!results.is_empty());
    // Alice's chunk should be among the top results (AI/software engineering is closer to ML than cooking).
    let top_paths: Vec<&str> = results.iter().map(|r| r.file_path.as_str()).collect();
    assert!(top_paths.iter().any(|p| p.contains("Alice")),
        "expected Alice among top results, got {:?}", top_paths);
}

#[test]
#[ignore = "requires model download — ~120MB"]
fn test_para_folder_filter() {
    let vault = TempDir::new().unwrap();
    write(vault.path(), "05-People/Alice.md", "---\nname: Alice\n---\n# Alice\n## Role\nSoftware engineer.\n");
    write(vault.path(), "01-Projects/Software.md", "---\nname: Software\n---\n# Software\n## Notes\nSoftware engineering guidelines.\n");

    let store = Store::open_in_memory().unwrap();
    let embedder = Arc::new(Embedder::new().unwrap());
    Indexer::new(vault.path(), embedder.clone()).index_all(&store).unwrap();

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
```

- [ ] **Step 4: Verify compilation**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo check -p messagehub-core
```

- [ ] **Step 5: Commit**

```bash
git add core/src/knowledge/retrieval.rs core/src/knowledge/mod.rs core/tests/knowledge_retrieval_test.rs
git commit -m "feat: add semantic retrieval with PARA folder filters

Retriever embeds the query with the 'query:' prefix, performs vec0
MATCH against vault_chunk_vecs, joins with vault_chunks for provenance,
and optionally filters by PARA folder. Returns top-k chunks ordered
by L2 distance.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: Optional File Watcher

**Files:**
- Create: `core/src/knowledge/watcher.rs`
- Modify: `core/src/knowledge/mod.rs`

`★ Why this matters:` This is the "incremental updates" feature from the spec — when the user adds a note to the Inbox in Obsidian, MessageHub should re-index that file without waiting for a full re-scan. We use `notify` to watch the vault root and dispatch to the indexer.

- [ ] **Step 1: Implement at `core/src/knowledge/watcher.rs`**

```rust
use std::path::{Path, PathBuf};
use std::sync::Arc;

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::error::{CoreError, Result};
use crate::knowledge::Indexer;
use crate::store::Store;

/// A running vault watcher. Dropping the watcher stops it.
pub struct VaultWatcher {
    _watcher: RecommendedWatcher,
    _task: tokio::task::JoinHandle<()>,
}

impl VaultWatcher {
    /// Start watching `vault_root`. On every create/modify/delete of a markdown file,
    /// the indexer is invoked (with the shared Store).
    ///
    /// The watcher runs until the returned `VaultWatcher` is dropped.
    pub fn start(
        vault_root: impl AsRef<Path>,
        indexer: Arc<Indexer>,
        store: Arc<Store>,
    ) -> Result<Self> {
        let vault_root = vault_root.as_ref().to_path_buf();
        let (tx, mut rx) = mpsc::unbounded_channel::<Event>();

        let mut watcher: RecommendedWatcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| match res {
                Ok(event) => {
                    let _ = tx.send(event);
                }
                Err(e) => warn!(error = %e, "watcher error"),
            },
            Config::default(),
        )
        .map_err(|e| CoreError::Knowledge(format!("watcher init failed: {}", e)))?;

        watcher
            .watch(&vault_root, RecursiveMode::Recursive)
            .map_err(|e| CoreError::Knowledge(format!("watch {} failed: {}", vault_root.display(), e)))?;

        let task = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if !event_is_markdown(&event) {
                    continue;
                }
                for path in &event.paths {
                    if !is_markdown(path) {
                        continue;
                    }
                    match event.kind {
                        EventKind::Create(_) | EventKind::Modify(_) => {
                            match indexer.index_file(path, &store) {
                                Ok(_) => info!(path = %path.display(), "indexed (watch)"),
                                Err(e) => warn!(path = %path.display(), error = %e, "index failed (watch)"),
                            }
                        }
                        EventKind::Remove(_) => {
                            match indexer.remove_file(path, &store) {
                                Ok(_) => info!(path = %path.display(), "removed (watch)"),
                                Err(e) => warn!(path = %path.display(), error = %e, "remove failed (watch)"),
                            }
                        }
                        _ => {}
                    }
                }
            }
        });

        Ok(Self {
            _watcher: watcher,
            _task: task,
        })
    }
}

fn event_is_markdown(event: &Event) -> bool {
    event.paths.iter().any(|p| is_markdown(p))
}

fn is_markdown(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("md")
}

#[allow(dead_code)]
fn paths_as_vec(event: &Event) -> Vec<PathBuf> {
    event.paths.clone()
}
```

- [ ] **Step 2: Update `core/src/knowledge/mod.rs`**

```rust
pub mod embedder;
pub mod indexer;
pub mod parser;
pub mod people;
pub mod retrieval;
pub mod watcher;

pub use embedder::{Embedder, EMBEDDING_DIM};
pub use indexer::{Indexer, IndexingReport, IndexOutcome};
pub use parser::{parse_markdown_file, ParsedFile, Section};
pub use people::{extract_person, PersonAddress, VaultPerson};
pub use retrieval::{RetrievalFilters, RetrievedChunk, Retriever};
pub use watcher::VaultWatcher;
```

- [ ] **Step 3: Verify compilation**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo check -p messagehub-core
```

- [ ] **Step 4: Commit**

```bash
git add core/src/knowledge/watcher.rs core/src/knowledge/mod.rs
git commit -m "feat: add vault file watcher with notify

VaultWatcher wraps notify's RecommendedWatcher, filters to markdown
files, and dispatches create/modify events to Indexer::index_file
and delete events to Indexer::remove_file. Runs on a tokio task
until dropped.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

## Summary

After completing all 8 tasks, you have:

- **`vault_files` / `vault_chunks` / `vault_chunk_vecs` / `vault_people` tables** with sqlite-vec virtual table for 384-dim vectors
- **Markdown parser** with YAML frontmatter support, H1/H2 section splitting, and content hashing
- **Fastembed wrapper** enforcing the E5 `passage:` / `query:` prefix convention for multilingual embeddings
- **People extractor** parsing structured contact data from 05-People/*.md frontmatter
- **Transactional Store methods** for upserting indexed files and looking up people by address
- **Indexer** with full-vault scan and incremental updates via blake3 hashing
- **Retriever** with top-k vector search and PARA folder filtering
- **File watcher** for real-time re-indexing on vault changes
- **~15 new tests** covering parser (unit), people extraction (unit), indexer (integration), and retrieval (integration)

**Next plan:** Plan 4 (AI Pipeline) will use `Retriever` to pull vault context into prompts for local classification (priority/category) and cloud RAG actions (summarize, draft reply, smart search).
