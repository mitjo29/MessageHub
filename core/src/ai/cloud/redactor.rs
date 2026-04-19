use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::error::Result;
use crate::store::Store;

/// Map from token (e.g. `"[PERSON_1]"`) back to the original verbatim
/// string that was scrubbed. `un_redact` applies it to restore
/// user-visible output.
pub type ReverseMap = HashMap<String, String>;

/// Longest-match-first entity scrubber.
///
/// Three classes, applied in order:
/// 1. Vault-matched names (from `05-People/*.md` via `Store::list_vault_people`).
///    Loaded at construction; no mid-session refresh in Plan 5.
/// 2. Email addresses (regex).
/// 3. Phone numbers (regex, min 9 chars to avoid order numbers / SKUs).
///
/// Same original → same token across one `redact` call (stable numbering
/// within the call). Different calls get fresh maps.
pub struct Redactor {
    /// Per-name compiled regex (case-insensitive, longest-first).
    name_regexes: Vec<(String, Regex)>,
}

static EMAIL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\w.+-]+@[\w-]+\.[\w.-]+").expect("email regex must compile")
});

static PHONE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // At least 9 total characters (including separators) with 9+ digits.
    Regex::new(r"\+?\d[\d\s\-().]{7,}\d").expect("phone regex must compile")
});

impl Redactor {
    /// Load vault people from the store and build a redactor.
    ///
    /// If the vault is empty or the query fails, returns a redactor
    /// that still scrubs emails and phone numbers (regex-only).
    pub fn build(store: &Store) -> Result<Self> {
        let names = match store.list_vault_people() {
            Ok(people) => people.into_iter().map(|p| p.name).collect(),
            Err(_) => Vec::new(),
        };
        Ok(Self::from_names(names))
    }

    /// Construct from an explicit name list. Public so tests can build a
    /// redactor without seeding a vault.
    pub fn from_names(mut names: Vec<String>) -> Self {
        // Longest-first so "Alice Example" wins over "Alice" in a
        // greedy forward scan.
        names.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()));
        let name_regexes = names
            .into_iter()
            .filter_map(|n| {
                Regex::new(&format!("(?i){}", regex::escape(&n)))
                    .ok()
                    .map(|re| (n, re))
            })
            .collect();
        Self { name_regexes }
    }

    /// Redact `input` and return `(redacted, reverse_map)`.
    ///
    /// Token numbering is per-call and per-class — a fresh counter for
    /// PERSON, EMAIL, and PHONE each time. Identical originals within
    /// one call share a token.
    pub fn redact(&self, input: &str) -> (String, ReverseMap) {
        let mut map: ReverseMap = HashMap::new();
        let mut current = input.to_string();
        let mut forward: HashMap<String, String> = HashMap::new();
        let mut counters = RedactCounters::default();

        // 1. Vault names (longest first, case-insensitive via compiled regex).
        for (_name, re) in &self.name_regexes {
            current = replace_regex(&current, re, |m| {
                assign_token(m, "PERSON", &mut counters.person, &mut forward, &mut map)
            });
        }

        // 2. Emails.
        current = replace_regex(&current, &EMAIL_RE, |m| {
            assign_token(m, "EMAIL", &mut counters.email, &mut forward, &mut map)
        });

        // 3. Phones.
        current = replace_regex(&current, &PHONE_RE, |m| {
            assign_token(m, "PHONE", &mut counters.phone, &mut forward, &mut map)
        });

        (current, map)
    }
}

/// Restore original strings from a redacted output using the `ReverseMap`
/// produced by `Redactor::redact`. Tokens not in the map pass through unchanged.
pub fn un_redact(text: &str, map: &ReverseMap) -> String {
    // Sort keys by length descending so tokens never partially
    // replace each other (e.g. `[PERSON_1]` vs `[PERSON_10]`).
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by(|a, b| b.len().cmp(&a.len()));
    let mut out = text.to_string();
    for k in keys {
        out = out.replace(k, map.get(k).unwrap());
    }
    out
}

#[derive(Default)]
struct RedactCounters {
    person: u32,
    email: u32,
    phone: u32,
}

/// Assign (or reuse) a token for `original` inside one class.
fn assign_token(
    original: &str,
    prefix: &str,
    counter: &mut u32,
    forward: &mut HashMap<String, String>,
    map: &mut ReverseMap,
) -> String {
    // Forward lookup key includes the class so "Alice" as PERSON and
    // "Alice" as some-other-class wouldn't collide (not a concern today,
    // defensive).
    let fwd_key = format!("{}:{}", prefix, original);
    if let Some(token) = forward.get(&fwd_key) {
        return token.clone();
    }
    *counter += 1;
    let token = format!("[{}_{}]", prefix, counter);
    forward.insert(fwd_key, token.clone());
    map.insert(token.clone(), original.to_string());
    token
}

/// Regex find-and-replace with a custom token-producer.
fn replace_regex(haystack: &str, re: &Regex, mut producer: impl FnMut(&str) -> String) -> String {
    let mut out = String::with_capacity(haystack.len());
    let mut cursor = 0;
    for m in re.find_iter(haystack) {
        out.push_str(&haystack[cursor..m.start()]);
        out.push_str(&producer(m.as_str()));
        cursor = m.end();
    }
    out.push_str(&haystack[cursor..]);
    out
}
