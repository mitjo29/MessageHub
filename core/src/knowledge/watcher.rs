use std::path::Path;
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
    /// the indexer is invoked.
    ///
    /// The watcher takes ownership of its own [`Store`] because `rusqlite::Connection`
    /// (the inner type) is `Send` but not `Sync`, so it cannot be shared behind an
    /// `Arc` across threads. Callers that also need to query the database from the
    /// main thread should open a separate `Store` there — SQLite handles concurrent
    /// connections to the same database file natively.
    ///
    /// The watcher runs until the returned `VaultWatcher` is dropped.
    pub fn start(
        vault_root: impl AsRef<Path>,
        indexer: Arc<Indexer>,
        store: Store,
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
            .map_err(|e| {
                CoreError::Knowledge(format!(
                    "watch {} failed: {}",
                    vault_root.display(),
                    e
                ))
            })?;

        // `Store` wraps a `rusqlite::Connection` (contains a `RefCell` stmt cache),
        // so it is `Send` but not `Sync`. Tokio's default `spawn` requires `Send`,
        // which in turn bans non-`Sync` types captured as `&T` across `.await`
        // points. The indexer work is synchronous (file I/O + SQLite writes)
        // anyway, so we run the drain loop on a dedicated blocking thread via
        // `spawn_blocking` and use `blocking_recv` on the unbounded channel.
        let task = tokio::task::spawn_blocking(move || {
            while let Some(event) = rx.blocking_recv() {
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
                                Err(e) => {
                                    warn!(path = %path.display(), error = %e, "index failed (watch)")
                                }
                            }
                        }
                        EventKind::Remove(_) => match indexer.remove_file(path, &store) {
                            Ok(_) => info!(path = %path.display(), "removed (watch)"),
                            Err(e) => {
                                warn!(path = %path.display(), error = %e, "remove failed (watch)")
                            }
                        },
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
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn is_markdown_matches_md_extension() {
        assert!(is_markdown(&PathBuf::from("note.md")));
        assert!(is_markdown(&PathBuf::from("/vault/01-Projects/a.md")));
    }

    #[test]
    fn is_markdown_is_case_insensitive() {
        assert!(is_markdown(&PathBuf::from("NOTE.MD")));
        assert!(is_markdown(&PathBuf::from("note.Md")));
    }

    #[test]
    fn is_markdown_rejects_non_md() {
        assert!(!is_markdown(&PathBuf::from("note.txt")));
        assert!(!is_markdown(&PathBuf::from("README")));
        assert!(!is_markdown(&PathBuf::from("archive.md.bak")));
        assert!(!is_markdown(&PathBuf::from("")));
    }

    #[test]
    fn event_is_markdown_requires_at_least_one_md_path() {
        let mut event = Event::new(EventKind::Modify(notify::event::ModifyKind::Any));
        event.paths.push(PathBuf::from("note.txt"));
        assert!(!event_is_markdown(&event));
        event.paths.push(PathBuf::from("note.md"));
        assert!(event_is_markdown(&event));
    }
}
