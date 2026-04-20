# Backlog

Issues and improvements queued for future plans. Each entry has a severity,
discovered-during context, and a proposed fix. When work begins, promote the
item to a proper plan under `docs/superpowers/plans/`.

*(Empty — no open items at the moment.)*

---

## Resolved

### B-001 — LLM timeout path is silent — **Fixed in `2bbc75e` (2026-04-20)**

Surfaced as empty `raw_preview=` in classifier logs when the model was too
slow. `OllamaLlm::complete` now checks `reqwest::Error::is_timeout()` and
returns `CoreError::AiTimeout { timeout_secs }`. `new_with_timeout()` lets
tests exercise short timeouts without the production 60s wait.

### B-002 — Classifier didn't strip markdown code fences — **Already handled; tests added in `597a738` (2026-04-20)**

Investigation during burn-down found `strip_fences` already existed in
`core/src/ai/prompts.rs` and was wired into the classifier parse path — the
reported issue was unreproducible. Added unit tests (plain fence, `json`
fence, passthrough, trailing whitespace) plus an end-to-end classifier test
to lock in the behavior and catch future regressions. The user's original
empty-response symptom was purely B-001 (timeout), not fence-parsing.
