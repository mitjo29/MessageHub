# Plan 4: Local AI Classification (Tier 1) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the always-on, fully-offline classification tier that scores incoming messages with a 1-5 priority and a PARA-derived category using a local LLM (Ollama). Every classification decision is persisted to `action_log` with reasoning for auditability.

**Architecture:** A new `ai` module in `core/src/ai/` with five components: (1) `llm` — a `LlmBackend` trait and an `OllamaLlm` HTTP client implementation (keeps the FFI surface out of Rust entirely, making the backend fully mockable), (2) `profile` — loads the user's vault `user-profile.md` and caches it, (3) `rag` — assembles the per-message RAG context by combining sender lookup (`Store::find_vault_person_by_address`), top-k vault retrieval (`knowledge::Retriever`), and the user profile, (4) `prompts` — builds the single-shot classification prompt and parses a strict JSON response (priority + category + reasoning), (5) `pipeline` — an `AiPipeline` orchestrator that takes a `Message`, runs the classifier, attaches `priority` and `category`, persists via `Store::insert_message`, and logs the decision to `action_log`. Graceful degradation: if the LLM is unreachable or returns malformed output, the message is stored with `priority = None` and a failure row is written to the log.

**Scope boundary (Tier 2 deferred to Plan 5):** Cloud actions (summarize thread, draft reply, smart search via Claude API) are NOT in this plan. They will be built on top of the same `rag` module in Plan 5, opt-in per user action, with entity redaction.

**Tech Stack:** `reqwest` (HTTP client — already a dep), `serde_json` (JSON prompt responses — already a dep), `async-trait` (trait with async methods — already a dep), `wiremock` (NEW dev-dep — HTTP mocking for Ollama client tests). No new runtime dependencies.

**Prerequisites:** Ollama must be running locally with a suitable model pulled (e.g. `ollama pull phi3:mini`). The app detects availability via a health check on startup. If Ollama is not running, the app still functions — messages are just stored without classification.

---

## File Structure

```
core/
├── Cargo.toml                           # MODIFY — add wiremock dev-dep
├── migrations/
│   └── 003_ai.sql                       # CREATE — index action_log for per-entity lookup
├── src/
│   ├── lib.rs                           # MODIFY — add `pub mod ai;`
│   ├── error.rs                         # MODIFY — add Ai variant
│   ├── store/
│   │   ├── mod.rs                        # unchanged
│   │   ├── migrations.rs                 # MODIFY — register 003
│   │   └── ai_log.rs                     # CREATE — log_ai_decision(), list_ai_decisions_for_entity()
│   └── ai/
│       ├── mod.rs                        # CREATE — module root + Category enum + re-exports
│       ├── llm.rs                        # CREATE — LlmBackend trait + OllamaLlm HTTP client
│       ├── profile.rs                    # CREATE — UserProfile loader
│       ├── rag.rs                        # CREATE — RagContext builder (sender + retrieval + profile)
│       ├── prompts.rs                    # CREATE — classification prompt template + JSON parser
│       ├── classifier.rs                 # CREATE — Classifier<L: LlmBackend>
│       └── pipeline.rs                   # CREATE — AiPipeline orchestrator
└── tests/
    ├── ai_prompts_test.rs               # CREATE — parser unit tests
    ├── ai_classifier_test.rs            # CREATE — classifier with mock LLM
    └── ai_pipeline_test.rs              # CREATE — end-to-end pipeline test
```

---

### Task 1: Error Variant + Module Skeleton

**Files:**
- Modify: `core/src/error.rs`
- Modify: `core/src/lib.rs`
- Create: `core/src/ai/mod.rs`

- [x] **Step 1: Add the `Ai` error variant**

Edit `core/src/error.rs`. Add one new variant to the `CoreError` enum, right after `Knowledge`:

```rust
    #[error("ai pipeline error: {0}")]
    Ai(String),
```

The final enum should end with:

```rust
    #[error("knowledge engine error: {0}")]
    Knowledge(String),

    #[error("ai pipeline error: {0}")]
    Ai(String),
}
```

- [x] **Step 2: Register the `ai` module**

Edit `core/src/lib.rs`. Append one line:

```rust
pub mod error;
pub mod types;
pub mod store;
pub mod adapters;
pub mod knowledge;
pub mod ai;
```

- [x] **Step 3: Create the AI module root with the `Category` enum**

Create `core/src/ai/mod.rs`:

```rust
//! Local AI classification pipeline (Tier 1).
//!
//! This module provides always-on, fully-offline priority scoring and
//! category tagging for incoming messages using a local LLM via Ollama.
//!
//! Architecture:
//! - `llm` — `LlmBackend` trait + `OllamaLlm` HTTP client
//! - `profile` — user profile loader (`user-profile.md`)
//! - `rag` — per-message RAG context builder
//! - `prompts` — classification prompt template + JSON response parser
//! - `classifier` — ties the above into a single `classify()` call
//! - `pipeline` — orchestrator: classify → store → log

pub mod classifier;
pub mod llm;
pub mod pipeline;
pub mod profile;
pub mod prompts;
pub mod rag;

pub use classifier::{Classification, Classifier};
pub use llm::{LlmBackend, OllamaLlm};
pub use pipeline::AiPipeline;
pub use profile::UserProfile;
pub use rag::RagContext;

use serde::{Deserialize, Serialize};

/// High-level category derived from the user's PARA vault structure.
///
/// These are the only values the classifier is allowed to emit.
/// The parser in `prompts::parse_classification_response` rejects
/// anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Work,
    Personal,
    Finance,
    Family,
    Notifications,
    Newsletters,
    Spam,
}

impl Category {
    pub fn as_str(&self) -> &'static str {
        match self {
            Category::Work => "work",
            Category::Personal => "personal",
            Category::Finance => "finance",
            Category::Family => "family",
            Category::Notifications => "notifications",
            Category::Newsletters => "newsletters",
            Category::Spam => "spam",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "work" => Some(Category::Work),
            "personal" => Some(Category::Personal),
            "finance" => Some(Category::Finance),
            "family" => Some(Category::Family),
            "notifications" => Some(Category::Notifications),
            "newsletters" => Some(Category::Newsletters),
            "spam" => Some(Category::Spam),
            _ => None,
        }
    }

    /// All valid category strings, used for prompt template injection.
    pub fn all_strs() -> &'static [&'static str] {
        &[
            "work",
            "personal",
            "finance",
            "family",
            "notifications",
            "newsletters",
            "spam",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::Category;

    #[test]
    fn test_category_roundtrip() {
        for s in Category::all_strs() {
            let cat = Category::from_str(s).unwrap();
            assert_eq!(cat.as_str(), *s);
        }
    }

    #[test]
    fn test_category_case_insensitive() {
        assert_eq!(Category::from_str("WORK").unwrap(), Category::Work);
        assert_eq!(Category::from_str("  Spam  ").unwrap(), Category::Spam);
    }

    #[test]
    fn test_category_rejects_unknown() {
        assert!(Category::from_str("important").is_none());
        assert!(Category::from_str("").is_none());
    }
}
```

- [x] **Step 4: Create stub files so `mod.rs` compiles**

The module root above references five sub-modules. Create minimal stubs; each will be filled in by later tasks.

Create `core/src/ai/llm.rs`:

```rust
//! Stub — filled in by Task 4.

use async_trait::async_trait;

use crate::error::Result;

#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn complete(&self, system: &str, user: &str, max_tokens: u32) -> Result<String>;
}

pub struct OllamaLlm;
```

Create `core/src/ai/profile.rs`:

```rust
//! Stub — filled in by Task 5.

pub struct UserProfile {
    pub content: String,
}
```

Create `core/src/ai/rag.rs`:

```rust
//! Stub — filled in by Task 6.

pub struct RagContext;
```

Create `core/src/ai/prompts.rs`:

```rust
//! Stub — filled in by Task 7.
```

Create `core/src/ai/classifier.rs`:

```rust
//! Stub — filled in by Task 8.

use crate::ai::Category;
use crate::types::PriorityScore;

#[derive(Debug, Clone)]
pub struct Classification {
    pub priority: PriorityScore,
    pub category: Category,
    pub reasoning: String,
}

pub struct Classifier;
```

Create `core/src/ai/pipeline.rs`:

```rust
//! Stub — filled in by Task 9.

pub struct AiPipeline;
```

- [x] **Step 5: Verify everything compiles**

Run: `cargo check -p messagehub-core`
Expected: PASS (warnings about unused items are fine; there should be no errors).

- [x] **Step 6: Run the Category unit tests**

Run: `cargo test -p messagehub-core ai::tests::test_category -- --nocapture`
Expected: 3 passing tests (`test_category_roundtrip`, `test_category_case_insensitive`, `test_category_rejects_unknown`).

- [x] **Step 7: Commit**

```bash
git add core/src/error.rs core/src/lib.rs core/src/ai/
git commit -m "feat(ai): scaffold ai module with Category enum and stubs"
```

---

### Task 2: Migration 003 — Index action_log

**Files:**
- Create: `core/migrations/003_ai.sql`
- Modify: `core/src/store/migrations.rs`

`★ Why this matters:` Every classification writes a row to `action_log` with `entity_type = "message"` and `entity_id = <message_uuid>`. The "Why is this prioritized?" UI feature will query `WHERE entity_type = ? AND entity_id = ?`, which without an index is a full table scan. A composite index keeps the lookup O(log n).

- [x] **Step 1: Create the migration SQL file**

Create `core/migrations/003_ai.sql`:

```sql
-- Per-entity lookup: "give me every AI decision about this message"
CREATE INDEX IF NOT EXISTS idx_action_log_entity
    ON action_log(entity_type, entity_id);

-- Secondary index for audit-by-type queries ("show me all priority_score actions from last week")
CREATE INDEX IF NOT EXISTS idx_action_log_type_time
    ON action_log(action_type, created_at DESC);
```

- [x] **Step 2: Register the migration**

Edit `core/src/store/migrations.rs`. Update the `MIGRATIONS` slice:

```rust
const MIGRATIONS: &[(&str, &str)] = &[
    ("001_initial", include_str!("../../migrations/001_initial.sql")),
    ("002_knowledge", include_str!("../../migrations/002_knowledge.sql")),
    ("003_ai", include_str!("../../migrations/003_ai.sql")),
];
```

- [x] **Step 3: Verify migration runs cleanly against an in-memory store**

The existing `Store::open_in_memory` runs all migrations. Verify by running any existing store test:

Run: `cargo test -p messagehub-core --test store_messages_test`
Expected: PASS.

- [x] **Step 4: Commit**

```bash
git add core/migrations/003_ai.sql core/src/store/migrations.rs
git commit -m "feat(ai): add action_log indexes for per-entity and per-type lookup"
```

---

### Task 3: Store Helper — AI Decision Logging

**Files:**
- Create: `core/src/store/ai_log.rs`
- Modify: `core/src/store/mod.rs`
- Test: `core/tests/store_ai_log_test.rs`

`★ Why this matters:` Classification decisions need to be audit-trail-able. The `action_log` table already exists from Plan 1 — we just need typed helpers so callers don't write raw SQL.

- [x] **Step 1: Write the failing integration test**

Create `core/tests/store_ai_log_test.rs`:

```rust
use messagehub_core::store::{AiDecision, Store};
use uuid::Uuid;

#[test]
fn test_log_and_retrieve_ai_decision() {
    let store = Store::open_in_memory().unwrap();
    let message_id = Uuid::new_v4();

    store
        .log_ai_decision(
            "classify",
            "message",
            &message_id.to_string(),
            "Sender is daughter; school topic -> family/high priority",
            0.87,
        )
        .unwrap();

    let decisions = store
        .list_ai_decisions_for_entity("message", &message_id.to_string())
        .unwrap();
    assert_eq!(decisions.len(), 1);
    let d: &AiDecision = &decisions[0];
    assert_eq!(d.action_type, "classify");
    assert_eq!(d.entity_type, Some("message".to_string()));
    assert!(d.reasoning.as_deref().unwrap().contains("daughter"));
    assert!((d.confidence_score.unwrap() - 0.87).abs() < 1e-6);
}

#[test]
fn test_multiple_decisions_per_entity_returned_in_insertion_order() {
    let store = Store::open_in_memory().unwrap();
    let message_id = Uuid::new_v4();

    store
        .log_ai_decision("classify", "message", &message_id.to_string(), "first", 0.5)
        .unwrap();
    store
        .log_ai_decision("reprioritize", "message", &message_id.to_string(), "second", 0.9)
        .unwrap();

    let decisions = store
        .list_ai_decisions_for_entity("message", &message_id.to_string())
        .unwrap();
    assert_eq!(decisions.len(), 2);
    assert_eq!(decisions[0].action_type, "classify");
    assert_eq!(decisions[1].action_type, "reprioritize");
}

#[test]
fn test_list_returns_empty_for_unknown_entity() {
    let store = Store::open_in_memory().unwrap();
    let decisions = store
        .list_ai_decisions_for_entity("message", "nonexistent")
        .unwrap();
    assert!(decisions.is_empty());
}
```

- [x] **Step 2: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test store_ai_log_test -- --nocapture`
Expected: FAIL with compile errors (`AiDecision`, `log_ai_decision`, `list_ai_decisions_for_entity` don't exist).

- [x] **Step 3: Create the store helper**

Create `core/src/store/ai_log.rs`:

```rust
use rusqlite::params;

use crate::error::{CoreError, Result};
use crate::store::Store;

/// A row from `action_log`. Populated by `log_ai_decision`.
#[derive(Debug, Clone)]
pub struct AiDecision {
    pub id: i64,
    pub action_type: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub reasoning: Option<String>,
    pub confidence_score: Option<f64>,
    pub created_at: String,
}

impl Store {
    /// Append an AI decision row to `action_log`.
    ///
    /// Used by the classifier and pipeline to record every enrichment
    /// decision with a reasoning string and confidence score, which the
    /// UI surfaces as "Why is this prioritized?"
    pub fn log_ai_decision(
        &self,
        action_type: &str,
        entity_type: &str,
        entity_id: &str,
        reasoning: &str,
        confidence_score: f64,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO action_log (action_type, entity_type, entity_id, reasoning, confidence_score)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![action_type, entity_type, entity_id, reasoning, confidence_score],
        )?;
        Ok(())
    }

    /// Return every decision about a given entity, oldest first.
    ///
    /// The ORDER BY `id` asc is deterministic even when multiple rows
    /// share the same `created_at` (sub-second inserts); the `id` column
    /// is autoincrement so insertion order is preserved.
    pub fn list_ai_decisions_for_entity(
        &self,
        entity_type: &str,
        entity_id: &str,
    ) -> Result<Vec<AiDecision>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, action_type, entity_type, entity_id, reasoning, confidence_score, created_at
             FROM action_log
             WHERE entity_type = ?1 AND entity_id = ?2
             ORDER BY id ASC",
        )?;
        let rows: std::result::Result<Vec<AiDecision>, rusqlite::Error> = stmt
            .query_map(params![entity_type, entity_id], |row| {
                Ok(AiDecision {
                    id: row.get(0)?,
                    action_type: row.get(1)?,
                    entity_type: row.get(2)?,
                    entity_id: row.get(3)?,
                    reasoning: row.get(4)?,
                    confidence_score: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?
            .collect();
        rows.map_err(CoreError::Database)
    }
}
```

- [x] **Step 4: Register the new module**

Edit `core/src/store/mod.rs`. Add `pub mod ai_log;` to the top of the file alongside the other module declarations:

```rust
pub mod ai_log;
pub mod channels;
pub mod contacts;
pub mod knowledge;
pub mod messages;
mod migrations;
```

And add a re-export just before `pub struct Store { ... }`:

```rust
pub use ai_log::AiDecision;
```

- [x] **Step 5: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test store_ai_log_test -- --nocapture`
Expected: 3 passing tests.

- [x] **Step 6: Commit**

```bash
git add core/src/store/ai_log.rs core/src/store/mod.rs core/tests/store_ai_log_test.rs
git commit -m "feat(ai): add Store::log_ai_decision and list_ai_decisions_for_entity"
```

---

### Task 4: LlmBackend Trait + OllamaLlm HTTP Client

**Files:**
- Modify: `core/Cargo.toml`
- Modify: `core/src/ai/llm.rs`
- Test: `core/tests/ai_llm_test.rs`

`★ Why this matters:` This task intentionally isolates the backend behind a trait so (a) integration tests can inject a mock LLM without running a real model and (b) if Ollama is later replaced with `llama.cpp` FFI or a different HTTP runtime (LM Studio, vLLM), only this file changes. The `OllamaLlm` implementation posts to `/api/chat` with `stream: false` — we don't stream in Tier 1 because classification is a single short response.

- [x] **Step 1: Add `wiremock` as a dev-dependency**

Edit `core/Cargo.toml`. Update the `[dev-dependencies]` section:

```toml
[dev-dependencies]
tempfile = "3"
wiremock = "0.6"
```

- [x] **Step 2: Write the failing integration test**

Create `core/tests/ai_llm_test.rs`:

```rust
use messagehub_core::ai::{LlmBackend, OllamaLlm};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_ollama_complete_posts_to_api_chat_and_parses_response() {
    let server = MockServer::start().await;

    // Canned Ollama /api/chat response for stream=false.
    let body = serde_json::json!({
        "model": "phi3:mini",
        "created_at": "2026-04-17T12:00:00Z",
        "message": { "role": "assistant", "content": "hello back" },
        "done": true
    });

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(&server)
        .await;

    let llm = OllamaLlm::new(server.uri(), "phi3:mini".to_string());
    let out = llm
        .complete("you are a test", "say hello", 32)
        .await
        .unwrap();
    assert_eq!(out, "hello back");
}

#[tokio::test]
async fn test_ollama_complete_returns_error_on_5xx() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let llm = OllamaLlm::new(server.uri(), "phi3:mini".to_string());
    let err = llm.complete("s", "u", 32).await.unwrap_err();
    let msg = format!("{}", err);
    assert!(
        msg.contains("ollama") || msg.contains("500") || msg.contains("ai"),
        "error does not mention ollama/500/ai: {}",
        msg
    );
}

#[tokio::test]
async fn test_ollama_complete_returns_error_on_malformed_body() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;

    let llm = OllamaLlm::new(server.uri(), "phi3:mini".to_string());
    assert!(llm.complete("s", "u", 32).await.is_err());
}

#[tokio::test]
async fn test_ollama_health_check_hits_api_tags() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"models": []})))
        .expect(1)
        .mount(&server)
        .await;

    let llm = OllamaLlm::new(server.uri(), "phi3:mini".to_string());
    assert!(llm.health_check().await.unwrap());
}

#[tokio::test]
async fn test_ollama_health_check_returns_false_when_server_down() {
    // Point at a localhost port that nothing is listening on.
    let llm = OllamaLlm::new("http://127.0.0.1:1".to_string(), "phi3:mini".to_string());
    assert_eq!(llm.health_check().await.unwrap(), false);
}
```

- [x] **Step 3: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test ai_llm_test -- --nocapture`
Expected: FAIL with compile errors (`OllamaLlm::new`, `health_check`, and the full trait body don't exist yet).

- [x] **Step 4: Implement the LlmBackend trait and OllamaLlm client**

Replace the entire contents of `core/src/ai/llm.rs`:

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

use crate::error::{CoreError, Result};

/// Abstraction over a local LLM that the classifier talks to.
///
/// The only method required is `complete`: given a system prompt and a
/// user prompt, produce a single assistant response. No streaming —
/// classification is a short single-shot call.
///
/// Implementations must be `Send + Sync` so they can be shared across
/// the AI pipeline's async tasks.
#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn complete(&self, system: &str, user: &str, max_tokens: u32) -> Result<String>;
}

/// HTTP client for Ollama's `/api/chat` endpoint.
///
/// Sends `stream: false` requests and reads the full JSON body. The
/// default base URL is `http://127.0.0.1:11434`, matching an Ollama
/// default installation. The model name (e.g. `"phi3:mini"`) is
/// configured at construction — the pipeline does not ship a hardcoded
/// default so tests and deployments can swap freely.
pub struct OllamaLlm {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

impl OllamaLlm {
    pub fn new(base_url: String, model: String) -> Self {
        // Classification should complete in <500ms on a 3B-param model but
        // cold starts can take several seconds. 60s is generous but not
        // unbounded — callers hang otherwise if the model fails to load.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client builder never fails with default config");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
        }
    }

    /// Check whether Ollama is reachable and responsive.
    ///
    /// Returns `Ok(false)` if the HTTP request fails (connection refused,
    /// timeout, 5xx). Returns `Ok(true)` on any 2xx. Only propagates
    /// `Err(...)` for logic bugs — never for "server unreachable", which
    /// is an expected-runtime condition the pipeline handles gracefully.
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/api/tags", self.base_url);
        match self.client.get(&url).send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(e) => {
                debug!(error = %e, url = %url, "ollama health check failed");
                Ok(false)
            }
        }
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
    options: ChatOptions,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatOptions {
    /// Deterministic-ish: low temperature for classification.
    temperature: f32,
    /// Cap response length.
    num_predict: i32,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

#[async_trait]
impl LlmBackend for OllamaLlm {
    async fn complete(&self, system: &str, user: &str, max_tokens: u32) -> Result<String> {
        let url = format!("{}/api/chat", self.base_url);
        let req = ChatRequest {
            model: &self.model,
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: system,
                },
                ChatMessage {
                    role: "user",
                    content: user,
                },
            ],
            stream: false,
            options: ChatOptions {
                temperature: 0.1,
                num_predict: max_tokens as i32,
            },
        };

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| CoreError::Ai(format!("ollama request failed: {}", e)))?;

        let status = resp.status();
        if !status.is_success() {
            let body_preview = resp.text().await.unwrap_or_default();
            let body_preview: String = body_preview.chars().take(200).collect();
            warn!(status = %status, body_preview = %body_preview, "ollama returned non-2xx");
            return Err(CoreError::Ai(format!(
                "ollama returned {} — {}",
                status, body_preview
            )));
        }

        let parsed: ChatResponse = resp
            .json()
            .await
            .map_err(|e| CoreError::Ai(format!("ollama response body is not valid chat JSON: {}", e)))?;

        Ok(parsed.message.content)
    }
}
```

- [x] **Step 5: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test ai_llm_test -- --nocapture`
Expected: 5 passing tests.

- [x] **Step 6: Commit**

```bash
git add core/Cargo.toml core/src/ai/llm.rs core/tests/ai_llm_test.rs
git commit -m "feat(ai): add LlmBackend trait and OllamaLlm HTTP client"
```

---

### Task 5: UserProfile Loader

**Files:**
- Modify: `core/src/ai/profile.rs`
- Test: `core/tests/ai_profile_test.rs`

`★ Why this matters:` The spec says "always include `user-profile.md` context: languages (EN/FR/DE), role, tone, life areas." The profile file is at a user-configured path (typically `02-Areas/User/user-profile.md` in the vault). We load it once at startup, cache it, and truncate if it's too long — the whole file shouldn't blow out the LLM context window. The file watcher from Plan 3 could later invalidate the cache; for now we load-and-hold.

- [x] **Step 1: Write the failing unit test**

Create `core/tests/ai_profile_test.rs`:

```rust
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
```

- [x] **Step 2: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test ai_profile_test -- --nocapture`
Expected: FAIL with compile errors (`UserProfile::load`, `has_content` do not exist).

- [x] **Step 3: Implement the UserProfile loader**

Replace the entire contents of `core/src/ai/profile.rs`:

```rust
use std::path::Path;

use tracing::{debug, warn};

use crate::error::Result;

/// The user's self-authored profile, loaded from a vault markdown file
/// (typically `02-Areas/User/user-profile.md`).
///
/// The content is injected into every classification prompt so the LLM
/// has standing context about the user's languages, role, and life areas.
/// The file is truncated to `MAX_PROFILE_CHARS` to keep the prompt under
/// a reasonable token budget.
pub struct UserProfile {
    pub content: String,
}

/// Character budget for the profile content injected into prompts.
/// At ~4 chars/token this is ~1000 tokens — comfortable for a 3B model's
/// context window while leaving room for the message and retrieved chunks.
const MAX_PROFILE_CHARS: usize = 4_000;

impl UserProfile {
    /// Load the profile from a markdown file on disk.
    ///
    /// If the file doesn't exist, returns an empty profile rather than
    /// an error — the pipeline degrades gracefully when there's no
    /// user-authored profile yet.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(raw) => {
                let content = if raw.chars().count() > MAX_PROFILE_CHARS {
                    let truncated: String = raw.chars().take(MAX_PROFILE_CHARS - 12).collect();
                    warn!(
                        path = %path.display(),
                        original_chars = raw.chars().count(),
                        "user profile truncated to fit prompt budget"
                    );
                    format!("{}[truncated]", truncated)
                } else {
                    raw
                };
                debug!(path = %path.display(), chars = content.len(), "user profile loaded");
                Ok(Self { content })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!(path = %path.display(), "user profile file not found; using empty profile");
                Ok(Self {
                    content: String::new(),
                })
            }
            Err(e) => {
                warn!(path = %path.display(), error = %e, "failed to read user profile");
                Ok(Self {
                    content: String::new(),
                })
            }
        }
    }

    /// True when the profile has any non-whitespace content.
    pub fn has_content(&self) -> bool {
        !self.content.trim().is_empty()
    }
}
```

- [x] **Step 4: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test ai_profile_test -- --nocapture`
Expected: 4 passing tests.

- [x] **Step 5: Commit**

```bash
git add core/src/ai/profile.rs core/tests/ai_profile_test.rs
git commit -m "feat(ai): add UserProfile loader with char-budget truncation"
```

---

### Task 6: RagContext Builder

**Files:**
- Modify: `core/src/ai/rag.rs`
- Test: `core/tests/ai_rag_test.rs`

`★ Why this matters:` This is where the knowledge engine built in Plan 3 pays off. For every incoming message, we need three pieces of grounding context: (a) who is this sender according to the vault, (b) what vault chunks are semantically relevant to the message body, (c) the user's standing profile. `RagContext` bundles all three so prompt assembly has everything in one place.

- [x] **Step 1: Write the failing integration test**

Create `core/tests/ai_rag_test.rs`:

```rust
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
```

- [x] **Step 2: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test ai_rag_test -- --nocapture`
Expected: FAIL with compile errors.

- [x] **Step 3: Implement `RagContext` and `build_rag_context`**

Replace the entire contents of `core/src/ai/rag.rs`:

```rust
use std::sync::Arc;

use crate::ai::profile::UserProfile;
use crate::error::Result;
use crate::knowledge::{RetrievalFilters, Retriever};
use crate::store::Store;
use crate::types::Channel;

/// Everything the classifier prompt needs about the world surrounding
/// an incoming message.
///
/// Contains three buckets: (1) who the sender is per the vault,
/// (2) vault chunks semantically near the message body, (3) the user's
/// standing profile. All three are optional — any combination can be
/// empty and the prompt will still be well-formed.
#[derive(Debug, Clone)]
pub struct RagContext {
    pub sender_name: Option<String>,
    pub sender_vault_path: Option<String>,
    pub topic_chunks: Vec<ContextChunk>,
    pub user_profile_content: String,
}

/// One vault chunk surfaced by semantic retrieval.
#[derive(Debug, Clone)]
pub struct ContextChunk {
    pub file_path: String,
    pub heading: Option<String>,
    pub content: String,
}

impl RagContext {
    /// Render this context as a markdown section suitable to paste into
    /// a user prompt. Uses headings and bullet points for clarity.
    pub fn to_prompt_section(&self) -> String {
        let mut out = String::new();

        // Sender section
        out.push_str("# Sender context (from vault)\n");
        match (&self.sender_name, &self.sender_vault_path) {
            (Some(name), Some(path)) => {
                out.push_str(&format!("- Known contact: {} (profile: {})\n", name, path));
            }
            _ => {
                out.push_str("- Unknown sender — no vault profile match.\n");
            }
        }
        out.push('\n');

        // Topic chunks
        out.push_str("# Relevant vault notes\n");
        if self.topic_chunks.is_empty() {
            out.push_str("- (no vault content matched this message)\n");
        } else {
            for chunk in &self.topic_chunks {
                let heading = chunk.heading.as_deref().unwrap_or("(no heading)");
                out.push_str(&format!(
                    "- [{} — {}] {}\n",
                    chunk.file_path,
                    heading,
                    chunk.content.trim()
                ));
            }
        }
        out.push('\n');

        // User profile
        out.push_str("# User profile\n");
        if self.user_profile_content.trim().is_empty() {
            out.push_str("- (no profile configured)\n");
        } else {
            out.push_str(&self.user_profile_content);
            if !self.user_profile_content.ends_with('\n') {
                out.push('\n');
            }
        }

        out
    }
}

/// Assemble a `RagContext` for an incoming message.
///
/// Parameters:
/// - `store` — live database handle
/// - `retriever` — optional vault retriever. If `None`, topic chunks are
///   skipped (useful for tests that don't want to load the embedder, and
///   for the degraded mode where the knowledge engine is disabled).
/// - `profile` — pre-loaded user profile (empty if not configured)
/// - `channel`, `sender_address` — used for sender lookup via `Store::find_vault_person_by_address`
/// - `subject`, `body` — combined into the retrieval query string
///
/// The retrieval filter is left `Default` so the top-k pulls from any
/// PARA folder. Callers that want folder-scoped retrieval (e.g. "only
/// business notes for work messages") can extend this signature later.
pub fn build_rag_context(
    store: &Store,
    retriever: Option<&Arc<Retriever>>,
    profile: &UserProfile,
    channel: Channel,
    sender_address: &str,
    subject: &str,
    body: &str,
) -> Result<RagContext> {
    let (sender_name, sender_vault_path) =
        match store.find_vault_person_by_address(channel.to_db_str(), sender_address)? {
            Some((name, path)) => (Some(name), Some(path)),
            None => (None, None),
        };

    let topic_chunks = match retriever {
        Some(r) => {
            let query = build_retrieval_query(subject, body);
            let filters = RetrievalFilters {
                para_folders: None,
                top_k: Some(5),
            };
            r.search(store, &query, &filters)?
                .into_iter()
                .map(|rc| ContextChunk {
                    file_path: rc.file_path,
                    heading: rc.section_heading,
                    content: rc.content,
                })
                .collect()
        }
        None => Vec::new(),
    };

    Ok(RagContext {
        sender_name,
        sender_vault_path,
        topic_chunks,
        user_profile_content: profile.content.clone(),
    })
}

/// Combine subject and body into a single retrieval query.
/// Subject gets repeated twice so keyword-like phrasing has a stronger
/// signal than a single mention inside a long body.
fn build_retrieval_query(subject: &str, body: &str) -> String {
    let subject = subject.trim();
    let body_excerpt: String = body.trim().chars().take(500).collect();
    if subject.is_empty() {
        body_excerpt
    } else {
        format!("{}\n{}\n{}", subject, subject, body_excerpt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_retrieval_query_handles_empty_subject() {
        let q = build_retrieval_query("", "Hello world");
        assert_eq!(q, "Hello world");
    }

    #[test]
    fn test_build_retrieval_query_emphasizes_subject() {
        let q = build_retrieval_query("Urgent", "detail");
        let count = q.matches("Urgent").count();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_build_retrieval_query_truncates_long_bodies() {
        let long = "x".repeat(1_000);
        let q = build_retrieval_query("S", &long);
        // "S\nS\n" prefix (4 chars) + at most 500 body chars = 504
        assert!(q.chars().count() <= 504);
    }
}
```

- [x] **Step 4: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test ai_rag_test -- --nocapture`
Expected: 4 passing tests.

Also re-run the inline tests:
Run: `cargo test -p messagehub-core ai::rag -- --nocapture`
Expected: 3 passing tests.

- [x] **Step 5: Commit**

```bash
git add core/src/ai/rag.rs core/tests/ai_rag_test.rs
git commit -m "feat(ai): add RagContext builder combining sender, retrieval, and profile"
```

---

### Task 7: Classification Prompt + JSON Parser

**Files:**
- Modify: `core/src/ai/prompts.rs`
- Test: `core/tests/ai_prompts_test.rs`

`★ Why this matters:` This is the most fragile surface: a free-text LLM has to emit structured data we can trust. Two defenses: (a) the system prompt explicitly demands a single JSON object with an enumerated field set, and (b) the parser rejects anything that doesn't match — no fuzzy "maybe they meant X" logic. If the LLM hallucinates a category we don't know, the whole classification is thrown out and the message is stored with `priority = None`. That's safer than guessing.

- [x] **Step 1: Write the failing unit tests**

Create `core/tests/ai_prompts_test.rs`:

```rust
use messagehub_core::ai::prompts::{
    CLASSIFICATION_SYSTEM_PROMPT, build_classification_user_prompt, parse_classification_response,
};
use messagehub_core::ai::{Category, RagContext};
use messagehub_core::types::Channel;

#[test]
fn test_system_prompt_enumerates_categories() {
    for cat in Category::all_strs() {
        assert!(
            CLASSIFICATION_SYSTEM_PROMPT.contains(cat),
            "system prompt missing category literal '{}'",
            cat
        );
    }
    assert!(CLASSIFICATION_SYSTEM_PROMPT.contains("priority"));
    assert!(CLASSIFICATION_SYSTEM_PROMPT.contains("category"));
    assert!(CLASSIFICATION_SYSTEM_PROMPT.contains("reasoning"));
}

#[test]
fn test_build_user_prompt_includes_message_fields() {
    let ctx = RagContext {
        sender_name: Some("Alice".to_string()),
        sender_vault_path: None,
        topic_chunks: vec![],
        user_profile_content: "Role: freelancer".to_string(),
    };
    let prompt = build_classification_user_prompt(
        Channel::Email,
        "Alice Example",
        "alice@example.com",
        "Project X update",
        "Hey, are we still on for tomorrow?",
        &ctx,
    );
    assert!(prompt.contains("Email"));
    assert!(prompt.contains("Alice Example"));
    assert!(prompt.contains("alice@example.com"));
    assert!(prompt.contains("Project X update"));
    assert!(prompt.contains("still on for tomorrow"));
    assert!(prompt.contains("freelancer"));
    assert!(prompt.contains("Alice"));
}

#[test]
fn test_build_user_prompt_handles_none_subject() {
    let ctx = RagContext {
        sender_name: None,
        sender_vault_path: None,
        topic_chunks: vec![],
        user_profile_content: String::new(),
    };
    let prompt = build_classification_user_prompt(
        Channel::Telegram,
        "Bob",
        "@bob",
        "", // no subject for IM channels
        "Ping",
        &ctx,
    );
    assert!(prompt.contains("Telegram"));
    assert!(prompt.contains("Bob"));
    assert!(prompt.contains("Ping"));
}

#[test]
fn test_parse_response_valid_json() {
    let raw = r#"{"priority": 4, "category": "family", "reasoning": "Sender is daughter."}"#;
    let parsed = parse_classification_response(raw).unwrap();
    assert_eq!(parsed.priority.value(), 4);
    assert_eq!(parsed.category, Category::Family);
    assert_eq!(parsed.reasoning, "Sender is daughter.");
}

#[test]
fn test_parse_response_tolerates_markdown_code_fence() {
    let raw = "```json\n{\"priority\": 2, \"category\": \"newsletters\", \"reasoning\": \"Bulk promo.\"}\n```";
    let parsed = parse_classification_response(raw).unwrap();
    assert_eq!(parsed.priority.value(), 2);
    assert_eq!(parsed.category, Category::Newsletters);
}

#[test]
fn test_parse_response_tolerates_leading_explanation_text() {
    let raw = r#"Here is my classification:
{"priority": 5, "category": "work", "reasoning": "Deadline today"}"#;
    let parsed = parse_classification_response(raw).unwrap();
    assert_eq!(parsed.priority.value(), 5);
    assert_eq!(parsed.category, Category::Work);
}

#[test]
fn test_parse_rejects_priority_out_of_range() {
    let raw = r#"{"priority": 10, "category": "work", "reasoning": "x"}"#;
    assert!(parse_classification_response(raw).is_err());

    let raw = r#"{"priority": 0, "category": "work", "reasoning": "x"}"#;
    assert!(parse_classification_response(raw).is_err());
}

#[test]
fn test_parse_rejects_unknown_category() {
    let raw = r#"{"priority": 3, "category": "important", "reasoning": "x"}"#;
    assert!(parse_classification_response(raw).is_err());
}

#[test]
fn test_parse_rejects_missing_fields() {
    let raw = r#"{"priority": 3}"#;
    assert!(parse_classification_response(raw).is_err());
}

#[test]
fn test_parse_rejects_non_json() {
    let raw = "The message is important, priority 4, work category.";
    assert!(parse_classification_response(raw).is_err());
}
```

- [x] **Step 2: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test ai_prompts_test -- --nocapture`
Expected: FAIL — compile errors for missing items.

- [x] **Step 3: Implement the prompt module**

Replace the entire contents of `core/src/ai/prompts.rs`:

```rust
use serde::Deserialize;

use crate::ai::classifier::Classification;
use crate::ai::{Category, RagContext};
use crate::error::{CoreError, Result};
use crate::types::{Channel, PriorityScore};

/// System prompt for the one-shot classification call.
///
/// The model MUST emit a single JSON object. The system prompt enumerates
/// every legal category so the model has no freedom to invent values.
/// We also ask for reasoning so the UI can show "Why is this prioritized?"
pub const CLASSIFICATION_SYSTEM_PROMPT: &str = r#"You are an inbox classification assistant running locally on the user's device. Your job is to prioritize and categorize incoming messages.

You must respond with a single JSON object and nothing else. The schema is strict:

{
  "priority": <integer 1 to 5>,
  "category": <one of: "work", "personal", "finance", "family", "notifications", "newsletters", "spam">,
  "reasoning": <one short sentence explaining your choice>
}

Priority scale:
- 5 = urgent, needs action today (personal emergencies, hard deadlines, direct asks from family/key clients)
- 4 = important, action this week (replies needed from known contacts, meeting confirmations)
- 3 = normal (regular conversation, FYI)
- 2 = low (newsletters you subscribed to, non-urgent notifications)
- 1 = spam or irrelevant (promotional, unknown bulk senders, noise)

Use the "Sender context" section to identify who the sender is relative to the user's vault. Known contacts from the user's vault (especially family and work relationships) should generally score higher.

Use the "Relevant vault notes" section to understand project and topic context.

Use the "User profile" section to understand the user's languages, role, and life areas.

Do not include any text outside the JSON object. Do not wrap the JSON in code fences unless you absolutely cannot avoid it.
"#;

/// Build the user-turn prompt.
///
/// Layout:
/// - Incoming message metadata (channel, sender, subject, body)
/// - RAG context rendered via `RagContext::to_prompt_section`
/// - Final "Classify this message." instruction
pub fn build_classification_user_prompt(
    channel: Channel,
    sender_name: &str,
    sender_address: &str,
    subject: &str,
    body: &str,
    rag: &RagContext,
) -> String {
    let mut out = String::new();
    out.push_str("# Incoming message\n");
    out.push_str(&format!("Channel: {}\n", channel));
    out.push_str(&format!("From: {} <{}>\n", sender_name, sender_address));
    if !subject.trim().is_empty() {
        out.push_str(&format!("Subject: {}\n", subject));
    }
    out.push_str("\nBody:\n");
    // Truncate very long bodies to keep the prompt in budget.
    let body_excerpt: String = body.chars().take(2_000).collect();
    out.push_str(body_excerpt.trim());
    out.push_str("\n\n");

    out.push_str(&rag.to_prompt_section());
    out.push_str("\nClassify this message.\n");
    out
}

/// Parse the LLM's raw response into a `Classification`.
///
/// Accepts:
/// - Pure JSON
/// - JSON wrapped in triple-backtick code fences
/// - JSON preceded by a leading explanation (we extract the first balanced {...} block)
///
/// Rejects:
/// - Priority outside 1..=5
/// - Category not in `Category::all_strs`
/// - Missing `priority`, `category`, or `reasoning` fields
/// - No JSON object at all
pub fn parse_classification_response(raw: &str) -> Result<Classification> {
    let stripped = strip_fences(raw);
    let json_slice = extract_first_json_object(&stripped)
        .ok_or_else(|| CoreError::Ai(format!("no JSON object found in response: {:?}", raw)))?;

    let parsed: RawResponse = serde_json::from_str(json_slice)
        .map_err(|e| CoreError::Ai(format!("response JSON does not match schema: {}", e)))?;

    let priority = PriorityScore::new(parsed.priority).ok_or_else(|| {
        CoreError::Ai(format!(
            "priority {} out of range 1..=5",
            parsed.priority
        ))
    })?;
    let category = Category::from_str(&parsed.category).ok_or_else(|| {
        CoreError::Ai(format!(
            "unknown category '{}'; must be one of {:?}",
            parsed.category,
            Category::all_strs()
        ))
    })?;
    if parsed.reasoning.trim().is_empty() {
        return Err(CoreError::Ai("empty reasoning field".to_string()));
    }

    Ok(Classification {
        priority,
        category,
        reasoning: parsed.reasoning,
    })
}

#[derive(Deserialize)]
struct RawResponse {
    priority: u8,
    category: String,
    reasoning: String,
}

/// Strip triple-backtick fences (with or without a `json` language tag).
fn strip_fences(raw: &str) -> String {
    let trimmed = raw.trim();
    let fence = "```";
    if let Some(rest) = trimmed.strip_prefix(fence) {
        // Drop optional language tag on first line.
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        let rest = rest.strip_suffix(fence).unwrap_or(rest);
        return rest.trim().to_string();
    }
    trimmed.to_string()
}

/// Find the first `{...}` block with balanced braces.
///
/// Naive but reliable for LLM outputs that don't contain strings with
/// unescaped braces. If the JSON contains a `}` inside a string, this
/// could mismatch — but `serde_json::from_str` would then fail with a
/// parse error, which surfaces as `CoreError::Ai` upstream.
fn extract_first_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let mut depth = 0;
    for (i, c) in s[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=start + i]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_fences_noop_when_absent() {
        assert_eq!(strip_fences("{\"a\": 1}"), "{\"a\": 1}");
    }

    #[test]
    fn test_strip_fences_handles_json_label() {
        let input = "```json\n{\"a\": 1}\n```";
        assert_eq!(strip_fences(input), "{\"a\": 1}");
    }

    #[test]
    fn test_extract_first_json_object_finds_balanced_block() {
        let input = "prefix {\"a\": {\"b\": 1}} suffix";
        assert_eq!(
            extract_first_json_object(input).unwrap(),
            "{\"a\": {\"b\": 1}}"
        );
    }

    #[test]
    fn test_extract_first_json_object_returns_none_when_no_object() {
        assert_eq!(extract_first_json_object("no braces here"), None);
    }
}
```

- [x] **Step 4: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test ai_prompts_test -- --nocapture`
Expected: 10 passing tests.

Also re-run the inline tests:
Run: `cargo test -p messagehub-core ai::prompts -- --nocapture`
Expected: 4 passing tests.

- [x] **Step 5: Commit**

```bash
git add core/src/ai/prompts.rs core/tests/ai_prompts_test.rs
git commit -m "feat(ai): add classification prompt template and strict JSON parser"
```

---

### Task 8: Classifier Wiring

**Files:**
- Modify: `core/src/ai/classifier.rs`
- Test: `core/tests/ai_classifier_test.rs`

`★ Why this matters:` The classifier ties `LlmBackend` + prompt builder + response parser into a single `classify()` call. Keeping these concerns in separate modules pays off here: the classifier is a dozen lines of glue. Tests use a scripted impl of `LlmBackend` to return canned responses, so no model download or Ollama server is needed.

- [x] **Step 1: Write the failing integration test**

Create `core/tests/ai_classifier_test.rs`:

```rust
use async_trait::async_trait;
use messagehub_core::ai::{Category, Classifier, LlmBackend, RagContext};
use messagehub_core::error::{CoreError, Result};
use messagehub_core::types::Channel;
use std::sync::Arc;
use std::sync::Mutex;

struct ScriptedLlm {
    responses: Mutex<Vec<Result<String>>>,
    last_user_prompt: Mutex<Option<String>>,
}

impl ScriptedLlm {
    fn new(responses: Vec<Result<String>>) -> Self {
        Self {
            responses: Mutex::new(responses),
            last_user_prompt: Mutex::new(None),
        }
    }
}

#[async_trait]
impl LlmBackend for ScriptedLlm {
    async fn complete(&self, _system: &str, user: &str, _max_tokens: u32) -> Result<String> {
        *self.last_user_prompt.lock().unwrap() = Some(user.to_string());
        self.responses.lock().unwrap().remove(0)
    }
}

fn empty_ctx() -> RagContext {
    RagContext {
        sender_name: None,
        sender_vault_path: None,
        topic_chunks: vec![],
        user_profile_content: String::new(),
    }
}

#[tokio::test]
async fn test_classify_happy_path() {
    let llm = Arc::new(ScriptedLlm::new(vec![Ok(
        r#"{"priority": 4, "category": "family", "reasoning": "Daughter's school."}"#.to_string(),
    )]));
    let classifier = Classifier::new(llm.clone());
    let result = classifier
        .classify(
            Channel::Email,
            "Alice",
            "alice@example.com",
            "School",
            "Hi",
            &empty_ctx(),
        )
        .await
        .unwrap();
    assert_eq!(result.priority.value(), 4);
    assert_eq!(result.category, Category::Family);
    assert!(result.reasoning.contains("school") || result.reasoning.contains("School"));

    let prompt = llm.last_user_prompt.lock().unwrap().clone().unwrap();
    assert!(prompt.contains("alice@example.com"));
    assert!(prompt.contains("School"));
}

#[tokio::test]
async fn test_classify_surfaces_llm_error() {
    let llm = Arc::new(ScriptedLlm::new(vec![Err(CoreError::Ai(
        "backend down".to_string(),
    ))]));
    let classifier = Classifier::new(llm);
    let err = classifier
        .classify(
            Channel::Email,
            "Alice",
            "alice@example.com",
            "s",
            "b",
            &empty_ctx(),
        )
        .await
        .unwrap_err();
    assert!(format!("{}", err).contains("backend down"));
}

#[tokio::test]
async fn test_classify_surfaces_parse_error() {
    let llm = Arc::new(ScriptedLlm::new(vec![Ok(
        "I think this is work priority 3".to_string(), // not JSON
    )]));
    let classifier = Classifier::new(llm);
    let err = classifier
        .classify(Channel::Email, "A", "a@x.com", "s", "b", &empty_ctx())
        .await
        .unwrap_err();
    let msg = format!("{}", err);
    assert!(msg.contains("JSON") || msg.contains("ai"));
}
```

- [x] **Step 2: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test ai_classifier_test -- --nocapture`
Expected: FAIL — `Classifier::new`, `classify` do not exist.

- [x] **Step 3: Implement the classifier**

Replace the entire contents of `core/src/ai/classifier.rs`:

```rust
use std::sync::Arc;

use tracing::{debug, warn};

use crate::ai::llm::LlmBackend;
use crate::ai::prompts::{
    CLASSIFICATION_SYSTEM_PROMPT, build_classification_user_prompt, parse_classification_response,
};
use crate::ai::{Category, RagContext};
use crate::error::Result;
use crate::types::{Channel, PriorityScore};

/// The three fields the classifier produces.
///
/// `priority` is the 1-5 integer score; `category` is one of the
/// enumerated `Category` variants; `reasoning` is a short
/// human-readable sentence the UI can surface as "Why is this
/// prioritized?"
#[derive(Debug, Clone)]
pub struct Classification {
    pub priority: PriorityScore,
    pub category: Category,
    pub reasoning: String,
}

/// Runs the one-shot classification call against an `LlmBackend`.
///
/// The classifier holds a dynamically-dispatched backend so tests can
/// inject a scripted implementation that never touches the network.
pub struct Classifier {
    llm: Arc<dyn LlmBackend>,
    /// Cap on tokens the model is allowed to generate for a single
    /// classification response. A well-behaved response is ~50 tokens.
    /// 256 leaves headroom for chain-of-thought preambles we discard.
    max_tokens: u32,
}

impl Classifier {
    pub fn new(llm: Arc<dyn LlmBackend>) -> Self {
        Self {
            llm,
            max_tokens: 256,
        }
    }

    /// Run classification for a single message.
    ///
    /// Returns `Err(CoreError::Ai(...))` if:
    /// - the backend call fails (Ollama down, timeout, 5xx)
    /// - the response is not valid JSON matching the schema
    /// - the priority is outside 1..=5 or the category is unknown
    ///
    /// The pipeline in Task 9 catches these errors and stores the
    /// message with `priority = None` — this method should not swallow.
    pub async fn classify(
        &self,
        channel: Channel,
        sender_name: &str,
        sender_address: &str,
        subject: &str,
        body: &str,
        rag: &RagContext,
    ) -> Result<Classification> {
        let user_prompt = build_classification_user_prompt(
            channel,
            sender_name,
            sender_address,
            subject,
            body,
            rag,
        );
        debug!(
            channel = %channel,
            sender = %sender_address,
            prompt_chars = user_prompt.len(),
            "running classifier"
        );

        let raw = self
            .llm
            .complete(CLASSIFICATION_SYSTEM_PROMPT, &user_prompt, self.max_tokens)
            .await?;

        match parse_classification_response(&raw) {
            Ok(classification) => {
                debug!(
                    priority = classification.priority.value(),
                    category = classification.category.as_str(),
                    "classification succeeded"
                );
                Ok(classification)
            }
            Err(e) => {
                warn!(
                    raw_preview = %raw.chars().take(200).collect::<String>(),
                    error = %e,
                    "classifier parse failure"
                );
                Err(e)
            }
        }
    }
}
```

- [x] **Step 4: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test ai_classifier_test -- --nocapture`
Expected: 3 passing tests.

- [x] **Step 5: Commit**

```bash
git add core/src/ai/classifier.rs core/tests/ai_classifier_test.rs
git commit -m "feat(ai): add Classifier tying LlmBackend + prompts + parser"
```

---

### Task 9: AiPipeline Orchestrator

**Files:**
- Modify: `core/src/ai/pipeline.rs`
- Test: `core/tests/ai_pipeline_test.rs`

`★ Why this matters:` This is the public entry point the channel adapter manager calls. It wraps: build RAG context → classify → attach priority/category to the `Message` → persist via `Store::insert_message` → log the decision to `action_log`. Graceful degradation: when the LLM fails, the message is still stored — just with `priority = None` and `category = None`, and the failure is logged so the UI can surface a retry.

`★ Design note:` The pipeline takes a `Message` (not `RawMessage`) because sender/thread resolution is the adapter manager's responsibility (see `adapters::normalize`). The pipeline only owns the AI-enrichment step.

- [x] **Step 1: Write the failing end-to-end integration test**

Create `core/tests/ai_pipeline_test.rs`:

```rust
use async_trait::async_trait;
use chrono::Utc;
use messagehub_core::ai::{AiPipeline, LlmBackend, UserProfile};
use messagehub_core::error::Result;
use messagehub_core::store::Store;
use messagehub_core::types::{
    Channel, Contact, ContactIdentity, Message, MessageContent, Thread,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

struct ScriptedLlm {
    next: Mutex<Option<Result<String>>>,
}

impl ScriptedLlm {
    fn ok(body: &str) -> Self {
        Self {
            next: Mutex::new(Some(Ok(body.to_string()))),
        }
    }
    fn err(e: messagehub_core::error::CoreError) -> Self {
        Self {
            next: Mutex::new(Some(Err(e))),
        }
    }
}

#[async_trait]
impl LlmBackend for ScriptedLlm {
    async fn complete(&self, _system: &str, _user: &str, _max_tokens: u32) -> Result<String> {
        self.next.lock().unwrap().take().unwrap()
    }
}

fn seed_sender(store: &Store) -> (Uuid, Uuid) {
    // Build a minimum contact + thread so insert_message FK constraints pass.
    let contact_id = Uuid::new_v4();
    let thread_id = Uuid::new_v4();
    store
        .insert_contact(&Contact {
            id: contact_id,
            display_name: "Alice".to_string(),
            identities: vec![ContactIdentity {
                channel: Channel::Email,
                address: "alice@example.com".to_string(),
            }],
            vault_ref: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .unwrap();
    store
        .insert_thread(&Thread {
            id: thread_id,
            channel: Channel::Email,
            subject: Some("Hi".to_string()),
            message_count: 0,
            last_message_at: Utc::now(),
            created_at: Utc::now(),
        })
        .unwrap();
    (contact_id, thread_id)
}

fn make_msg(sender_id: Uuid, thread_id: Uuid) -> Message {
    Message {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        thread_id,
        sender_id,
        content: MessageContent {
            text: Some("Are we still on for tomorrow?".to_string()),
            html: None,
            subject: Some("Project X update".to_string()),
            attachments: vec![],
        },
        timestamp: Utc::now(),
        metadata: HashMap::new(),
        priority: None,
        category: None,
        is_read: false,
        is_archived: false,
    }
}

#[tokio::test]
async fn test_pipeline_happy_path_stores_enriched_message_and_logs_decision() {
    let store = Store::open_in_memory().unwrap();
    let (sender_id, thread_id) = seed_sender(&store);
    let msg = make_msg(sender_id, thread_id);
    let message_id = msg.id;

    let llm = Arc::new(ScriptedLlm::ok(
        r#"{"priority": 4, "category": "work", "reasoning": "Active project."}"#,
    ));
    let pipeline = AiPipeline::new(
        llm,
        None, // no retriever for this test
        UserProfile {
            content: "Languages: EN".to_string(),
        },
    );

    let outcome = pipeline
        .enrich_and_store(&store, msg, "alice@example.com", "Alice")
        .await
        .unwrap();
    assert!(outcome.classified);

    let stored = store.get_message(&message_id).unwrap();
    assert_eq!(stored.priority.unwrap().value(), 4);
    assert_eq!(stored.category.as_deref(), Some("work"));

    let log = store
        .list_ai_decisions_for_entity("message", &message_id.to_string())
        .unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].action_type, "classify");
    assert!(log[0].reasoning.as_deref().unwrap().contains("project"));
}

#[tokio::test]
async fn test_pipeline_graceful_degradation_on_llm_failure() {
    use messagehub_core::error::CoreError;

    let store = Store::open_in_memory().unwrap();
    let (sender_id, thread_id) = seed_sender(&store);
    let msg = make_msg(sender_id, thread_id);
    let message_id = msg.id;

    let llm = Arc::new(ScriptedLlm::err(CoreError::Ai("ollama down".to_string())));
    let pipeline = AiPipeline::new(
        llm,
        None,
        UserProfile {
            content: String::new(),
        },
    );

    let outcome = pipeline
        .enrich_and_store(&store, msg, "alice@example.com", "Alice")
        .await
        .unwrap();
    // Degraded mode — classification failed, message still stored.
    assert!(!outcome.classified);

    let stored = store.get_message(&message_id).unwrap();
    assert!(stored.priority.is_none());
    assert!(stored.category.is_none());

    // A failure row should be in the log so the UI can surface "retry classification".
    let log = store
        .list_ai_decisions_for_entity("message", &message_id.to_string())
        .unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].action_type, "classify_failed");
    assert!(log[0].reasoning.as_deref().unwrap().contains("ollama down"));
}

#[tokio::test]
async fn test_pipeline_graceful_degradation_on_parse_failure() {
    let store = Store::open_in_memory().unwrap();
    let (sender_id, thread_id) = seed_sender(&store);
    let msg = make_msg(sender_id, thread_id);
    let message_id = msg.id;

    let llm = Arc::new(ScriptedLlm::ok("this is not valid json"));
    let pipeline = AiPipeline::new(
        llm,
        None,
        UserProfile {
            content: String::new(),
        },
    );

    let outcome = pipeline
        .enrich_and_store(&store, msg, "alice@example.com", "Alice")
        .await
        .unwrap();
    assert!(!outcome.classified);

    let stored = store.get_message(&message_id).unwrap();
    assert!(stored.priority.is_none());

    let log = store
        .list_ai_decisions_for_entity("message", &message_id.to_string())
        .unwrap();
    assert_eq!(log[0].action_type, "classify_failed");
}
```

- [x] **Step 2: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test ai_pipeline_test -- --nocapture`
Expected: FAIL — `AiPipeline::new`, `enrich_and_store` do not exist.

- [x] **Step 3: Implement `AiPipeline`**

Replace the entire contents of `core/src/ai/pipeline.rs`:

```rust
use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::ai::classifier::Classifier;
use crate::ai::llm::LlmBackend;
use crate::ai::profile::UserProfile;
use crate::ai::rag::build_rag_context;
use crate::error::Result;
use crate::knowledge::Retriever;
use crate::store::Store;
use crate::types::Message;

/// Outcome of processing a single message.
#[derive(Debug, Clone, Copy)]
pub struct EnrichOutcome {
    /// True when the LLM succeeded and a priority + category were attached.
    /// False when classification failed and the message was stored in
    /// degraded (priority = None, category = None) form.
    pub classified: bool,
}

/// Top-level AI pipeline.
///
/// Holds the pieces the classifier needs (`LlmBackend`, optional
/// `Retriever`, `UserProfile`) and exposes a single `enrich_and_store`
/// entry point that the channel adapter manager calls for every
/// incoming normalized `Message`.
///
/// The pipeline is `Clone` via the inner `Arc`s so it can be passed
/// into the `AdapterManager::on_messages` callback closure without
/// ownership gymnastics.
#[derive(Clone)]
pub struct AiPipeline {
    classifier: Arc<Classifier>,
    retriever: Option<Arc<Retriever>>,
    profile: Arc<UserProfile>,
}

impl AiPipeline {
    pub fn new(
        llm: Arc<dyn LlmBackend>,
        retriever: Option<Arc<Retriever>>,
        profile: UserProfile,
    ) -> Self {
        Self {
            classifier: Arc::new(Classifier::new(llm)),
            retriever,
            profile: Arc::new(profile),
        }
    }

    /// Classify a message, attach `priority` + `category`, persist via
    /// `Store::insert_message`, and log the decision to `action_log`.
    ///
    /// `sender_address` and `sender_name` are passed through rather than
    /// re-resolved from the store because the adapter manager has already
    /// done that lookup to produce the `Message::sender_id`.
    ///
    /// Graceful degradation: if classification fails for any reason
    /// (LLM down, parse error, bad output), the message is stored
    /// without a priority/category and a `classify_failed` row is
    /// written to the log. The outer `Result` only returns `Err` for
    /// storage failures (which are unrecoverable).
    pub async fn enrich_and_store(
        &self,
        store: &Store,
        mut msg: Message,
        sender_address: &str,
        sender_name: &str,
    ) -> Result<EnrichOutcome> {
        let subject = msg.content.subject.clone().unwrap_or_default();
        let body = msg.content.text.clone().unwrap_or_default();

        let rag = build_rag_context(
            store,
            self.retriever.as_ref(),
            &self.profile,
            msg.channel,
            sender_address,
            &subject,
            &body,
        )?;

        let classification_result = self
            .classifier
            .classify(
                msg.channel,
                sender_name,
                sender_address,
                &subject,
                &body,
                &rag,
            )
            .await;

        let message_id_str = msg.id.to_string();

        match classification_result {
            Ok(classification) => {
                msg.priority = Some(classification.priority);
                msg.category = Some(classification.category.as_str().to_string());
                store.insert_message(&msg)?;
                // Confidence score: we don't yet expose model log-probs; use
                // 1.0 for parsed successes. Plan 5 can refine this when cloud
                // tier exposes confidence.
                store.log_ai_decision(
                    "classify",
                    "message",
                    &message_id_str,
                    &classification.reasoning,
                    1.0,
                )?;
                info!(
                    message_id = %message_id_str,
                    priority = classification.priority.value(),
                    category = classification.category.as_str(),
                    "message classified and stored"
                );
                Ok(EnrichOutcome { classified: true })
            }
            Err(e) => {
                // Degraded mode: store the message without priority and log
                // the failure so the UI can offer a retry.
                store.insert_message(&msg)?;
                let reason = format!("classification failed: {}", e);
                if let Err(log_err) = store.log_ai_decision(
                    "classify_failed",
                    "message",
                    &message_id_str,
                    &reason,
                    0.0,
                ) {
                    warn!(error = %log_err, "failed to log classification failure");
                }
                debug!(
                    message_id = %message_id_str,
                    error = %e,
                    "classification failed; stored in degraded mode"
                );
                Ok(EnrichOutcome { classified: false })
            }
        }
    }
}
```

- [x] **Step 4: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test ai_pipeline_test -- --nocapture`
Expected: 3 passing tests.

- [x] **Step 5: Commit**

```bash
git add core/src/ai/pipeline.rs core/tests/ai_pipeline_test.rs
git commit -m "feat(ai): add AiPipeline with graceful degradation on LLM failure"
```

---

### Task 10: Final Integration Check — Full Test Suite + Real Ollama Smoke Test

**Files:**
- Create: `core/tests/ai_ollama_integration_test.rs`

`★ Why this matters:` The nine tasks above ran entirely against mocked `LlmBackend`s — the code has never actually spoken to Ollama. This final task adds an `#[ignore]`d integration test that talks to a real Ollama endpoint so a human operator can verify the wiring before shipping. It's marked `#[ignore]` so it doesn't run in default `cargo test` (it requires Ollama installed + a model pulled).

- [x] **Step 1: Create the real-ollama smoke test**

Create `core/tests/ai_ollama_integration_test.rs`:

```rust
//! Smoke test against a real local Ollama instance.
//!
//! Runs only when explicitly requested:
//!     cargo test -p messagehub-core --test ai_ollama_integration_test -- --ignored --nocapture
//!
//! Requires:
//! - Ollama running at http://127.0.0.1:11434
//! - A model named in MESSAGEHUB_TEST_MODEL (default: "phi3:mini") pulled

use messagehub_core::ai::{LlmBackend, OllamaLlm};

fn model_name() -> String {
    std::env::var("MESSAGEHUB_TEST_MODEL").unwrap_or_else(|_| "phi3:mini".to_string())
}

#[tokio::test]
#[ignore = "requires a running Ollama with a model pulled"]
async fn test_real_ollama_health_check() {
    let llm = OllamaLlm::new("http://127.0.0.1:11434".to_string(), model_name());
    assert!(
        llm.health_check().await.unwrap(),
        "Ollama is not responding at http://127.0.0.1:11434/api/tags"
    );
}

#[tokio::test]
#[ignore = "requires a running Ollama with a model pulled"]
async fn test_real_ollama_complete_returns_non_empty() {
    let llm = OllamaLlm::new("http://127.0.0.1:11434".to_string(), model_name());
    let out = llm
        .complete(
            "You are a helpful assistant.",
            "Say the word hello and nothing else.",
            32,
        )
        .await
        .unwrap();
    assert!(!out.trim().is_empty(), "ollama returned empty response");
}

#[tokio::test]
#[ignore = "requires a running Ollama with a model pulled"]
async fn test_real_ollama_can_classify_simple_message() {
    use messagehub_core::ai::prompts::{
        CLASSIFICATION_SYSTEM_PROMPT, build_classification_user_prompt,
        parse_classification_response,
    };
    use messagehub_core::ai::RagContext;
    use messagehub_core::types::Channel;

    let llm = OllamaLlm::new("http://127.0.0.1:11434".to_string(), model_name());
    let ctx = RagContext {
        sender_name: None,
        sender_vault_path: None,
        topic_chunks: vec![],
        user_profile_content: String::new(),
    };
    let prompt = build_classification_user_prompt(
        Channel::Email,
        "Newsletter Bot",
        "news@promo.example",
        "50% off this weekend only!",
        "Click here to save big.",
        &ctx,
    );
    let raw = llm
        .complete(CLASSIFICATION_SYSTEM_PROMPT, &prompt, 256)
        .await
        .unwrap();

    // We don't assert an exact classification — small models vary.
    // We DO assert the response parses: if the prompt pipeline is
    // well-formed, the model stays on-schema.
    let parsed = parse_classification_response(&raw);
    assert!(
        parsed.is_ok(),
        "real Ollama response did not parse: {:?} — raw: {}",
        parsed,
        raw
    );
}
```

- [x] **Step 2: Run the full mocked test suite to verify nothing regressed**

Run: `cargo test -p messagehub-core`
Expected: ALL tests pass except those marked `#[ignore]` (embedder tests, ollama integration tests).

- [x] **Step 3: (Optional, human operator only) Run the real-Ollama smoke tests**

This is a manual verification step — run only if you have Ollama installed and `phi3:mini` pulled.

```bash
ollama pull phi3:mini
cargo test -p messagehub-core --test ai_ollama_integration_test -- --ignored --nocapture
```
Expected: 3 passing tests. The classification smoke test's output shows a successful JSON parse from the real model.

If the tests fail, the failure mode is probably one of:
- Ollama not running → `test_real_ollama_health_check` fails first, clearly.
- Model not pulled → `complete` returns a 404-ish error.
- Model too small to honor JSON schema → `parse_classification_response` returns Err. Try a larger model via `MESSAGEHUB_TEST_MODEL=llama3.2:3b`.

- [x] **Step 4: Commit**

```bash
git add core/tests/ai_ollama_integration_test.rs
git commit -m "test(ai): add ignored integration test against a real Ollama instance"
```

---

## Summary

After completing all 10 tasks, the core crate has:

- **`ai::Category` enum** with seven PARA-derived values (work, personal, finance, family, notifications, newsletters, spam) — validated by strict `from_str` with no fuzzy matching.
- **`ai::llm` trait + `OllamaLlm` HTTP client** posting `stream: false` to `/api/chat`, with a health-check endpoint and 60s timeout. No FFI. Fully mockable via `LlmBackend` trait.
- **`ai::profile::UserProfile` loader** reading a markdown file with a 4000-char budget and truncation marker. Missing file is not an error — the pipeline degrades gracefully.
- **`ai::rag::RagContext` builder** combining sender lookup (`Store::find_vault_person_by_address`), top-5 vault retrieval (`knowledge::Retriever`), and user profile into a single prompt section. Subject is weighted 2x in the retrieval query.
- **`ai::prompts` module** with a strict system prompt (enumerates every category, demands JSON-only output) and a fence-tolerant JSON parser that rejects out-of-range priorities, unknown categories, and missing fields.
- **`ai::Classifier`** — 50 lines of glue wiring `LlmBackend` + prompt + parser into a single async `classify` call.
- **`ai::AiPipeline`** — the public entry point. Runs classification, attaches `priority` + `category` to the `Message`, persists via `Store::insert_message`, and writes an `action_log` row per decision. On any failure (LLM down, parse error, bad output), stores the message with `priority = None` and logs a `classify_failed` row.
- **Migration 003** indexes `action_log` on `(entity_type, entity_id)` for the "Why is this prioritized?" lookup and on `(action_type, created_at)` for audit queries.
- **`Store::log_ai_decision` + `Store::list_ai_decisions_for_entity`** typed helpers for the action log.
- **Full test coverage via mocked `LlmBackend`** — every task has TDD-ordered tests that run without a model download or a live Ollama. One `#[ignore]`d integration file exercises the wiring against a real Ollama for human verification.

**Verification checklist** — after all tasks complete, run:

```bash
cargo test -p messagehub-core
```

Expected: all tests pass except the `#[ignore]`d ones (embedder + ollama integration).

**Next plan:** Plan 5 (Tier 2 — Cloud Actions) will add opt-in user-triggered actions using the same `rag::RagContext` builder: `summarize_thread`, `draft_reply`, and `smart_search` via Claude API. It will introduce a `CloudProvider` trait (Anthropic/OpenAI-compatible), entity redaction for privacy, and surface `confidence_score` on drafts so the architectural hook for semi-autonomous mode is wired end-to-end.
