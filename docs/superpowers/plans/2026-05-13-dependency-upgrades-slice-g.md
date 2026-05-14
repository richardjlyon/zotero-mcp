# Dependency Upgrades — Slice G Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace 22 `Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))` sites across `crates/zotero-mcp/src/tools/{search,attachments,enrichment}.rs` with `Ok(Json(r))`, so the `#[tool]` macro auto-emits `outputSchema` entries on `tools/list` responses. Lands as one atomic commit on `main`.

**Architecture:** Compile-error-driven port across 9 files. First pass adds `JsonSchema` derives to ~18 core types (mechanical). Second pass introduces three new output structs (`CreateItemResult`, `AttachmentResult`, `WeakMetadataItem`) and changes `find_weak_metadata_items`'s return type. Third pass migrates each `tools/*.rs` file's 22 `_t` functions to `Result<Json<T>, Error>`, with matching `server.rs` wrapper signature changes done together so the build stays green between files. Final pass extends the existing smoke test with `output_schema` assertions.

**Tech Stack:** Rust, Cargo. rmcp 1.7's `rmcp::Json<T>` return wrapper (`rmcp::handler::server::wrapper::Json`). schemars 1.x for the `JsonSchema` derive.

**Spec:** `docs/superpowers/specs/2026-05-13-dependency-upgrades-slice-g-design.md` (commit `58a1b25`).

---

## File Structure

| File | Purpose | Action |
|---|---|---|
| `crates/zotero-mcp/src/core/types.rs` | 12 structs/enums get `JsonSchema` | Modify (12 derive lines + `use schemars::JsonSchema;` if absent) |
| `crates/zotero-mcp/src/core/pdf.rs` | 2 types get `JsonSchema` | Modify (2 derive lines + import) |
| `crates/zotero-mcp/src/core/web.rs` | 3 types get `JsonSchema` | Modify (3 derive lines + import) |
| `crates/zotero-mcp/src/core/enrichment/mod.rs` | `NormalizedRecord` gets `JsonSchema` | Modify (1 derive line + import) |
| `crates/zotero-mcp/src/core/enrichment/propose.rs` | NEW `WeakMetadataItem` struct; `find_weak_metadata_items` return-type change | Modify |
| `crates/zotero-mcp/src/tools/search.rs` | 5 `_t` functions migrate to `Result<Json<T>, Error>` | Modify |
| `crates/zotero-mcp/src/tools/attachments.rs` | 9 `_t` functions migrate; NEW `CreateItemResult` + `AttachmentResult` structs | Modify |
| `crates/zotero-mcp/src/tools/enrichment.rs` | 8 `_t` functions migrate (3 use `Json<Value>` for lookup_*); `find_weak_metadata_items_t` shape change | Modify |
| `crates/zotero-mcp/src/server.rs` | 22 wrapper signatures change `Result<CallToolResult, McpError>` → `Result<Json<OutputT>, McpError>`; smoke test extended | Modify |

9 files total. All changes land in one atomic commit.

---

## Tool → output-type lookup table

The implementer references this when migrating each `_t` function and its `server.rs` wrapper. The "Source" column names where the value comes from inside the `_t` function (useful when tracing what `T` should be in `Json<T>`).

| # | Tool | `Json<T>` where T = | Source value |
|---|---|---|---|
| 1 | `search_items` | `Vec<SearchHit>` | `search_metadata(...).await` returns `Result<Vec<SearchHit>>` |
| 2 | `get_item` | `Item` | `get_item_by_key(...)` + `hydrate_citation_key` mutates in place |
| 3 | `list_collections` | `Vec<Collection>` | `collections::list(...).await` |
| 4 | `list_tags` | `Vec<Tag>` | `tags::list(...).await` |
| 5 | `list_recent_items` | `Vec<SearchHit>` | `recent::list(...).await` returns `Result<Vec<SearchHit>>` |
| 6 | `list_attachments` | `Vec<Attachment>` | `list_attachments(...).await` |
| 7 | `get_pdf_text` | `PdfTextResult` | `get_pdf_text(...).await` |
| 8 | `get_pdf_first_pages` | `PdfTextResult` | `get_pdf_first_pages(...).await` |
| 9 | `list_annotations` | `Vec<Annotation>` | `list_annotations(...).await` |
| 10 | `get_webpage_content` | `WebContentResult` | `get_webpage_content(...).await` |
| 11 | `refetch_url` | `RefetchResult` | `refetch_url(...).await` |
| 12 | `create_item` | `CreateItemResult` (NEW) | `create_item(...).await` returns `(key, version)` tuple → wrap in `CreateItemResult { item_key: key, version }` |
| 13 | `attach_link` | `AttachmentResult` (NEW) | `attach_link(...).await` returns `String` (the key) → wrap in `AttachmentResult { attachment_key: key }` |
| 14 | `attach_file` | `AttachmentResult` (NEW) | `attach_file(...).await` returns `String` (the key) → wrap in `AttachmentResult { attachment_key: key }` |
| 15 | `find_weak_metadata_items` | `Vec<WeakMetadataItem>` (NEW) | `find_weak_metadata_items(...).await` — return type changes to `Result<Vec<WeakMetadataItem>>` (see Task 2) |
| 16 | `lookup_doi` | `serde_json::Value` | `render_record(&r, &a.format)` returns `Result<Value, Error>` |
| 17 | `lookup_isbn` | `serde_json::Value` | `render_record(&r, &a.format)` |
| 18 | `lookup_arxiv` | `serde_json::Value` | `render_record(&r, &a.format)` |
| 19 | `search_crossref` | `Vec<NormalizedRecord>` | `s.crossref.search(...).await` |
| 20 | `search_semantic_scholar` | `Vec<NormalizedRecord>` | `s.semantic_scholar.search(...).await` |
| 21 | `propose_metadata_update` | `EnrichmentProposal` | `propose_metadata_update(...).await` |
| 22 | `enrich_item` | `EnrichmentProposal` | `enrich_item(...).await` |

**Do NOT migrate** these 12 `Content::text` tools (their signatures stay `Result<CallToolResult, McpError>`):
`ping`, `get_pdf_path`, `format_citation`, `format_bibliography`, `add_note`, `update_item_fields`, `add_tags`, `remove_tags`, `add_to_collection`, `remove_from_collection`, `delete_item`, `apply_metadata_update`.

---

## Pre-flight: confirm clean state

**Files:**
- Read-only: working tree

- [ ] **Step 1: Confirm clean tree on `main`**

Run: `cd /Users/rjl/Code/github/zotero-connector && git status`

Expected: `nothing to commit, working tree clean` and branch `main`.

- [ ] **Step 2: Capture baseline test results**

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo test -p zotero-mcp 2>&1 | grep "^test result:" | sort | uniq -c`

Expected: every line `ok`, no `FAILED`. Lib tests pass at 107 (the Slice F baseline). Write the number down. Slice G's lib-test count adds +1 (the new `output_schemas_emitted_for_json_returning_tools` smoke test). Final expected: 108.

- [ ] **Step 3: Record the pre-flight SHA**

Run: `cd /Users/rjl/Code/github/zotero-connector && git rev-parse HEAD`

Expected: `58a1b25` (the Slice G spec commit). Write down the SHA. Rollback point if the slice escalates.

---

## Task 1: Add `JsonSchema` to 18 core types

**Files (changes staged, NOT committed yet):**
- Modify: `crates/zotero-mcp/src/core/types.rs`
- Modify: `crates/zotero-mcp/src/core/pdf.rs`
- Modify: `crates/zotero-mcp/src/core/web.rs`
- Modify: `crates/zotero-mcp/src/core/enrichment/mod.rs`

The 18 types compile standalone after this task — no other files depend on the new derives yet. `cargo build` should pass at the end of Task 1.

### Step 1.1: `core/types.rs` — add 12 derives

Open `/Users/rjl/Code/github/zotero-connector/crates/zotero-mcp/src/core/types.rs`. At line 1, the file imports `serde::{Deserialize, Serialize}`. Add `schemars::JsonSchema` import:

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
```

Then, for each of the 12 affected types, extend the existing `#[derive(...)]` line to include `JsonSchema`:

| Type | Existing derive | New derive |
|---|---|---|
| `Item` (line 4) | `#[derive(Debug, Clone, Serialize, Deserialize)]` | `#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]` |
| `Creator` (line 24) | same | same pattern |
| `Attachment` (line 34) | same | same pattern |
| `AttachmentLinkMode` (line 44) | `#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]` | `#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]` |
| `Collection` (line 71) | `#[derive(Debug, Clone, Serialize, Deserialize)]` | append `, JsonSchema` |
| `Tag` (line 79) | same | append `, JsonSchema` |
| `Annotation` (line 85) | same | append `, JsonSchema` |
| `SearchHit` (line 97) | same | append `, JsonSchema` |
| `Diff` (line 110) | `#[derive(Debug, Clone, Default, Serialize, Deserialize)]` | append `, JsonSchema` |
| `FieldChange` (line 115) | `#[derive(Debug, Clone, Serialize, Deserialize)]` | append `, JsonSchema` |
| `EnrichmentProposal` (line 122) | same | append `, JsonSchema` |
| `SourceBreakdown` (line 131) | same | append `, JsonSchema` |

### Step 1.2: `core/pdf.rs` — add 2 derives

Open `/Users/rjl/Code/github/zotero-connector/crates/zotero-mcp/src/core/pdf.rs`. Add `use schemars::JsonSchema;` at the top alongside existing serde imports.

For `PdfTextSource` (line 12) and `PdfTextResult` (line 20): extend `#[derive(...)]` to include `JsonSchema`.

### Step 1.3: `core/web.rs` — add 3 derives

Open `/Users/rjl/Code/github/zotero-connector/crates/zotero-mcp/src/core/web.rs`. Add `use schemars::JsonSchema;`.

Three types need `JsonSchema` added to their existing derive line:
- `WebMode` (around line 13 — `pub enum WebMode { Snapshot, Live, Auto }`) — see note below
- `WebSource` (an enum near `WebContentResult`)
- `WebContentResult` (line 17)
- `RefetchResult` (line 117)

**Important about `WebMode`:** the spec mentions `WebContentResult`, `WebSource`, `RefetchResult` (3 types). `WebMode` is not a tool-output type — it's an input enum used by `WebArgs.mode` (the `_t` parses a string into it). Do NOT add `JsonSchema` to `WebMode` unless the build complains. If the build does complain (because some output type references it transitively), add `JsonSchema` to `WebMode` then.

Verify which types are actually in the output graph by reading `WebContentResult` and `RefetchResult` field definitions and following any nested types.

### Step 1.4: `core/enrichment/mod.rs` — add 1 derive

Open `/Users/rjl/Code/github/zotero-connector/crates/zotero-mcp/src/core/enrichment/mod.rs`. Add `use schemars::JsonSchema;`.

For `NormalizedRecord` (line ~19): extend `#[derive(Debug, Clone, Serialize, Deserialize)]` to `#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]`.

`NormalizedRecord.fields: serde_json::Map<String, Value>` — schemars 1.x provides `impl JsonSchema for serde_json::Map<String, Value>` automatically. No per-field annotation needed.

`NormalizedRecord.creators: Vec<Creator>` — `Creator` already gained `JsonSchema` in Step 1.1.

### Step 1.5: Build and verify

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo build -p zotero-mcp 2>&1 | tail -30`

Expected: clean build.

Likely failure modes and fixes:

| Error | Fix |
|---|---|
| `the trait JsonSchema is not implemented for serde_json::Value` | Verify schemars version: `cargo tree -p zotero-mcp \| grep schemars`. Expected `schemars v1.x`. If 0.8 somehow, the upgrade path was different than recorded — escalate. |
| `the trait JsonSchema is not implemented for serde_json::Map<String, Value>` | Same as above — schemars 1.x should provide this; if not, add `#[schemars(with = "serde_json::Value")]` on the offending field. |
| `cannot find macro JsonSchema in this scope` | Missing `use schemars::JsonSchema;` at the file head. Add it. |
| `the trait JsonSchema is not implemented for WebMode` (or similar) | A tool-output type transitively references this enum. Add `JsonSchema` to that enum's derive. |
| Anything else | If the fix needs more than a one-token derive addition or a single field annotation, escalate per the block at the end of the plan. |

---

## Task 2: Introduce `WeakMetadataItem` + change `find_weak_metadata_items` return type

**Files:**
- Modify: `crates/zotero-mcp/src/core/enrichment/propose.rs`

### Step 2.1: Read the current function and its callers

Open `/Users/rjl/Code/github/zotero-connector/crates/zotero-mcp/src/core/enrichment/propose.rs`. Find `find_weak_metadata_items` at line 34. Current signature:

```rust
pub async fn find_weak_metadata_items(
    pool: &ReadOnlyPool,
    library_id: i64,
    limit: i64,
) -> Result<Vec<(String, Vec<String>)>> { ... }
```

Run a grep to identify callers:

```bash
cd /Users/rjl/Code/github/zotero-connector && \
  grep -rn "find_weak_metadata_items" crates/zotero-mcp/src/ crates/zotero-mcp/tests/
```

Expected: callers are (1) the tool layer at `tools/enrichment.rs::find_weak_metadata_items_t` (line 34) and (2) possibly tests in `propose.rs` itself or `tests/`. If any caller outside these expected locations exists, note it — the migration covers it in Task 5.

### Step 2.2: Add the `WeakMetadataItem` struct

At an appropriate location in `propose.rs` (near the existing type imports at the top, but below any `use` statements), add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WeakMetadataItem {
    pub item_key: String,
    pub weak_fields: Vec<String>,
}
```

Add `use schemars::JsonSchema;` at the file head if not already present. Add `use serde::{Deserialize, Serialize};` if not present.

### Step 2.3: Change the function's return type

Modify `find_weak_metadata_items`:

```rust
pub async fn find_weak_metadata_items(
    pool: &ReadOnlyPool,
    library_id: i64,
    limit: i64,
) -> Result<Vec<WeakMetadataItem>> {
    // ... existing body ...
    // Whatever the body currently constructs as Vec<(String, Vec<String>)>,
    // map it into Vec<WeakMetadataItem>:
    //   .map(|(item_key, weak_fields)| WeakMetadataItem { item_key, weak_fields })
    //   .collect()
}
```

The simplest mechanical change: find where the function currently builds its result (a `Vec<(String, Vec<String>)>`) and `.map(|(k, fs)| WeakMetadataItem { item_key: k, weak_fields: fs })` at the end. Read the existing body to find the exact construction site — it's straightforward.

### Step 2.4: Update internal callers identified in Step 2.1

For each caller found, change the destructuring/usage from `(String, Vec<String>)` shape to `WeakMetadataItem` field access (`.item_key`, `.weak_fields`).

If `propose.rs` has unit tests that exercise this function, update their assertions. Expected: tests are minimal or absent here, but verify.

### Step 2.5: Build and verify

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo build -p zotero-mcp 2>&1 | tail -20`

Expected: clean build. If `tools/enrichment.rs::find_weak_metadata_items_t` fails to compile because it destructures the tuple, that's fine — that file gets migrated in Task 5. For now, if a caller other than `find_weak_metadata_items_t` is broken, fix it (don't touch `find_weak_metadata_items_t` yet).

The cleanest sequence: at this stage `find_weak_metadata_items_t` may still compile if it just passes the result through to `serde_json::to_value(&r).unwrap()` (which works for any Serialize). Read the current `_t` to verify. If it does compile (just produces different JSON shape internally), good — Task 5 will migrate it formally.

---

## Task 3: Introduce `CreateItemResult` and `AttachmentResult` in `tools/attachments.rs`

**Files:**
- Modify: `crates/zotero-mcp/src/tools/attachments.rs`

### Step 3.1: Add the two output structs

Open `/Users/rjl/Code/github/zotero-connector/crates/zotero-mcp/src/tools/attachments.rs`. Add the two new structs at a reasonable location (e.g., near the existing `CreateItemArgs` struct around line 131):

```rust
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CreateItemResult {
    pub item_key: String,
    pub version: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AttachmentResult {
    pub attachment_key: String,
}
```

The existing `use schemars::JsonSchema;` import at the top of the file (already present — used by the Args derives) covers these. No new imports needed.

### Step 3.2: Build and verify

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo build -p zotero-mcp 2>&1 | tail -20`

Expected: clean. The two new structs are unused at this point — Rust may emit "struct never used" warnings; ignore them, they'll go away after Task 4/5.

---

## Task 4: Migrate `tools/search.rs` + matching `server.rs` wrappers (5 tools)

**Files:**
- Modify: `crates/zotero-mcp/src/tools/search.rs`
- Modify: `crates/zotero-mcp/src/server.rs` (only the 5 wrappers that route to `search.rs`)

5 tools: `search_items`, `get_item`, `list_collections`, `list_tags`, `list_recent_items`.

### Step 4.1: Update imports in `tools/search.rs`

Open `/Users/rjl/Code/github/zotero-connector/crates/zotero-mcp/src/tools/search.rs`. Current rmcp import (line 3):

```rust
use rmcp::model::{CallToolResult, Content};
```

Replace with (removing the now-unused `CallToolResult` and `Content`, adding `Json`):

```rust
use rmcp::Json;
```

If the build later complains that `CallToolResult` or `Content` are still referenced (they shouldn't be after Step 4.2), restore them in the import. After this slice no `_t` in this file uses them.

The `Item, Collection, Tag, Annotation, SearchHit` types are reached via `core::reader::...`/`core::types`-paths. The current file already imports what it needs (verify: `use crate::core::reader::items::...; use crate::core::reader::search::...; use crate::core::reader::{collections, recent, tags};`). For the new `Json<T>` signatures, `T` is `Vec<SearchHit>` / `Item` / `Vec<Collection>` / `Vec<Tag>` — these need to be in scope. Add (if not already):

```rust
use crate::core::types::{Collection, Item, SearchHit, Tag};
```

Verify the imports already present at top of file and add only the missing ones.

### Step 4.2: Migrate the 5 `_t` functions

For each function below, replace the entire body following the pattern:

**`search_items`:**

```rust
// Before
pub async fn search_items(s: &AppState, a: SearchArgs) -> Result<CallToolResult, Error> {
    let hits = search_metadata(
        &s.pool,
        1,
        SearchParams { /* ... */ },
    )
    .await
    .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&hits).unwrap(),
    )?]))
}

// After
pub async fn search_items(s: &AppState, a: SearchArgs) -> Result<Json<Vec<SearchHit>>, Error> {
    let hits = search_metadata(
        &s.pool,
        1,
        SearchParams { /* ... */ },
    )
    .await
    .map_err(map_err)?;
    Ok(Json(hits))
}
```

**`get_item`** — return type becomes `Result<Json<Item>, Error>`. The function builds `item` (already `Item`), hydrates the citation key in place, then today calls `Content::json(serde_json::to_value(&item).unwrap())`. After: `Ok(Json(item))`. Keep all the error handling on `item_key`/`citation_key` (the `Err(...)` arms return `Error`, which works with `Result<Json<Item>, Error>`).

**`list_collections`** — return type becomes `Result<Json<Vec<Collection>>, Error>`. Last line becomes `Ok(Json(cs))`.

**`list_tags`** — return type becomes `Result<Json<Vec<Tag>>, Error>`. Last line becomes `Ok(Json(ts))`.

**`list_recent_items`** — return type becomes `Result<Json<Vec<SearchHit>>, Error>`. Last line becomes `Ok(Json(r))`. (The function body calls `recent::list(...).await` which returns `Result<Vec<SearchHit>>`.)

### Step 4.3: Migrate the 5 matching wrappers in `server.rs`

Open `/Users/rjl/Code/github/zotero-connector/crates/zotero-mcp/src/server.rs`. Add `Json` to the rmcp import block:

```rust
// Before (current import block at line 12)
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{
        AnnotateAble, CallToolResult, Content, Implementation,
        ListResourcesResult, PaginatedRequestParams,
        RawResource, ReadResourceRequestParams, ReadResourceResult,
        ResourceContents, ServerCapabilities, ServerInfo,
    },
    service::RequestContext,
    tool, tool_handler, tool_router, RoleServer,
};

// After — add Json at the top level, keep everything else as-is
use rmcp::{
    ErrorData as McpError, Json, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{
        AnnotateAble, CallToolResult, Content, Implementation,
        ListResourcesResult, PaginatedRequestParams,
        RawResource, ReadResourceRequestParams, ReadResourceResult,
        ResourceContents, ServerCapabilities, ServerInfo,
    },
    service::RequestContext,
    tool, tool_handler, tool_router, RoleServer,
};
```

Then add an import for the 5 output types (also at the top of server.rs, alongside the existing `use crate::tools::...` lines):

```rust
use crate::core::types::{Collection, Item, SearchHit, Tag};
```

Then, for each of the 5 wrappers — `search_items`, `get_item`, `list_collections`, `list_tags`, `list_recent_items` — change the return type from `Result<CallToolResult, McpError>` to `Result<Json<...>, McpError>` per the lookup table:

```rust
// Before
#[tool(description = "Search the local Zotero library (metadata + optional fulltext).",
       annotations(read_only_hint = true, open_world_hint = false))]
pub async fn search_items(
    &self,
    Parameters(args): Parameters<SearchArgs>,
) -> Result<CallToolResult, McpError> {
    search::search_items(&self.state, args).await
}

// After
#[tool(description = "Search the local Zotero library (metadata + optional fulltext).",
       annotations(read_only_hint = true, open_world_hint = false))]
pub async fn search_items(
    &self,
    Parameters(args): Parameters<SearchArgs>,
) -> Result<Json<Vec<SearchHit>>, McpError> {
    search::search_items(&self.state, args).await
}
```

The body is unchanged. Only the return-type annotation changes. The 5 mappings:

| Wrapper | New return type |
|---|---|
| `search_items` | `Result<Json<Vec<SearchHit>>, McpError>` |
| `get_item` | `Result<Json<Item>, McpError>` |
| `list_collections` | `Result<Json<Vec<Collection>>, McpError>` |
| `list_tags` | `Result<Json<Vec<Tag>>, McpError>` |
| `list_recent_items` | `Result<Json<Vec<SearchHit>>, McpError>` |

### Step 4.4: Build and verify

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo build -p zotero-mcp 2>&1 | tail -30`

Expected: clean. The 5 search-related tools now return `Json<T>`; everything else still compiles.

---

## Task 5: Migrate `tools/attachments.rs` + matching `server.rs` wrappers (9 tools)

**Files:**
- Modify: `crates/zotero-mcp/src/tools/attachments.rs`
- Modify: `crates/zotero-mcp/src/server.rs` (only the 9 wrappers that route to `attachments.rs`)

9 tools (in the order they appear in the file): `list_attachments`, `get_pdf_text`, `get_pdf_first_pages`, `list_annotations`, `get_webpage_content`, `refetch_url`, `create_item`, `attach_link`, `attach_file`.

Note: `get_pdf_path` is in this file but uses `Content::text` — **do NOT migrate it**. Its signature stays `Result<CallToolResult, Error>`.

### Step 5.1: Update imports in `tools/attachments.rs`

Open `tools/attachments.rs`. The current rmcp imports:

```rust
use rmcp::ErrorData as Error;
use rmcp::model::{CallToolResult, Content};
```

After this task, the file uses BOTH `Json` (for migrated tools) AND `CallToolResult`+`Content` (for `get_pdf_path`). Change to:

```rust
use rmcp::ErrorData as Error;
use rmcp::Json;
use rmcp::model::{CallToolResult, Content};
```

Add `use` lines for the output types if not present:

```rust
use crate::core::types::{Annotation, Attachment};
use crate::core::pdf::PdfTextResult;
use crate::core::web::{RefetchResult, WebContentResult};
```

Verify against the current import block and add only what's missing.

Remove the `serde_json::{Map, Value}` import line ONLY if no remaining function uses `Value::Object(a.item)` in `create_item_t` — it does. Keep `Map, Value` imports.

### Step 5.2: Migrate the 9 `_t` functions

**`list_attachments_t`** — return type `Result<Json<Vec<Attachment>>, Error>`. Last line: `Ok(Json(r))`.

**`get_pdf_text_t`** — return type `Result<Json<PdfTextResult>, Error>`. Last line: `Ok(Json(r))`.

**`get_pdf_first_pages_t`** — return type `Result<Json<PdfTextResult>, Error>`. Last line: `Ok(Json(r))`.

**`list_annotations_t`** — return type `Result<Json<Vec<Annotation>>, Error>`. Last line: `Ok(Json(r))`.

**`get_webpage_content_t`** — return type `Result<Json<WebContentResult>, Error>`. Last line: `Ok(Json(r))`.

**`refetch_url_t`** — return type `Result<Json<RefetchResult>, Error>`. Last line: `Ok(Json(r))`.

**`create_item_t`** — return type changes from `Result<CallToolResult, Error>` to `Result<Json<CreateItemResult>, Error>`. The body currently constructs a `serde_json::json!({"item_key": key, "version": version})` ad-hoc; replace with the typed struct:

```rust
// Before
pub async fn create_item_t(s: &AppState, a: CreateItemArgs) -> Result<CallToolResult, Error> {
    let item_value = Value::Object(a.item);
    let (key, version) = create_item(&s.api, &item_value, &a.collection_keys)
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::json!({
        "item_key": key,
        "version": version,
    }))?]))
}

// After
pub async fn create_item_t(s: &AppState, a: CreateItemArgs) -> Result<Json<CreateItemResult>, Error> {
    let item_value = Value::Object(a.item);
    let (key, version) = create_item(&s.api, &item_value, &a.collection_keys)
        .await
        .map_err(map_err)?;
    Ok(Json(CreateItemResult {
        item_key: key,
        version: version as i64,
    }))
}
```

Note: `version` from `create_item` is `u64` or similar — verify the actual type. If it's already `i64`, drop the `as i64`. If it's `u64`, the cast is needed because `CreateItemResult.version` is `i64`.

**`attach_link_t`** — return type `Result<Json<AttachmentResult>, Error>`. Body change:

```rust
// Before
let key = attach_link(...).await.map_err(map_err)?;
Ok(CallToolResult::success(vec![Content::json(serde_json::json!({
    "attachment_key": key,
}))?]))

// After
let key = attach_link(...).await.map_err(map_err)?;
Ok(Json(AttachmentResult { attachment_key: key }))
```

**`attach_file_t`** — same pattern as `attach_link_t`. Return type `Result<Json<AttachmentResult>, Error>`. Last line: `Ok(Json(AttachmentResult { attachment_key: key }))`.

### Step 5.3: Migrate the 9 matching wrappers in `server.rs`

In `server.rs`, add imports for the 5 new types (and the 2 handler structs from `tools/attachments.rs`):

```rust
// Add to existing imports
use crate::core::types::{Annotation, Attachment, /* keep prior additions */};
use crate::core::pdf::PdfTextResult;
use crate::core::web::{RefetchResult, WebContentResult};

// The handler structs are re-exported via the `att` alias already used in server.rs:
// `use crate::tools::attachments::{self as att, ...};`
// So they can be referenced as `att::CreateItemResult` and `att::AttachmentResult`
// in the type signatures below.
```

Then change the 9 wrapper signatures:

| Wrapper | New return type |
|---|---|
| `list_attachments` | `Result<Json<Vec<Attachment>>, McpError>` |
| `get_pdf_text` | `Result<Json<PdfTextResult>, McpError>` |
| `get_pdf_first_pages` | `Result<Json<PdfTextResult>, McpError>` |
| `list_annotations` | `Result<Json<Vec<Annotation>>, McpError>` |
| `get_webpage_content` | `Result<Json<WebContentResult>, McpError>` |
| `refetch_url` | `Result<Json<RefetchResult>, McpError>` |
| `create_item` | `Result<Json<att::CreateItemResult>, McpError>` |
| `attach_link` | `Result<Json<att::AttachmentResult>, McpError>` |
| `attach_file` | `Result<Json<att::AttachmentResult>, McpError>` |

Bodies are unchanged.

`get_pdf_path` wrapper signature stays `Result<CallToolResult, McpError>`.

### Step 5.4: Build and verify

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo build -p zotero-mcp 2>&1 | tail -30`

Expected: clean. If `create_item_t`'s `version as i64` cast is wrong (type mismatch), the error tells you the actual return type; adjust the cast or remove it.

---

## Task 6: Migrate `tools/enrichment.rs` + matching `server.rs` wrappers (8 tools)

**Files:**
- Modify: `crates/zotero-mcp/src/tools/enrichment.rs`
- Modify: `crates/zotero-mcp/src/server.rs` (only the 8 wrappers that route to `enrichment.rs`)

8 tools: `find_weak_metadata_items`, `lookup_doi`, `lookup_isbn`, `lookup_arxiv`, `search_crossref`, `search_semantic_scholar`, `propose_metadata_update`, `enrich_item`.

Note: `apply_metadata_update` is in this file but uses `Content::text("applied")` — **do NOT migrate it**. Its signature stays `Result<CallToolResult, Error>`.

### Step 6.1: Update imports in `tools/enrichment.rs`

Open `tools/enrichment.rs`. Current rmcp imports:

```rust
use rmcp::ErrorData as Error;
use rmcp::model::{CallToolResult, Content};
```

The file retains `CallToolResult` + `Content` (for `apply_metadata_update_t`) and adds `Json`:

```rust
use rmcp::ErrorData as Error;
use rmcp::Json;
use rmcp::model::{CallToolResult, Content};
```

Add (verify presence first):

```rust
use crate::core::enrichment::propose::WeakMetadataItem;
use crate::core::types::EnrichmentProposal;  // may already be imported
```

`NormalizedRecord` is already imported (used by `render_record`). No new import for it.

### Step 6.2: Migrate the 8 `_t` functions

**`find_weak_metadata_items_t`** — return type `Result<Json<Vec<WeakMetadataItem>>, Error>`. The function now receives `Vec<WeakMetadataItem>` directly from `find_weak_metadata_items` (per Task 2's signature change). Body:

```rust
// After
pub async fn find_weak_metadata_items_t(
    s: &AppState,
    a: WeakArgs,
) -> Result<Json<Vec<WeakMetadataItem>>, Error> {
    let r = find_weak_metadata_items(&s.pool, 1, a.limit)
        .await
        .map_err(map_err)?;
    Ok(Json(r))
}
```

**`lookup_doi_t`** — return type `Result<Json<serde_json::Value>, Error>`. Body:

```rust
// After
pub async fn lookup_doi_t(s: &AppState, a: DoiArgs) -> Result<Json<serde_json::Value>, Error> {
    let r = s.crossref.lookup_doi(&a.doi).await.map_err(map_err)?;
    let body = render_record(&r, &a.format)?;
    Ok(Json(body))
}
```

**`lookup_isbn_t`** — same pattern as `lookup_doi_t`. Return type `Result<Json<serde_json::Value>, Error>`.

**`lookup_arxiv_t`** — same pattern. Return type `Result<Json<serde_json::Value>, Error>`.

**`search_crossref_t`** — return type `Result<Json<Vec<NormalizedRecord>>, Error>`. Last line: `Ok(Json(r))`.

**`search_semantic_scholar_t`** — return type `Result<Json<Vec<NormalizedRecord>>, Error>`. Last line: `Ok(Json(r))`.

**`propose_metadata_update_t`** — return type `Result<Json<EnrichmentProposal>, Error>`. Last line: `Ok(Json(p))`.

**`enrich_item_t`** — return type `Result<Json<EnrichmentProposal>, Error>`. Last line: `Ok(Json(p))`.

**`apply_metadata_update_t`** — NOT migrated. Stays `Result<CallToolResult, Error>` returning `Ok(CallToolResult::success(vec![Content::text("applied")]))`.

### Step 6.3: Migrate the 8 matching wrappers in `server.rs`

In `server.rs`, add the remaining type imports:

```rust
use crate::core::enrichment::NormalizedRecord;
use crate::core::enrichment::propose::WeakMetadataItem;
use crate::core::types::EnrichmentProposal;
```

Then change the 8 wrapper signatures:

| Wrapper | New return type |
|---|---|
| `find_weak_metadata_items` | `Result<Json<Vec<WeakMetadataItem>>, McpError>` |
| `lookup_doi` | `Result<Json<serde_json::Value>, McpError>` |
| `lookup_isbn` | `Result<Json<serde_json::Value>, McpError>` |
| `lookup_arxiv` | `Result<Json<serde_json::Value>, McpError>` |
| `search_crossref` | `Result<Json<Vec<NormalizedRecord>>, McpError>` |
| `search_semantic_scholar` | `Result<Json<Vec<NormalizedRecord>>, McpError>` |
| `propose_metadata_update` | `Result<Json<EnrichmentProposal>, McpError>` |
| `enrich_item` | `Result<Json<EnrichmentProposal>, McpError>` |

Bodies unchanged.

`apply_metadata_update` wrapper signature stays `Result<CallToolResult, McpError>`.

### Step 6.4: Build and verify

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo build -p zotero-mcp 2>&1 | tail -30`

Expected: clean build, no errors, no warnings about unused imports.

If unused-import warnings appear (e.g., `Content` or `CallToolResult` unused in a file where every `_t` migrated), trim the imports.

---

## Task 7: Extend smoke test, run tests, fmt, commit

**Files:**
- Modify: `crates/zotero-mcp/src/server.rs` (extend the existing `mod tests` block)

### Step 7.1: Extend the smoke test

Open `server.rs` and find the existing `#[cfg(test)] mod tests` block at the bottom (added in Slice F). Append a second `#[test]` function:

```rust
#[test]
fn output_schemas_emitted_for_json_returning_tools() {
    // Json<T> tools get an output schema auto-generated by the #[tool] macro.
    assert!(ZoteroServer::search_items_tool_attr().output_schema.is_some());
    assert!(ZoteroServer::get_item_tool_attr().output_schema.is_some());
    assert!(ZoteroServer::list_collections_tool_attr().output_schema.is_some());
    assert!(ZoteroServer::create_item_tool_attr().output_schema.is_some());

    // Content::text tools don't have output schemas (Slice G didn't touch them).
    assert!(ZoteroServer::format_citation_tool_attr().output_schema.is_none());
    assert!(ZoteroServer::add_tags_tool_attr().output_schema.is_none());
    assert!(ZoteroServer::delete_item_tool_attr().output_schema.is_none());
}
```

The block now has two test functions (`tool_annotations_present_on_representative_tools` from Slice F + `output_schemas_emitted_for_json_returning_tools` new).

### Step 7.2: Run the full test suite

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo test -p zotero-mcp 2>&1 | tail -30`

Expected:
- All existing tests pass.
- `tool_annotations_present_on_representative_tools` passes (unchanged from Slice F).
- `output_schemas_emitted_for_json_returning_tools` passes.
- Lib-test count: 108 (was 107 at Slice F baseline; +1).

If `output_schemas_emitted_for_json_returning_tools` fails:
- Fails on `is_some()` line for a migrated tool → the macro didn't pick up `Json<T>`. Verify the wrapper's return type literally contains `Json<T>` (not `Result<Json<T>, McpError>` somewhere else accidentally). Re-read the wrapper.
- Fails on `is_none()` line for a text tool → that text tool got migrated by accident. Find it (the assertion names which tool) and revert.

Other test failures (e.g., `render_record_*` in `enrichment.rs`) should not occur — those tests exercise `render_record` directly, which is unchanged. If they do fail, investigate immediately (likely import or `use` change broke them).

Write down the final lib-test count for the commit body.

### Step 7.3: Format

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo fmt -p zotero-mcp -- --check`

If `--check` exits non-zero, run: `cargo fmt -p zotero-mcp` and include the result in the same commit. Multi-line return types may get rewrapped.

### Step 7.4: Stage and scope-check

Run:

```bash
cd /Users/rjl/Code/github/zotero-connector && \
git add \
  crates/zotero-mcp/src/core/types.rs \
  crates/zotero-mcp/src/core/pdf.rs \
  crates/zotero-mcp/src/core/web.rs \
  crates/zotero-mcp/src/core/enrichment/mod.rs \
  crates/zotero-mcp/src/core/enrichment/propose.rs \
  crates/zotero-mcp/src/tools/search.rs \
  crates/zotero-mcp/src/tools/attachments.rs \
  crates/zotero-mcp/src/tools/enrichment.rs \
  crates/zotero-mcp/src/server.rs && \
git status --short
```

Expected staged set: exactly these 9 files all with `M` status. Nothing else.

If any other file shows up (e.g., `Cargo.toml`, `Cargo.lock`, `main.rs`, `lib.rs`, `http_transport.rs`, `bearer.rs`, `oauth*`), that's out-of-bounds — either revert or escalate.

Run the invariant counts:

```bash
git diff --cached | grep -E "^\+.*Result<Json<" | wc -l
```

Expected: 44 (22 `_t` functions × 1 line each + 22 `server.rs` wrappers × 1 line each = 44 added `Result<Json<` patterns). The migration is symmetric.

```bash
git diff --cached | grep -E "^\+.*Ok\(Json\(" | wc -l
```

Expected: ~22 (one per `_t` function). May be slightly different if a function had multiple return sites (e.g., `get_item` has multiple `Err(...)` arms but only one `Ok(...)`).

```bash
git diff --cached | grep -E "^-.*CallToolResult::success" | wc -l
```

Expected: 22 (the removed lines from the 22 migrated `_t` functions).

```bash
git diff --cached | grep -E "^\+.*JsonSchema\b" | wc -l
```

Expected: ≥19 (18 derives on existing types + 1+ on new types `WeakMetadataItem`, `CreateItemResult`, `AttachmentResult` = 21 minimum). The exact number depends on whether `JsonSchema` was added in standalone derive lines or appended to existing derives.

### Step 7.5: Commit

Fill `[N]` with the actual lib-test count from Step 7.2.

```bash
git commit -m "$(cat <<'EOF'
chore(tools): migrate 22 tools to Json<T> for output schemas + structured_content

Replaces ~120 lines of CallToolResult::success(vec![Content::json(...)])
boilerplate at 22 sites across tools/{search,attachments,enrichment}.rs
with Ok(Json(r)). The #[tool] macro auto-emits outputSchema on
tools/list responses for each migrated tool, and structured_content
becomes populated alongside the existing content[0] (additive wire
change; backwards compatible).

Site count by file:
  - tools/search.rs:       5 sites
  - tools/attachments.rs:  9 sites
  - tools/enrichment.rs:   8 sites
                          ----------
                          22 total

Cross-cutting prerequisites (per spec):
  - 18 core types gained JsonSchema:
      core/types.rs (12 types: Item, Creator, Attachment,
        AttachmentLinkMode, Collection, Tag, Annotation, SearchHit,
        Diff, FieldChange, EnrichmentProposal, SourceBreakdown)
      core/pdf.rs (2 types: PdfTextResult, PdfTextSource)
      core/web.rs (3 types: WebContentResult, WebSource, RefetchResult)
      core/enrichment/mod.rs (1 type: NormalizedRecord)
  - 3 new structs:
      tools/attachments.rs: CreateItemResult { item_key, version }
                            AttachmentResult { attachment_key }
      core/enrichment/propose.rs: WeakMetadataItem { item_key, weak_fields }
  - core/enrichment/propose.rs: find_weak_metadata_items return type
      changes from Vec<(String, Vec<String>)> to Vec<WeakMetadataItem>.

Tool wrappers in server.rs: 22 return-type annotations changed from
Result<CallToolResult, McpError> to Result<Json<T>, McpError>. Bodies
unchanged. The other 12 tool wrappers (ping + 11 Content::text-returning
mutations and formatters) keep Result<CallToolResult, McpError>.

lookup_doi/isbn/arxiv use Json<serde_json::Value> because render_record's
output shape varies with the format parameter. Properly typing these
needs API redesign (deferred).

Smoke test extended: output_schemas_emitted_for_json_returning_tools
asserts output_schema.is_some() on 4 migrated tools (search_items,
get_item, list_collections, create_item) and is_none() on 3 text-returning
tools (format_citation, add_tags, delete_item). Lib-test count: [N]
passed; 0 failed (was 107 at Slice F baseline; delta: +1).

Wire format: backwards compatible. CallToolResult::structured (which
Json<T> calls into) populates content[0] = Content::text(stringified_json)
identically to the old Content::json(value) path AND populates the new
structured_content: Some(value) field. Existing clients see no change;
new clients can read structured_content + validate against the new
outputSchema.

REINSTALL + LAUNCHD FLIP DELIBERATELY NOT PERFORMED in this commit.
User triggers `cargo install --path crates/zotero-mcp` and
`launchctl kickstart -k gui/$(id -u)/com.zotero-mcp.http` separately.
After install, tools/list responses on the migrated tools will carry
outputSchema for the first time.

Spec: docs/superpowers/specs/2026-05-13-dependency-upgrades-slice-g-design.md
Plan: docs/superpowers/plans/2026-05-13-dependency-upgrades-slice-g.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Replace `[N]` with the actual lib-test count from Step 7.2 (expected: 108).

### Step 7.6: After commit, STOP — do not reinstall or restart launchd

Report `git rev-parse HEAD` to the controller. Confirm:

- ✅ Build clean.
- ✅ Tests pass at the documented count (108).
- ✅ Only the 9 expected files changed (`git diff --stat HEAD~1 HEAD`).
- ✅ Invariant counts from Step 7.4 within expected ranges.
- ✅ Commit body's `[N]` placeholders filled.
- ❌ Do NOT run `cargo install --path crates/zotero-mcp`. User's separate step.

---

## Escalation block

The slice can be partially landed across tasks BUT not partially committed (single atomic commit). If during any task one of these surfaces and resists a fix scoped to the 9 named files:

1. **`JsonSchema` derive fails on a `serde_json::Value` or `Map<String, Value>` field.** schemars 1.x should provide these impls. If it doesn't, try `#[schemars(with = "serde_json::Value")]` on the field. If that doesn't compile, escalate.
2. **`find_weak_metadata_items` has an external caller** (outside `tools/enrichment.rs`) whose update would ripple beyond the 9 files. Examples: a CLI subcommand, an integration test, a benchmark. Either include the caller's file in the commit (still within the spirit of the slice — the surface change cascades naturally) or escalate.
3. **The `#[tool]` macro doesn't pick up `Json<T>`** for a specific tool — `output_schema_emitted_for_json_returning_tools` test fails on a tool we expected to have a schema. Re-read `rmcp-macros-1.7.0/src/tool.rs` lines 8–70 to ground-truth the inner-type extraction logic. The macro handles direct `Json<T>` and `Result<Json<T>, E>` — confirm the wrapper's return type literally matches one of those forms.
4. **A test in `tools/enrichment.rs` (the `render_record_*` tests) starts failing.** These exercise `render_record` directly and should be unaffected by the migration. If they fail, the migration accidentally changed `render_record`'s shape — revert that change.
5. **`cargo install`'s rebuild fails after the commit** (user-reported, post-slice). Almost certainly an out-of-tree consumer of a changed public surface. Document the symptom and escalate; the commit itself is sound.

**Revert:**

```bash
cd /Users/rjl/Code/github/zotero-connector && \
git checkout -- \
  crates/zotero-mcp/src/core/types.rs \
  crates/zotero-mcp/src/core/pdf.rs \
  crates/zotero-mcp/src/core/web.rs \
  crates/zotero-mcp/src/core/enrichment/mod.rs \
  crates/zotero-mcp/src/core/enrichment/propose.rs \
  crates/zotero-mcp/src/tools/search.rs \
  crates/zotero-mcp/src/tools/attachments.rs \
  crates/zotero-mcp/src/tools/enrichment.rs \
  crates/zotero-mcp/src/server.rs
```

Then amend the spec at `docs/superpowers/specs/2026-05-13-dependency-upgrades-slice-g-design.md` — append:

```markdown
---

## Deferred (Slice G Json<T> migration)

[Date]: This slice was attempted and reverted. Blocker:

[One paragraph naming the specific file, API, and reason mechanical
fixing wasn't possible. Reference the escalation-block item number.]
```

Commit as: `docs(spec): defer Slice G — <one-line reason>`. Report DONE_WITH_CONCERNS.

---

## Hand-off

After Task 7 lands (commit on `main`, tests green, build clean):

- [ ] Implementer reports `git rev-parse HEAD` and the lib-test count.
- [ ] Two-stage review per the campaign pattern (spec-compliance reviewer, then code-quality reviewer).
- [ ] User decides when to `cargo install --path crates/zotero-mcp` + `launchctl kickstart -k gui/$(id -u)/com.zotero-mcp.http`.
- [ ] After install, user can verify the migration on a Cowork roundtrip: a `tools/list` response should now include `outputSchema` for migrated tools (e.g., `search_items`, `delete_item` should NOT have one). Optional spot-check.

---

## Verification checklist

After Task 7 completes:

- [ ] One commit lands on `main` with the documented message format (`chore(tools): migrate 22 tools to Json<T> for output schemas + structured_content`).
- [ ] The commit passes `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp`.
- [ ] The new lib-test count is explicitly noted in the commit body (108 = 107 + 1).
- [ ] `git diff --stat HEAD~1 HEAD` shows exactly 9 files modified.
- [ ] Diff contains ~44 added `Result<Json<` lines (22 `_t` + 22 wrapper) — Step 7.4 invariant.
- [ ] Diff contains exactly 22 removed `CallToolResult::success` lines (one per migrated tool).
- [ ] Diff contains ≥19 added `JsonSchema` substrings (18 core derives + new struct derives).
- [ ] `apply_metadata_update_t` and `get_pdf_path` were NOT migrated (their `Content::text` returns are byte-identical to before).
- [ ] Smoke test `output_schemas_emitted_for_json_returning_tools` passes.
- [ ] `cargo install` and `launchctl kickstart` were NOT executed by the implementer.
