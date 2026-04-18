# Plan 5: Tier 2 Cloud Actions — Design Specification

**Date:** 2026-04-18
**Status:** Approved (design; implementation plan pending)
**Author:** Jocelyn Moreau + Claude
**Depends on:** Plan 1 (core + storage), Plan 3 (knowledge engine), Plan 4 (local AI classification)

## Goal

Add opt-in, user-triggered cloud AI actions — `summarize_thread`, `draft_reply`, and `smart_search` — on top of the existing `ai::rag` and `knowledge::Retriever` infrastructure. Introduce a `CloudProvider` trait with an Anthropic implementation, an entity `Redactor` for privacy, a heuristic confidence score, and durable storage for drafts.

## Non-goals

- **Multi-provider support.** Only Anthropic in Plan 5. The `CloudProvider` trait leaves the door open for OpenAI-compatible backends in a later plan.
- **Automatic retries / backoff.** Retry is a UI concern (user clicks "regenerate"). Plan 5 surfaces errors cleanly; it does not hide them behind exponential backoff.
- **Semi-autonomous mode.** Plan 5 wires the `confidence` hook so a future plan can act on it, but this plan only reads confidence — nothing automatic happens based on it.
- **OS keychain integration.** The `AnthropicCloud` constructor takes the key as a `String` argument. Where that key comes from (env var, keychain, config file) is the caller's problem.
- **Streaming responses.** All calls are `stream = false`. Drafts and summaries are short; we don't need incremental UI.

## Architecture

### Module Layout

A new `ai/cloud/` subtree sibling to the existing `ai/*` modules. Shares `ai::rag`, `ai::profile`, and `knowledge::Retriever` rather than re-implementing them.

```
core/src/ai/cloud/
├── mod.rs              # re-exports + CloudAction enum + CloudConfig + DraftRecord
├── provider.rs         # CloudProvider trait + AnthropicCloud impl
├── redactor.rs         # Redactor struct + ReverseMap
├── confidence.rs       # derive_confidence(rag, retrieval_sims) -> f32
└── actions/
    ├── mod.rs          # CloudActions facade (the public entry point)
    ├── summarize.rs    # summarize_thread
    ├── draft.rs        # draft_reply
    └── search.rs       # smart_search

core/src/store/
├── drafts.rs           # NEW — insert_draft, list_drafts_for_message, update_draft_output
└── messages.rs         # ADD — list_messages_in_thread

core/migrations/
└── 004_cloud.sql       # ai_drafts table + indexes

core/tests/              # ~7 new integration test files mirroring Plan 4
```

### Public Surface

```rust
// Marker for which cloud action ran (persisted to both ai_drafts and action_log).
pub enum CloudAction { SummarizeThread, DraftReply, SmartSearch }

// Per-call options. Minimal for Plan 5 — the only knob is redaction.
pub struct CloudConfig { pub redact: bool }

// Provider trait. Mirrors LlmBackend from Plan 4 so prompt/parser helpers
// and test doubles work the same way across local and cloud tiers.
#[async_trait]
pub trait CloudProvider: Send + Sync {
    async fn complete(&self, system: &str, user: &str, max_tokens: u32) -> Result<String>;
}

// Anthropic HTTP client. No env-var coupling; key is injected by the caller.
pub struct AnthropicCloud { /* client, api_key, model, base_url */ }
impl AnthropicCloud {
    pub fn new(api_key: String, model: String) -> Self;
    pub fn with_base_url(self, base_url: String) -> Self; // for wiremock tests
    pub async fn health_check(&self) -> Result<bool>;
}

// Returned by every action. `id` is the row id in `ai_drafts`.
pub struct DraftRecord {
    pub id: Uuid,
    pub action: CloudAction,
    pub output: String,
    pub confidence: f32,
    pub created_at: DateTime<Utc>,
}

// Facade the UI / app binary calls. Holds everything the three actions need.
pub struct CloudActions { /* Arc<dyn CloudProvider>, Redactor, Option<Arc<Retriever>>, Arc<UserProfile> */ }
impl CloudActions {
    pub fn new(
        provider: Arc<dyn CloudProvider>,
        retriever: Option<Arc<Retriever>>,
        profile: UserProfile,
    ) -> Self;

    pub async fn summarize_thread(&self, store: &Store, thread_id: Uuid, cfg: CloudConfig)
        -> Result<DraftRecord>;
    pub async fn draft_reply(&self, store: &Store, message_id: Uuid, cfg: CloudConfig)
        -> Result<DraftRecord>;
    pub async fn smart_search(&self, store: &Store, query: &str, cfg: CloudConfig)
        -> Result<DraftRecord>;
}
```

## Data Flow

`draft_reply` is the full path. The other two actions are strict subsets.

```
CloudActions::draft_reply(message_id, CloudConfig { redact: true })

1. store.get_message(message_id)                              → Message
   store.list_messages_in_thread(thread_id, limit=20)         → Vec<Message>  (NEW helper)
   store.find_vault_person_by_address(channel, sender_addr)   → Option<(name, path)>

2. build_rag_context(...)                                     → RagContext
   (reuses ai::rag::build_rag_context — unchanged from Plan 4)

3. Redactor::redact(&thread_text, &rag_context)               → (redacted_text, ReverseMap)

4. build_draft_prompt(redacted_thread, rag_context, profile)  → (system, user)
   System prompt demands JSON: { "draft": "...", "language": "en|fr|de" }

5. provider.complete(system, user, 1024).await                → raw_json_string

6. parse_draft_response(raw)                                  → ParsedDraft

7. Redactor::un_redact(parsed.draft, &reverse_map)            → final_draft

8. confidence = derive_confidence(&rag_context, &retrieval_sims)

9. store.insert_draft(message_id, "draft_reply", redacted_preview,
                      final_draft, confidence, "anthropic", model)
   store.log_ai_decision("draft_reply", "message", message_id,
                         reasoning, confidence)

return DraftRecord { id, action: DraftReply, output: final_draft, confidence, created_at }
```

**`summarize_thread`** — identical, minus the `list_messages_in_thread` fetch detail (needs it for the summary input itself). Un-redacts the summary before storing.

**`smart_search`** — no thread; step 1 is `Retriever::search(query, top_k=10)`, step 3 only redacts the *query* (retrieved chunks never left the device). Output is a natural-language answer that cites vault file paths.

**Invariant:** redaction happens before step 5 (the network call); un-redaction happens after step 6 (the parse). Nothing between those two points can observe un-redacted content.

## Components

### CloudProvider & AnthropicCloud

Mirrors Plan 4's `LlmBackend` so test doubles and prompt helpers compose the same way. The concrete `AnthropicCloud` hits `POST /v1/messages` on `https://api.anthropic.com` with:

- `x-api-key` header
- `anthropic-version: 2023-06-01`
- `Content-Type: application/json`
- `stream: false`
- `max_tokens` configurable per call

Parses the `content` array from the response, joining the `type: "text"` blocks into a single string. Non-text blocks (tool use, etc.) are dropped — not expected for Plan 5 actions.

`health_check()` hits `GET /v1/models` (no model list is actually used; only checks for 2xx + valid key). Returns `Ok(false)` on any network error — never throws for "unreachable" (same pattern as Plan 4's `OllamaLlm::health_check`).

**Errors:**
- 4xx from the API → `CoreError::Cloud("anthropic returned 4xx: ...")`
- 5xx / timeout → `CoreError::Cloud("anthropic service unavailable: ...")`
- Response body doesn't match schema → `CoreError::Cloud("anthropic response body malformed: ...")`

Timeout: 60s (same as `OllamaLlm`). No automatic retries.

### Redactor

Three entity classes, applied longest-match-first:

1. **Vault-matched names.** At construction, `Redactor::build(store)` loads `Store::list_vault_people()` and builds a case-insensitive trie `{ name → token_seed }`. A longest-match scan over the input replaces each hit with `[PERSON_<n>]`.
2. **Email addresses.** Regex `[\w.+-]+@[\w-]+\.[\w.-]+` → `[EMAIL_<n>]`.
3. **Phone numbers.** Regex `\+?\d[\d\s\-().]{7,}\d` (min 9 chars) → `[PHONE_<n>]`. Conservative to avoid catching order numbers / SKUs.

Same entity → same token across one call (stable numbering). Different calls get fresh maps.

**Vault refresh:** `Redactor` loads vault people once at construction. A new person added to `05-People/` after construction is invisible until the app restarts (or until Plan 3's file watcher is wired to call `Redactor::refresh(store)` — deferred to a later plan). Practical impact: a brand-new contact added mid-session won't be scrubbed until restart. Acceptable for Plan 5; noted as a known limitation.

**Return type:** `(String, ReverseMap)` where `ReverseMap = HashMap<String, String>` maps tokens back to originals. `un_redact(text, &map)` is a straight find-and-replace.

**What redaction does NOT do:**
- Doesn't touch the system prompt (no user data there).
- Doesn't touch `user_profile_content` — the user authored it and expects it to flow through.
- Doesn't alter text outside tokens; surrounding context goes to the cloud verbatim.

**Un-redaction policy:** always applied to the final user-visible output (`draft_reply`, `summarize_thread`, `smart_search`). We never store un-redacted text in the `input_redacted` audit column.

### derive_confidence

```rust
fn derive_confidence(rag: &RagContext, retrieval_sims: &[f32]) -> f32 {
    let top_sim = retrieval_sims.iter().cloned().fold(0.0_f32, f32::max);
    let sender_signal = if rag.sender_name.is_some() { 1.0 } else { 0.7 };
    let profile_signal = if !rag.user_profile_content.trim().is_empty() { 1.0 } else { 0.8 };
    (top_sim * sender_signal * profile_signal).clamp(0.0, 1.0)
}
```

- `top_sim`: best cosine similarity from the retriever (already 0.0-1.0). For `summarize_thread` (no retrieval) pass `&[0.85]` as a baseline — the grounding is the thread itself.
- `sender_signal`: known vault contact = 1.0, unknown = 0.7.
- `profile_signal`: profile present = 1.0, absent = 0.8.

Property: zero retrieval + unknown sender → 0.0 (honest).

### Action modules

Each of `summarize.rs`, `draft.rs`, `search.rs` exports one async function with the signature `(&CloudActions, &Store, <action-specific inputs>) -> Result<DraftRecord>`. Each owns its system prompt + JSON schema + response parser. All three share:

- `build_rag_context` from `ai::rag`
- `Redactor::redact` / `un_redact`
- `derive_confidence`
- `provider.complete`
- `store.insert_draft` + `store.log_ai_decision`

## Storage

### Migration 004_cloud.sql

```sql
CREATE TABLE ai_drafts (
    id                 TEXT PRIMARY KEY,            -- UUID
    message_id         TEXT,                        -- FK messages for summarize/draft;
                                                    -- NULL for smart_search (no anchor message)
    action_type        TEXT NOT NULL CHECK (action_type IN
                          ('summarize_thread', 'draft_reply', 'smart_search')),
    input_redacted     TEXT NOT NULL,               -- bytes that left the device (audit)
    output             TEXT NOT NULL,               -- un-redacted response shown to user
    user_edited_output TEXT,                        -- populated when user saves edits
    confidence         REAL NOT NULL,
    provider           TEXT NOT NULL,               -- 'anthropic'
    model              TEXT NOT NULL,
    created_at         TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_ai_drafts_message ON ai_drafts(message_id, created_at DESC);
CREATE INDEX idx_ai_drafts_action  ON ai_drafts(action_type, created_at DESC);
```

**Companion `action_log` row** is written for every call (success or failure) using the Plan 4 `Store::log_ai_decision` helper, so audit-by-entity (`"why is this summarized?"`) works uniformly across local and cloud tiers.

### Helpers added to `Store`

```rust
// core/src/store/messages.rs
pub fn list_messages_in_thread(&self, thread_id: Uuid, limit: u32)
    -> Result<Vec<Message>>;

// core/src/store/drafts.rs (NEW)
pub fn insert_draft(&self, draft: &NewDraft) -> Result<Uuid>;
pub fn list_drafts_for_message(&self, message_id: Uuid) -> Result<Vec<DraftRecord>>;
pub fn update_draft_output(&self, draft_id: Uuid, edited: &str) -> Result<()>;
```

## Error Handling

Three error classes, all surfaced as `CoreError::Cloud(String)`:

1. **Network/provider errors** — 4xx, 5xx, timeouts. Not retried automatically.
2. **Parse errors** — provider response doesn't match the action's JSON schema. Strict rejection (no fuzzy recovery), matching Plan 4's `parse_classification_response` conventions.
3. **Storage errors** — `CoreError::Database` propagates unchanged. A failed `insert_draft` does NOT fail the whole call: the draft is still returned to the caller, a warning is logged, and the user can re-save. Rationale: the user-visible draft is what matters; audit persistence is best-effort.

**Degradation parity with Plan 4:** A cloud call that fails before the storage step writes a `{action}_failed` row to `action_log` with the error string in `reasoning`, so the UI can offer retry the same way it does for local classification failures.

## Testing Strategy

| Layer | Test type | Tooling |
|-------|-----------|---------|
| `Redactor` | unit (strings in → strings + reverse map out) | none |
| `derive_confidence` | unit (table-driven) | none |
| `parse_*_response` (three actions) | unit | none |
| `AnthropicCloud::complete` | integration with mocked HTTP | `wiremock` (already a dev-dep) |
| Each action module | integration with `ScriptedCloudProvider` | trait double |
| `CloudActions` facade | end-to-end with in-memory `Store` + scripted provider | existing `Store::open_in_memory` |
| Real-Anthropic smoke | `#[ignore]`d, reads `ANTHROPIC_API_KEY` from env | gated behind env var |

**Coverage target:** ~40-50 new tests. All but the `#[ignore]`d smoke run offline.

## Task Breakdown

10 tasks, mirroring Plan 4's structure so progress is visible and each commit is bisectable.

1. **Schema + error + module skeleton.** Migration 004; `CoreError::Cloud` variant; `ai/cloud/mod.rs` with `CloudAction`, `CloudConfig`, `DraftRecord` public types; stubs for the sub-modules so the crate compiles.
2. **`Store::list_messages_in_thread`.** Helper + integration tests.
3. **`Store` draft helpers.** `insert_draft`, `list_drafts_for_message`, `update_draft_output` + integration tests.
4. **`CloudProvider` trait + `AnthropicCloud`.** HTTP client, health check, wiremock tests for happy path, 4xx, 5xx, malformed body.
5. **`Redactor`.** Vault-trie + email/phone regex + reverse-map + `un_redact`; unit tests for each entity class and round-trip.
6. **`derive_confidence` helper.** Table-driven tests covering known/unknown sender × with/without profile × zero/full retrieval.
7. **`summarize_thread` action.** Prompt + strict parser + orchestrator; integration tests with `ScriptedCloudProvider`.
8. **`draft_reply` action.** Prompt + language detection (emits `language: en|fr|de`) + orchestrator; tests.
9. **`smart_search` action.** Retriever wiring + prompt + orchestrator; tests (including the "redact the query" path).
10. **`CloudActions` facade + real-Anthropic smoke.** Wire everything; `#[ignore]`d smoke test that reads `ANTHROPIC_API_KEY` from env. Full `cargo test -p messagehub-core` passes.

## Open Questions

None blocking. Deferred to later plans:

- How does the UI expose "redact: yes/no" per action? → desktop/mobile plans.
- Where does the API key come from at runtime? → OS keychain integration in a desktop/mobile plan.
- Second provider (OpenAI-compatible) for self-hosted vLLM / LM Studio users. → Plan 6 candidate.
- Streaming responses for long summaries. → Plan 6+.
- "Semi-autonomous" mode that acts on `confidence > 0.9` drafts. → Plan 7+.

## Summary

After Plan 5, the app can (on user request, one action at a time) send a redacted thread or query to Anthropic, receive a JSON-schema-validated response, un-redact it, persist it to `ai_drafts` with a heuristic confidence score, and log the decision to `action_log`. All three Tier 2 actions from the master design ship. The cloud layer shares `ai::rag`, `ai::profile`, and `knowledge::Retriever` with the local tier, reuses Plan 4's `Store::log_ai_decision` for audit parity, and mirrors the `LlmBackend`/`OllamaLlm` test-double pattern for offline testability.
