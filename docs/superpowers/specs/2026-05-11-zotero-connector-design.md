# zotero-connector — Design

**Date:** 2026-05-11
**Status:** Approved (design phase)
**Author:** rjl

## 1. Overview

`zotero-connector` is a Rust binary (`zotero-mcp`) that exposes the user's local Zotero library to Claude via the Model Context Protocol. It runs as a subprocess of Claude Code over stdio. Reads come straight from Zotero's local SQLite database and the filesystem for maximum speed; writes go through Zotero's Local Web API so that Zotero remains the single source of truth for mutation, version counters, and sync invariants.

The connector is the structured-access layer for Zotero. Semantic search, embeddings, and any Obsidian wiki integration live in separate components that may consume this MCP, but are not built into it.

### 1.1 Goals

- Fast, low-latency read access to library metadata, full text, and attachments
- Safe writes for notes, metadata edits, and tags via the Local Zotero API
- First-class support for citation keys (BetterBibTeX) and formatted citations (CSL styles)
- A specific feature path for **metadata enrichment** — finding stub items, looking up missing fields against external scholarly sources, and applying changes with optional confidence-gated auto-apply
- First-class support for **webpage items** including saved HTML snapshots and live re-fetch

### 1.2 Non-goals (v1)

- Embeddings, vector search, or semantic ranking
- Obsidian read/write
- File watching, change notifications, or background sync
- A graphical interface or installer
- Headless-browser rendering of JavaScript-heavy webpages

## 2. Verified Environment

Confirmed on the user's machine during design:

- Zotero desktop is the user's primary client; runs continuously during work sessions
- Zotero data directory: `~/Zotero/`
- SQLite database: `~/Zotero/zotero.sqlite` (~26 MB)
- Attachment storage: `~/Zotero/storage/<itemKey>/` (~838 folders)
- Resilio Sync replicates the storage folder to a NAS; local copies remain authoritative for the connector
- **Local Zotero Web API is enabled** at `http://localhost:23119/api/`
- User library ID: `93338` ("My Library")
- One group library present: `richlyon` (id 2)
- BetterBibTeX plugin is installed; JSON-RPC reachable at `http://localhost:23119/better-bibtex/json-rpc`

## 3. Architecture

### 3.1 Boundaries

```
Claude Code
   │  (stdio, MCP JSON-RPC over newline-delimited JSON)
   ▼
zotero-mcp  ── read  ──▶ ~/Zotero/zotero.sqlite          (read-only, WAL-aware)
            ── read  ──▶ ~/Zotero/storage/<key>/<file>   (filesystem)
            ── write ──▶ localhost:23119/api/...         (Local Zotero Web API)
            ── lookup ─▶ localhost:23119/better-bibtex/json-rpc
            ── enrich ─▶ api.crossref.org
                         openlibrary.org
                         export.arxiv.org
                         api.semanticscholar.org
```

### 3.2 Data paths

| Concern | Source | Why |
|---|---|---|
| Search (metadata + full-text) | SQLite read | Zotero already indexes full-text PDF content in the `fulltextItems` / `fulltextWords` tables; querying directly is dramatically faster than paginating the Web API |
| Item metadata | SQLite read | Cheap join across `items`, `itemData`, `itemDataValues`, `itemCreators`, `creators` |
| Collections & tags | SQLite read | Native hierarchy in `collections`; tags in `tags` + `itemTags` |
| Annotations | SQLite read | Zotero stores highlights in `itemAnnotations` |
| Attachment file resolution | SQLite + filesystem | `itemAttachments` row → `~/Zotero/storage/<key>/<filename>` |
| PDF extracted text | SQLite (preferred), fallback to live extraction | Zotero's full-text index lives in `fulltextItemWords` keyed by item; if absent, fall back to extracting on the fly |
| Citation formatting | Local Zotero API | `/api/users/93338/items/<key>?format=bib&style=<csl>` |
| BBT citation key lookup | BetterBibTeX JSON-RPC | `item.citationkey` method |
| Item writes (metadata, notes, tags) | Local Zotero API | Zotero handles version counters, search-index updates, sync invariants |
| External metadata lookups | Public scholarly APIs | CrossRef, OpenLibrary, arXiv, Semantic Scholar |

### 3.3 Failure model

- **Zotero not running and write requested** → fail with structured error message identifying which tool needs Zotero running.
- **SQLite locked despite WAL** → retry with exponential backoff, max 3 attempts, then fail with a clear error.
- **Schema version mismatch** → on startup, read Zotero's `version` table; if the schema version is outside the tested range (range to be pinned during implementation based on the current installed Zotero version), refuse to start with a precise error naming the expected and found versions.
- **External API failure** → return partial enrichment results with a note of which sources failed rather than hard-failing the whole operation.
- **PDF text extraction failure** → return error but never crash; full-text search still works for items with indexed text.

## 4. MCP Surface

### 4.1 Tools — search and retrieve

| Tool | Purpose | Key params |
|---|---|---|
| `search_items` | Combined metadata + full-text search | `query`, optional `fields`, `collection`, `tag`, `item_type`, `limit`, `offset` |
| `get_item` | Full metadata for one item, by Zotero key OR BBT citation key | `item_key` OR `citation_key` |
| `list_collections` | Collection tree | optional `parent` |
| `list_tags` | All tags with usage counts | optional `prefix` |
| `list_recent_items` | Recently added or modified | `sort_by`, `limit` |

The `get_item` response includes a computed `recommended_content_tool` field (`"get_pdf_text"`, `"get_webpage_content"`, or `null`) so callers do not have to dispatch on item type.

### 4.2 Tools — attachments and content

| Tool | Purpose | Key params |
|---|---|---|
| `list_attachments` | List attachments for an item; distinguishes `application/pdf` from `text/html` snapshots clearly | `item_key` |
| `get_pdf_path` | Resolve a PDF attachment to its absolute filesystem path | `item_key` or `attachment_key` |
| `get_pdf_text` | Full extracted text; returns `source: "zotero_index" \| "live_extract"` | `item_key`, optional `page_range` |
| `get_pdf_first_pages` | First N pages of extracted text — convenience for enrichment | `item_key`, `n` (default 2) |
| `list_annotations` | Highlights and notes from Zotero's PDF reader | `item_key` |
| `get_webpage_content` | Clean article text from a webpage item | `item_key`, `mode: "snapshot" \| "live" \| "auto"` (default `"auto"`) |
| `refetch_url` | Fetch the URL live; optionally save back to Zotero as a new snapshot attachment | `item_key`, `save_as_snapshot: bool` |

### 4.3 Tools — citations

| Tool | Purpose | Key params |
|---|---|---|
| `format_citation` | Single formatted citation | `item_key` OR `citation_key`, `style` (e.g. `apa`, `chicago-author-date`, `bibtex`, `csl-json`) |
| `format_bibliography` | Many items, one style | `item_keys[]` or `citation_keys[]`, `style` |

Both delegate to the Local API's `format=` parameter where possible. BBT citation keys are first-class everywhere — every read tool that returns an item includes its citation key, and every tool that accepts an item identifier accepts either form.

### 4.4 Tools — writes

| Tool | Purpose | Key params |
|---|---|---|
| `add_note` | Attach a Markdown note to an item (converted server-side) | `item_key`, `markdown` |
| `update_item_fields` | Patch specific metadata fields | `item_key`, `fields` map |
| `add_tags` / `remove_tags` | Tag mutations | `item_key`, `tags[]` |
| `add_to_collection` / `remove_from_collection` | Collection membership | `item_key`, `collection_key` |

### 4.5 Tools — metadata enrichment

The enrichment subsystem exposes both primitives and a composite, per the agreed design (primitives never auto-apply; composite enforces a confidence threshold).

**Primitives:**

| Tool | Purpose |
|---|---|
| `find_weak_metadata_items` | Heuristic scan: items missing DOI/abstract/publisher/year, items whose title equals the attached filename, items with very short titles. Returns a ranked list with per-item reasons. |
| `lookup_doi` | CrossRef → normalized Zotero-schema fields |
| `lookup_isbn` | OpenLibrary → normalized fields |
| `lookup_arxiv` | arXiv API → normalized fields |
| `search_crossref` | Title + author query |
| `search_semantic_scholar` | Title + author query |
| `propose_metadata_update` | Takes `item_key` + candidate fields; returns a diff (current vs proposed). Never writes. |
| `apply_metadata_update` | Commits a diff via Local API. Idempotent on Zotero's item version. |

**Composite:**

| Tool | Purpose |
|---|---|
| `enrich_item` | Internally runs lookup chain, scores confidence, then either auto-applies (if `confidence ≥ auto_apply_threshold`) or returns the proposed diff with `needs_review: true`. Params: `item_key`, optional `auto_apply_threshold` (default from config, typically `0.9`). |

**Confidence scoring** combines:

- DOI verified by appearing in the PDF's first-page text (strong positive)
- Title fuzzy match score (token-overlap ≥ 0.9 → strong; 0.7–0.9 → weak; < 0.7 → reject)
- First-author surname match (boolean)
- Year match within ±1 year (boolean)
- Cross-source agreement (e.g., CrossRef + Semantic Scholar agree)

If any signal is weak or sources disagree, the composite returns the diff and `needs_review: true` regardless of overall score.

### 4.6 MCP Resources

Collections and tags are exposed as **resources** (browsable hierarchical structure Claude can read directly). All other capabilities are tools. This lets Claude discover library structure without an explicit query and keeps the tool surface focused on actions.

## 5. Internal Structure

### 5.1 Workspace layout

```
zotero-connector/
├── Cargo.toml                       # workspace manifest
├── crates/
│   ├── zotero-core/                 # library: data access + enrichment
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── types.rs             # Item, Attachment, Collection, Tag, Diff, …
│   │       ├── reader/              # SQLite reads
│   │       │   ├── mod.rs
│   │       │   ├── search.rs
│   │       │   ├── items.rs
│   │       │   ├── attachments.rs
│   │       │   ├── collections.rs
│   │       │   ├── tags.rs
│   │       │   ├── fulltext.rs
│   │       │   └── annotations.rs
│   │       ├── writer/              # Local API writes
│   │       │   ├── mod.rs
│   │       │   ├── items.rs
│   │       │   ├── notes.rs
│   │       │   └── tags.rs
│   │       ├── citations.rs
│   │       ├── bbt.rs               # BetterBibTeX JSON-RPC client
│   │       ├── pdf.rs               # text extraction fallback
│   │       ├── web.rs               # snapshot + live HTML → clean text
│   │       ├── enrichment/
│   │       │   ├── mod.rs
│   │       │   ├── crossref.rs
│   │       │   ├── openlibrary.rs
│   │       │   ├── arxiv.rs
│   │       │   ├── semantic_scholar.rs
│   │       │   ├── pdf_signals.rs   # DOI/title/author extraction from PDF text
│   │       │   └── scoring.rs       # confidence scoring
│   │       ├── cache.rs             # on-disk cache for external lookups
│   │       ├── config.rs
│   │       └── error.rs
│   └── zotero-mcp/                  # binary: MCP server
│       └── src/
│           ├── main.rs
│           ├── tools/
│           │   ├── mod.rs
│           │   ├── search.rs
│           │   ├── items.rs
│           │   ├── attachments.rs
│           │   ├── citations.rs
│           │   ├── writes.rs
│           │   └── enrichment.rs
│           └── resources/
│               ├── mod.rs
│               ├── collections.rs
│               └── tags.rs
```

Each unit has a clear single purpose, communicates through typed interfaces in `zotero-core::types`, and is independently testable.

### 5.2 Key dependencies

- `rmcp` — official Rust MCP server crate
- `rusqlite` (with `bundled` feature) — no system libsqlite dependency
- `deadpool-sqlite` — async connection pool wrapping `rusqlite`
- `reqwest` — HTTP client for Local API and external sources
- `tokio` — async runtime
- `serde` / `serde_json` — serialization
- `pdf-extract` — fallback PDF text extraction
- `readability` — Mozilla Readability port for clean HTML → article text
- `directories` — platform-correct paths for cache and config
- `tracing` + `tracing-subscriber` — structured logs to stderr/file (never stdout — stdout is the MCP transport)
- `thiserror` + `miette` — typed errors with diagnostic context
- `wiremock` (dev-dependency) — HTTP mocking for tests

## 6. Cross-cutting Concerns

### 6.1 Configuration

TOML file at `~/.config/zotero-mcp/config.toml`, fully optional with sensible defaults.

```toml
# All fields optional. Shown with defaults.

[zotero]
data_dir = "~/Zotero"
local_api_base = "http://localhost:23119"
user_id = 93338
include_group_libraries = true

[enrichment]
auto_apply_threshold = 0.9
sources = ["crossref", "openlibrary", "arxiv", "semantic_scholar"]
cache_ttl_days = 30

[web]
snapshot_cache_ttl_hours = 24
user_agent = "zotero-mcp/0.1"

[paths]
cache_dir   = "~/Library/Caches/zotero-mcp"
log_dir     = "~/Library/Logs"

[logging]
level = "info"
```

### 6.2 Caching

On-disk JSON cache keyed by `(source, query_hash)`. Default TTLs:

- DOI / ISBN / arXiv lookups: 30 days
- CrossRef / Semantic Scholar fuzzy searches: 7 days
- Live webpage fetches: 24 hours

Cache writes are atomic (write-to-temp + rename) so concurrent invocations cannot corrupt entries.

### 6.3 Logging

`tracing` with structured spans. All output to stderr by default, plus optional file logging at `~/Library/Logs/zotero-mcp.log`. **Stdout is reserved exclusively for the MCP JSON-RPC transport.**

### 6.4 SQLite safety

- Open connections with `OpenFlags::SQLITE_OPEN_READ_ONLY`
- Honor Zotero's WAL mode — reads while Zotero is writing are safe
- On startup, read Zotero's `version` table; pin against a known-tested schema version range and refuse to start outside it with a clear error
- No DDL, no PRAGMAs that mutate state

### 6.5 Zotero Local API conventions

- All requests include `Zotero-API-Version: 3` header
- Writes use `If-Unmodified-Since-Version` to be safe against concurrent edits from the Zotero UI; on 412 (version conflict), refresh and surface a clear conflict error
- Bibliography requests use `format=bib&style=<csl-style-id>` for human-readable output, `format=biblatex` or `format=bibtex` for machine output

### 6.6 BetterBibTeX integration

BBT JSON-RPC at `http://localhost:23119/better-bibtex/json-rpc` is used solely to map Zotero item keys ↔ BBT citation keys. If BBT is unavailable at runtime, the connector still works but `citation_key` fields will be `null` in responses and the `citation_key` parameter on lookups will return a "BBT unavailable" error. BBT is therefore a soft dependency.

## 7. Performance Targets

Realistic targets for the user's 800-item library:

| Operation | Target |
|---|---|
| Cold start to first tool response | < 80 ms |
| `search_items` (typical query) with full-text | < 100 ms |
| `get_pdf_text` from Zotero's index | < 30 ms |
| `enrich_item` cache hit | < 50 ms |
| `enrich_item` cache miss (one external call) | ≈ 1.5 s |

Memory footprint: ~20 MB resident steady-state, ~50 MB peak.

## 8. Testing Strategy

- **Unit tests** against a fixture SQLite file in the repo (synthetic Zotero schema with ~10 items, a few attachments, full-text rows, tags, collections). Tests run offline, no Zotero process required.
- **Mock-HTTP tests** with `wiremock` covering Local API writes and external scholarly sources, including error paths (timeouts, 4xx, 5xx, malformed JSON, version conflicts).
- **Property tests** on the enrichment confidence scorer — empty fields, special characters, Unicode in author names, year boundary cases — to ensure scoring never panics and stays inside `[0.0, 1.0]`.
- **One real-Zotero integration test** gated by an env var (`ZOTERO_MCP_LIVE_TEST=1`), executed manually before release. Confirms schema assumptions against the actual local Zotero database.
- Continuous CI runs all but the live test.

## 9. Out of Scope (Future)

These are intentionally not in v1 and are flagged here so the design does not accidentally close doors:

- **Embeddings / semantic search.** Will be its own component, probably its own MCP server. It can consume `get_pdf_text` and `get_webpage_content` from this MCP.
- **Obsidian integration.** Separate concern; this MCP provides stable citation keys and content, which is all an Obsidian-side workflow needs.
- **Headless-browser rendering** for JS-heavy webpages. `refetch_url` only does plain HTTP in v1.
- **Real-time change notifications** (e.g., watch the Zotero DB and push events). Polling-by-tool-call covers v1 use cases.
- **Multi-user / hosted deployment.** The connector is single-user, single-machine. A hosted variant would require auth, sandboxing, and per-user data paths; out of scope.
- **GUI configuration.** Edit `config.toml` directly.

## 10. Open Questions

None at design time. All architectural questions resolved during brainstorming; implementation details (specific SQL queries, exact JSON schemas for each tool's response) will be settled in the implementation plan.
