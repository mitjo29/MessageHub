//! Deterministic channel-id derivation from `(kind, label)`.
//!
//! The runtime's `channels` table uses `Uuid` primary keys. Both
//! `runtime-demo` and the desktop host need to map a TOML
//! `[[channels]]` entry to the same id on every run — otherwise
//! replies compose against the wrong credentials, and re-runs create
//! duplicate rows. A UUIDv5 under `NAMESPACE_OID` gives us that
//! mapping for free.
//!
//! This function must not change. Any byte-level drift in the seed
//! format breaks every persisted row that was keyed off the old
//! hash. Historic format: `"{kind}:{label}"` (no trimming, no
//! lowercasing).

use uuid::Uuid;

/// Returns the UUIDv5 channel id for the given TOML `(kind, label)`
/// pair. Stable across processes and runs.
pub fn stable_channel_id(kind: &str, label: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("{}:{}", kind, label).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_input_same_id() {
        let a = stable_channel_id("email", "Personal Gmail");
        let b = stable_channel_id("email", "Personal Gmail");
        assert_eq!(a, b);
    }

    #[test]
    fn different_kind_different_id() {
        let a = stable_channel_id("email", "x");
        let b = stable_channel_id("telegram", "x");
        assert_ne!(a, b);
    }

    #[test]
    fn different_label_different_id() {
        let a = stable_channel_id("email", "a");
        let b = stable_channel_id("email", "b");
        assert_ne!(a, b);
    }

    /// Pinned output to catch accidental format changes (e.g. adding
    /// a separator, trimming, or lowercasing). If this test fails,
    /// every persisted channel row from before the change is orphaned.
    #[test]
    fn format_pinned() {
        let got = stable_channel_id("email", "Personal Gmail");
        assert_eq!(got.to_string(), "232ba225-0606-595e-8eee-3a15949591bd");
    }
}
