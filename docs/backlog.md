# Backlog

Issues and improvements queued for future plans. Each entry has a severity,
discovered-during context, and a proposed fix. When work begins, promote the
item to a proper plan under `docs/superpowers/plans/`.

---

## B-001 — LLM timeout path is silent (produces empty string instead of error)

**Severity:** medium (correctness / observability)
**Discovered:** Plan 7a dogfooding, 2026-04-19.
**Component:** `core/src/ai/llm.rs` — `OllamaLlm::complete`.

### Symptom

When the model is too slow and the 60s reqwest timeout fires, the classifier
logs:

```
WARN classifier parse failure raw_preview= error=ai pipeline error: no JSON object found in response: ""
```

— i.e. the classifier treats a timeout as "model returned empty string" and
falls through the JSON-parse error path, which is misleading and loses the
root cause.

### Proposed fix

- In `OllamaLlm::complete`, match on `reqwest::Error::is_timeout()` and return
  a distinct `CoreError::AiTimeout { timeout_secs }` variant (add to
  `core/src/error.rs`).
- In `Classifier`, propagate the timeout variant instead of normalizing it to a
  parse failure so the log makes the failure mode obvious.
- Optionally make the timeout configurable via `AiPipeline::new` parameters
  rather than the hard-coded `Duration::from_secs(60)` in the client builder.

### Tests

- Unit test using `wiremock` that responds with an intentionally long delay;
  assert `CoreError::AiTimeout` comes back.

---

## B-002 — Classifier doesn't strip markdown code fences from LLM output

**Severity:** medium (model compatibility)
**Discovered:** Plan 7a dogfooding, 2026-04-19.
**Component:** `core/src/ai/classifier.rs` — JSON extraction.

### Symptom

Models like `gemma4:latest` wrap JSON output in markdown:

````
```json
{"category": "Finance/Billing", "priority": 3, "reasoning": "..."}
```
````

The classifier's JSON extractor doesn't strip the surrounding fences, so it
fails to find the object and falls back to `category=Unknown, priority=Low`.
Coder-trained models (e.g. `qwen2.5-coder:3b`) avoid the fences, so they work
today — but that's fragile compatibility.

### Proposed fix

- Before calling the JSON parser, strip a leading ` ```json\n` / ` ```\n` prefix
  and a trailing ` \n``` ` suffix from the raw response if present.
- Alternatively: widen the JSON-extraction regex to match `{...}` across newlines
  regardless of surrounding fences.
- Add a trace-level log of the raw response (first 200 chars) when falling into
  the `Unknown` path, so future model-compatibility bugs are diagnosable from
  logs alone.

### Tests

- Table-driven: feed the classifier raw strings with and without fences, assert
  both parse to the same result.
- Include an end-to-end test against a stub `LlmBackend` that always returns a
  fenced response.
