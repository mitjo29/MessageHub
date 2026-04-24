# Message body rendering — design

**Date:** 2026-04-24
**Status:** Design — pending plan
**Scope:** Frontend only. No backend, DTO, or database changes.

## Problem

`desktop/src/components/MessageDetail.tsx:75` renders the message body as:

```tsx
<pre className="detail-body">{detail.body}</pre>
```

Raw plain text in a monospace box. Two concrete pains:

1. **Links show as full URLs.** HTML emails carry anchor text (`<a href="https://...">the Q2 numbers here</a>`) that is discarded when the adapter stores the plain-text branch. Plain-text messages that contain bare URLs show the URL unabridged, often wrapping badly.
2. **Quoted reply history dominates.** `> On Mon, X wrote:…` chains stretch messages into screens of indented quotes; the user's actual reply drowns in its own thread.

The DTO already carries `detail.html` (populated by `core/src/adapters/email.rs` via `mail_parser::body_html(0)`) — the frontend just ignores it.

## Goals

- Render HTML emails as readable, theme-consistent content.
- Show links with their anchor text ("the Q2 numbers here") rather than raw URLs.
- Collapse quoted reply history behind a toggle.
- Keep plain-text-only channels (Telegram, SMS pastes) in the same render path so bare URLs become clickable there too.
- Add zero backend surface area — this is a frontend refactor.

## Non-goals

- No image-tracking-pixel allowlist per sender (manual "Load all" toggle is sufficient for v1).
- No dark-theme support (app is light-only today).
- No localized quote detection ("Le 24 avr. 2026, X a écrit :"); English-only heuristic.
- No Outlook `-----Original Message-----` separator support in v1.
- No backend pre-conversion or storage of markdown. A future B-008-adjacent change may revisit this if cloud drafts benefit from clean markdown input.

## Approach

**One pipeline:**

```
MessageDetail DTO  ──▶  <MessageBody>
  detail.html  ─┐
                ├──▶  htmlToMarkdown(html)  ─┐
  detail.body  ─┘   (turndown + config)      │
                                             ▼
                                      react-markdown
                                   (remark-gfm, remark-breaks)
                                             │
                                             ▼
                               JSX with intercepted <img> and <a>
```

- **HTML-bearing emails** go through turndown to produce markdown, then react-markdown to render.
- **Plain-text-only messages** skip turndown and feed their text directly into react-markdown, which autolinks bare URLs via `remark-gfm`.
- **Fallback:** if turndown throws (malformed HTML), catch and render `detail.body` as plain text. If both `html` and `body` are empty/null, render "(empty body)".

## Libraries added

Added to `desktop/package.json`:

- `turndown` — HTML→markdown converter (mature, standard pick).
- `turndown-plugin-gfm` — adds table support to turndown output.
- `@types/turndown` — TypeScript types.
- `react-markdown` — markdown renderer with component-slot API.
- `remark-gfm` — GitHub-flavored extensions (tables, strikethrough, autolinks).
- `remark-breaks` — treat single newlines as `<br>` (email convention).

Approximate bundle impact: ~150KB gzipped. Conversion is cheap (5–20ms for a typical message); it runs on every detail-open and the cost is imperceptible.

## File changes

### New: `src/lib/htmlToMarkdown.ts`

Thin wrapper around `TurndownService`. Exports `htmlToMarkdown(html: string): string`.

Configuration:
- `headingStyle: "atx"`
- `codeBlockStyle: "fenced"`
- GFM plugin for tables
- Custom rule to flatten Gmail wrapper divs (`div.gmail_quote`, `div.gmail_extra`) so they don't inflate the markdown tree.

### New: `src/lib/detectQuotedBlock.ts`

Pure function. Signature:

```ts
export function detectQuotedBlock(md: string): { visible: string; quoted: string | null };
```

Heuristic (single left-to-right pass):

1. Search for the first line matching `/^On .+ wrote:\s*$/i` that is immediately followed (tolerating one blank line) by a line starting with `>`. Split there.
2. If no match: search for a run of ≥3 consecutive `>`-prefixed lines. Split at its start.
3. If still no match: return `{ visible: md, quoted: null }` and no toggle renders.

Gmail-wrapper `<div class="gmail_quote">` is stripped by the turndown rule before this function runs, so it never appears in the markdown and doesn't need its own case here.

### New: `src/components/MessageBody.tsx`

Props: `detail: MessageDetail`.

**Input dispatch (inside the component, via `useMemo` keyed on `detail.id`):**
- If `detail.html` is non-empty, call `htmlToMarkdown(detail.html)` (inside try/catch — on throw, fall through).
- Else use `detail.body` verbatim.
- If both are empty/null, render "(empty body)" and return.

The resulting markdown string is then passed through `detectQuotedBlock` to split `visible` / `quoted`.

Owns two pieces of local state:
- `loadImages: boolean` (default `false`)
- `showQuoted: boolean` (default `false`)

Both reset via `useEffect` keyed on `detail.id` so switching messages resets view state.

Renders `<ReactMarkdown remarkPlugins={[remarkGfm, remarkBreaks]} components={...}>` with two overrides:

- **`img`:** when `loadImages === false`, renders an inline placeholder with alt text and a single "Load all" button that flips state; when `true`, renders a standard `<img>`. Data URIs (`src` starts with `data:`) bypass the gate (no network).
- **`a`:** when the anchor text equals `href` (autolinked plain-text URL), truncate visible text at 60 chars with ellipsis while keeping the full `href`. Named anchor text is passed through unchanged. All links open via `target="_blank" rel="noopener noreferrer"`.

If `detectQuotedBlock` returns a `quoted` string, render a small button above the body: "Show trimmed content (N lines hidden)". Clicking toggles expansion inline in a muted blockquote.

### Modified: `src/components/MessageDetail.tsx`

Line 75 becomes:

```tsx
<MessageBody detail={detail} />
```

The `.detail-body` class/rules are retired. No other changes.

### Modified: `src/App.css`

- Remove `.detail-body` block.
- Add `.message-body` rules scoped to the component:
  - Typography: 0.95rem / 1.55 line-height, system-font stack (no monospace unless inside code blocks).
  - Headings h1–h3: 1.25rem / 1.1rem / 1rem, bold, tight top margin.
  - `a`: `color: var(--accent)`, underline on hover only.
  - `blockquote`: 4px left border in `var(--border)`, `color: var(--text-secondary)`, padding-left 0.75rem. Used for inline quotes and the expanded trimmed-content block (the latter gets `opacity: 0.85`).
  - Image placeholder: dashed 1px `var(--border)` border, `var(--bg)` background, alt-text plus "Load" button inline-right.
  - `code`/`pre`: muted `var(--bg)` background, 0.85em, mono font.
  - `table`: basic border + zebra rows.

### Modified: `desktop/package.json`

Dependency additions listed above. Lockfile updated by `npm install`.

## Testing

**Unit tests (Vitest):**

- `htmlToMarkdown` fixtures:
  - Simple paragraph with anchor → `[text](url)`
  - HTML table → GFM pipe table
  - HTML unordered list → markdown bullets
  - `<div class="gmail_quote">…</div>` wrapper flattened
  - `<img src alt>` passes through as `![alt](url)`
  - Malformed HTML → returns empty string (not throws)
- `detectQuotedBlock` fixtures (6):
  - Gmail-style "On … wrote:" followed by `>` lines
  - Apple Mail (blockquote converted by turndown to `>` lines)
  - Plain-text `>` run with no "wrote:" preamble
  - Multi-level nested quotes
  - No quote present — `quoted: null`
  - Empty string — `quoted: null`

**Manual UAT checklist** (part of the plan, not automated):
- Plain-text Telegram message: paragraphs preserved, bare URLs clickable, no quote toggle.
- HTML marketing newsletter: images hidden; clicking "Load all" loads them; navigating away and back re-hides.
- Reply thread: quoted history collapsed by default; toggle expands/collapses.
- Message with `<a href="...">Click here</a>`: visible text is "Click here", not the URL.
- Malformed HTML: falls back to plain text without console errors.

## Risk and limitations

- **Heuristic quote detection is English-only.** Non-English locale quote preambles won't match; affected messages show the full history. Acceptable for v1; expand regex later if needed.
- **No Outlook `-----Original Message-----` detection.** Add if users report it.
- **Nested "reply inside quote" not unwound.** The detector collapses from the first quote onward, so your reply embedded inside a quoted block is hidden too. Rare in modern clients (Gmail composes always quote *below*).
- **150KB bundle growth.** Acceptable for a desktop app; Tauri bundles are already multi-MB.
- **Tracking pixels load when user clicks "Load all".** This is deliberate — same gesture as Gmail. Per-sender allowlist is a future feature.
- **Tauri link-opening behavior** needs verification: we set `target="_blank"`, and Tauri's default config opens external URLs in the system browser. Confirm during implementation and add an `allowList` entry if not automatic.

## Out-of-scope follow-ups

- Dark theme styles when dark theme ships.
- Backend pre-conversion / markdown storage (consider alongside B-008 when wiring `UserProfile` into `CloudActions` — cloud drafts would benefit from clean markdown input).
- Localized quote-preamble regexes.
- Per-sender remote-image allowlist persisted in the store.
