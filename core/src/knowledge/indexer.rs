use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use tracing::{debug, info, warn};

use crate::error::{CoreError, Result};
use crate::knowledge::{
    ParsedFile, embedder::Embedder, parser::parse_markdown_file, people::extract_person,
};
use crate::store::Store;
use crate::store::knowledge::IndexedFile;

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
    pub files_indexed: usize,   // newly indexed
    pub files_reindexed: usize, // content changed
    pub files_skipped: usize,   // unchanged
    pub files_failed: usize,
    pub people_indexed: usize,
}

#[derive(Debug, Clone, Copy)]
pub enum IndexOutcome {
    Indexed { is_new: bool, is_person: bool },
    Skipped,
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
        let existing_hash = store.indexed_content_hash(&rel_path)?;
        if let Some(hash) = &existing_hash {
            if hash == &parsed.content_hash {
                debug!(path = %rel_path, "unchanged, skipping");
                return Ok(IndexOutcome::Skipped);
            }
        }

        let is_new = existing_hash.is_none();
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
        let rel = abs_path.strip_prefix(&self.vault_root).map_err(|_| {
            CoreError::Knowledge(format!(
                "{} is not under vault root {}",
                abs_path.display(),
                self.vault_root.display()
            ))
        })?;
        Ok(rel.to_string_lossy().replace('\\', "/"))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_para_folder_recognizes_known() {
        assert_eq!(
            detect_para_folder("05-People/Alice.md"),
            Some("05-People".to_string())
        );
        assert_eq!(
            detect_para_folder("01-Projects/X/notes.md"),
            Some("01-Projects".to_string())
        );
        assert_eq!(
            detect_para_folder("07-Daily/2026-04-17.md"),
            Some("07-Daily".to_string())
        );
    }

    #[test]
    fn test_detect_para_folder_unknown_returns_none() {
        assert_eq!(detect_para_folder("notes.md"), None);
        assert_eq!(detect_para_folder("Random/file.md"), None);
        assert_eq!(detect_para_folder("08-Other/x.md"), None);
    }

    #[test]
    fn test_walk_markdown_files_skips_hidden_and_non_md() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();

        std::fs::create_dir_all(root.join("01-Projects")).unwrap();
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        std::fs::write(root.join("01-Projects/a.md"), "# a").unwrap();
        std::fs::write(root.join(".obsidian/workspace.json"), "{}").unwrap();
        std::fs::write(root.join("readme.txt"), "hi").unwrap();
        std::fs::write(root.join("b.md"), "# b").unwrap();

        let found = walk_markdown_files(root);
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|p| p.ends_with("a.md")));
        assert!(found.iter().any(|p| p.ends_with("b.md")));
    }
}
