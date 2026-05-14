# Spec: Dependency upgrades — Slice G (migrate 22 tools to `Json<T>` return type)

**Status:** Approved design, ready for plan-writing.
**Author:** Richard Lyon (with Claude Opus 4.7).
**Date:** 2026-05-13.
**Goal:** Replace the `Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))` boilerplate at 22 sites across `crates/zotero-mcp/src/tools/{search,attachments,enrichment}.rs` with `Ok(Json(r))`. The `#[tool]` macro then auto-generates `outputSchema` entries on `tools/list` responses. Lands as one atomic commit.

---

## Problem

The 21 `Content::json` sites in `tools/*.rs` use the pattern:

```rust
pub async fn tool_t(...) -> Result<CallToolResult, Error> {
    let r = some_core_call(...).await.map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}
```

Two costs:

1. **~6 lines × 21 = ~120 lines of mechanical boilerplate** in each tool handler, drowning the actual domain call.
2. **No `outputSchema` on the `tools/list` response.** MCP clients (Claude Cowork, Claude Desktop) have no static description of what a tool returns. They learn the shape only by calling the tool and inspecting the response, which is brittle for tool selection.

rmcp 1.7 exposes a return-type wrapper at `rmcp::handler::server::wrapper::Json` (re-exported as `rmcp::Json`). The wrapper is essentially `pub struct Json<T>(pub T)` with two trait impls:

- `impl<T: JsonSchema> JsonSchema for Json<T>` — delegates to `T`'s schema.
- `impl<T: Serialize + JsonSchema + 'static> IntoCallToolResult for Json<T>` — serializes `T` into `CallToolResult::structured(value)`, which populates both `content[0] = Content::text(value.to_string())` (backwards-compatible) AND `structured_content: Some(value)` (the new typed field).

The `#[tool]` macro in `rmcp-macros-1.7.0/src/tool.rs` inspects each handler's return type. When it detects `Json<T>` or `Result<Json<T>, E>`, it emits a `with_raw_output_schema(...)` call (line 271, `model/tool.rs`) populated from `<T as JsonSchema>::json_schema(...)`. The schema lands in `Tool.output_schema: Option<Arc<JsonObject>>` and surfaces on the wire as `outputSchema`.

Verified at spec time by reading `rmcp-1.7.0/tests/test_structured_output.rs` (working example with `Result<Json<CalculationResult>, String>`) and `rmcp-macros-1.7.0/src/tool.rs` lines 8–70 (the inner-type extraction logic that handles both `Json<T>` and `Result<Json<T>, E>`).

The wire change is **additive**: existing clients reading `content[0].text` see an identical JSON-stringified payload (today `Content::json(value)` is `RawContent::Text` with stringified JSON — verified in `rmcp-1.7.0/src/model/content.rs:164–174`; `CallToolResult::structured(value)` populates `content[0]` the same way). New clients reading `structured_content` get a typed object and can validate against `outputSchema`. No client breakage path.

---

## Decisions

1. **Migrate only the 21 `Content::json` sites.** The 13 `Content::text` sites (`format_citation`, `format_bibliography`, `get_pdf_path`, `add_note`, `update_item_fields`, `add_tags`, `remove_tags`, `add_to_collection`, `remove_from_collection`, `delete_item`, `apply_metadata_update`, and `ping`/the small text returns) stay unchanged — their bodies are intentionally bare strings, not JSON. A typed `{status: "ok"}` envelope adds noise without benefit.

2. **Full `_t` migration.** Each migrated `_t` function's signature changes from `Result<CallToolResult, Error>` to `Result<Json<OutputT>, Error>`. The matching wrapper in `server.rs` also changes from `Result<CallToolResult, McpError>` to `Result<Json<OutputT>, McpError>`. Both halves are required so the macro can detect `Json<T>` in the wrapper signature — see Migration Plan §5.

3. **Add `JsonSchema` to ~18 core types.** Mechanical addition to the existing `#[derive(Debug, Clone, Serialize, Deserialize)]` line on each affected type. Files and types:

   | File | Types getting `JsonSchema` |
   |---|---|
   | `core/types.rs` | `Item`, `Creator`, `Attachment`, `AttachmentLinkMode`, `Collection`, `Tag`, `Annotation`, `SearchHit`, `Diff`, `FieldChange`, `EnrichmentProposal`, `SourceBreakdown` (12 types) |
   | `core/pdf.rs` | `PdfTextResult`, `PdfTextSource` (2 types) |
   | `core/web.rs` | `WebContentResult`, `WebSource`, `RefetchResult` (3 types) |
   | `core/enrichment/mod.rs` | `NormalizedRecord` (1 type) |

   Each file gets `use schemars::JsonSchema` added at the head if not already present. schemars 1.x supplies `impl JsonSchema for serde_json::Value` and `for serde_json::Map<String, Value>` out of the box, so types containing `Value` or `Map<String, Value>` fields (`Item.fields`, `FieldChange.current/proposed`, `NormalizedRecord.fields`) compile without per-field annotations.

4. **`lookup_doi` / `lookup_isbn` / `lookup_arxiv`: `Json<serde_json::Value>`.** `render_record(record, format)` returns a `Value` whose shape varies on the `format` parameter (flat Zotero item vs. NormalizedRecord envelope). Wrap in `Json<Value>`; the output schema becomes effectively "any object" (no useful shape advertised), but `structured_content` still gets populated for clients that read it. Properly typing this is deferred — requires redesigning the `format` parameter, which is out of Slice G's scope.

5. **Three new output structs** in `tools/attachments.rs` (handler-shaped, co-located with the handlers that use them — not promoted to `core/types.rs`):

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

   `AttachmentResult` is reused by `attach_link_t` and `attach_file_t`.

6. **`WeakMetadataItem` struct + return-type change** in `core/enrichment/propose.rs`. `find_weak_metadata_items` currently returns `Result<Vec<(String, Vec<String>)>>` — a tuple of tuples without a clean JSON Schema. Replace with:

   ```rust
   #[derive(Debug, Serialize, Deserialize, JsonSchema)]
   pub struct WeakMetadataItem {
       pub item_key: String,
       pub weak_fields: Vec<String>,
   }
   ```

   `find_weak_metadata_items` returns `Result<Vec<WeakMetadataItem>>`. Internal callers (probably 1-2: the tool layer plus possibly a unit test) update to the new shape. The function lives in `core/enrichment/propose.rs`, so the new struct goes there.

7. **Smoke test extended, not replaced.** Slice F's `tool_annotations_present_on_representative_tools` test stays. A new test asserts the macro's `output_schema` behaviour:

   ```rust
   #[test]
   fn output_schemas_emitted_for_json_returning_tools() {
       // Json<T> tools get an output schema
       assert!(ZoteroServer::search_items_tool_attr().output_schema.is_some());
       assert!(ZoteroServer::get_item_tool_attr().output_schema.is_some());
       assert!(ZoteroServer::list_collections_tool_attr().output_schema.is_some());
       assert!(ZoteroServer::create_item_tool_attr().output_schema.is_some());

       // Content::text tools don't have output schemas
       assert!(ZoteroServer::format_citation_tool_attr().output_schema.is_none());
       assert!(ZoteroServer::add_tags_tool_attr().output_schema.is_none());
       assert!(ZoteroServer::delete_item_tool_attr().output_schema.is_none());
   }
   ```

   The `Tool.output_schema: Option<Arc<JsonObject>>` accessor is verified in `rmcp-1.7.0/src/model/tool.rs:30`.

8. **Single atomic commit directly on `main`, no PR.** Matches the campaign pattern. The migration can't be partially landed — signature changes ripple from `_t` to wrapper, and the JsonSchema derives are prerequisites for every migrated site.

   Commit message format:

   ```
   chore(tools): migrate 22 tools to Json<T> for output schemas + structured_content
   ```

9. **Test gate.** `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp` both clean. Lib-test count delta: **+1** (the new `output_schemas_emitted_for_json_returning_tools` smoke test). Note the new total in the commit body.

10. **Reinstall + launchd restart NOT performed in the slice.** Consistent with the campaign pattern. After the commit lands and tests pass, the user decides when to `cargo install --path crates/zotero-mcp` + `launchctl kickstart -k gui/$(id -u)/com.zotero-mcp.http`. Spot-check on Cowork: `tools/list` shows `outputSchema` populated on migrated tools.

---

## Site inventory (the 22 tools + their output types)

| # | Tool | `_t` output type | Source of the value |
|---|---|---|---|
| 1 | `search_items` | `Vec<SearchHit>` | `core::reader::search::search_metadata` |
| 2 | `get_item` | `Item` | `core::reader::items::get_item_by_key` |
| 3 | `list_collections` | `Vec<Collection>` | `core::reader::collections::list` |
| 4 | `list_tags` | `Vec<Tag>` | `core::reader::tags::list` |
| 5 | `list_recent_items` | `Vec<SearchHit>` | `core::reader::recent::list` |
| 6 | `list_attachments` | `Vec<Attachment>` | `core::reader::attachments::list_attachments` |
| 7 | `get_pdf_text` | `PdfTextResult` | `core::pdf::get_pdf_text` |
| 8 | `get_pdf_first_pages` | `PdfTextResult` | `core::pdf::get_pdf_first_pages` |
| 9 | `list_annotations` | `Vec<Annotation>` | `core::reader::annotations::list_annotations` |
| 10 | `get_webpage_content` | `WebContentResult` | `core::web::get_webpage_content` |
| 11 | `refetch_url` | `RefetchResult` | `core::web::refetch_url` |
| 12 | `create_item` | NEW `CreateItemResult` | (handler-only) |
| 13 | `attach_link` | NEW `AttachmentResult` | (handler-only) |
| 14 | `attach_file` | NEW `AttachmentResult` | (handler-only) |
| 15 | `find_weak_metadata_items` | `Vec<WeakMetadataItem>` (NEW) | `core::enrichment::propose::find_weak_metadata_items` (return-type change) |
| 16 | `lookup_doi` | `serde_json::Value` | `render_record` (varying shape) |
| 17 | `lookup_isbn` | `serde_json::Value` | `render_record` |
| 18 | `lookup_arxiv` | `serde_json::Value` | `render_record` |
| 19 | `search_crossref` | `Vec<NormalizedRecord>` | `core::enrichment::crossref::Crossref::search` |
| 20 | `search_semantic_scholar` | `Vec<NormalizedRecord>` | `core::enrichment::semantic_scholar::SemanticScholar::search` |
| 21 | `propose_metadata_update` | `EnrichmentProposal` | `core::enrichment::propose::propose_metadata_update` |
| 22 | `enrich_item` | `EnrichmentProposal` | `core::enrichment::propose::enrich_item` |

(The campaign brief's "21 sites" was approximate; the file actually has 22. Same magnitude, same migration pattern.)

The 13 tools that stay on the existing return pattern (verify NOT migrated): `ping`, `get_pdf_path`, `format_citation`, `format_bibliography`, `add_note`, `update_item_fields`, `add_tags`, `remove_tags`, `add_to_collection`, `remove_from_collection`, `delete_item`, `apply_metadata_update`. Each returns `Content::text(...)` and `Json<T>` would not add value.

---

## Migration plan

Compile-error-driven port. The migration sequence below documents the order; the implementer iterates `cargo build` and applies the patterns to each named site.

### 1. Add `JsonSchema` to core types (Decision 3)

For each of the 18 types listed in Decision 3, extend the existing `#[derive(...)]` line to include `JsonSchema`. Add `use schemars::JsonSchema;` at each file head if not already imported.

Example:

```rust
// Before — core/types.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item { ... }

// After
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Item { ... }
```

### 2. Introduce `WeakMetadataItem` + change `find_weak_metadata_items` return type (Decision 6)

In `core/enrichment/propose.rs`:

```rust
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WeakMetadataItem {
    pub item_key: String,
    pub weak_fields: Vec<String>,
}

pub async fn find_weak_metadata_items(
    pool: &ReadOnlyPool,
    library_id: i64,
    limit: i64,
) -> Result<Vec<WeakMetadataItem>> {
    // Returns the same data, just shaped differently.
}
```

Update internal callers (likely just `tools::enrichment::find_weak_metadata_items_t`) and any unit tests that consume the tuple shape.

### 3. Introduce `CreateItemResult` and `AttachmentResult` in `tools/attachments.rs` (Decision 5)

Add the two structs at the top of `tools/attachments.rs` (alongside the existing `#[derive(Debug, Deserialize, Serialize, JsonSchema)]` Args structs).

### 4. Migrate the 22 `_t` functions (Decision 2)

For each migrated site, replace:

```rust
pub async fn search_items(s: &AppState, a: SearchArgs) -> Result<CallToolResult, Error> {
    let hits = search_metadata(...).await.map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&hits).unwrap(),
    )?]))
}
```

with:

```rust
pub async fn search_items(s: &AppState, a: SearchArgs) -> Result<Json<Vec<SearchHit>>, Error> {
    let hits = search_metadata(...).await.map_err(map_err)?;
    Ok(Json(hits))
}
```

Add `use rmcp::Json;` at each file head. Remove `Content` from the rmcp imports if no Content::text sites remain in the file; keep it otherwise (e.g., `tools/attachments.rs` still has `get_pdf_path` returning `Content::text`).

### 5. Migrate matching `server.rs` wrappers (Decision 2)

For each of the 22 migrated tools, change the wrapper's return type:

```rust
// Before
#[tool(description = "...", annotations(...))]
pub async fn search_items(
    &self,
    Parameters(args): Parameters<SearchArgs>,
) -> Result<CallToolResult, McpError> {
    search::search_items(&self.state, args).await
}

// After
#[tool(description = "...", annotations(...))]
pub async fn search_items(
    &self,
    Parameters(args): Parameters<SearchArgs>,
) -> Result<Json<Vec<SearchHit>>, McpError> {
    search::search_items(&self.state, args).await
}
```

Add `use rmcp::Json;` (and import the output types: `SearchHit`, `Item`, `Collection`, `Tag`, `Attachment`, `Annotation`, `PdfTextResult`, `WebContentResult`, `RefetchResult`, `EnrichmentProposal`, `NormalizedRecord`, `WeakMetadataItem`, plus the new handler structs `CreateItemResult` and `AttachmentResult`).

### 6. Extend the smoke test (Decision 7)

Append `output_schemas_emitted_for_json_returning_tools` to the existing `#[cfg(test)] mod tests` block at the bottom of `server.rs`. The Slice F test `tool_annotations_present_on_representative_tools` stays as-is.

### 7. Build, test, fmt, commit

Same flow as Slice F. One commit.

---

## Risks

1. **`JsonSchema` derive on `Value`-containing fields.** `Item.fields: serde_json::Value`, `FieldChange.current: Option<Value>`, `FieldChange.proposed: Value`, `NormalizedRecord.fields: serde_json::Map<String, Value>`. schemars 1.x provides built-in `impl JsonSchema` for `serde_json::Value` and `serde_json::Map`. Verified at spec time — should compile. Mitigation if it doesn't: add `#[schemars(with = "serde_json::Value")]` on the offending field, or open-code a `JsonSchema` impl. Escalate if neither works in <20 lines.

2. **`find_weak_metadata_items` external callers.** The function is `pub` and could be called from outside the tool layer (e.g., a CLI subcommand). Quick grep at implementation time confirms scope. If a caller outside the tool surface exists, fix it in the same commit — still inside Slice G's bounds (the surface change ripples to its consumers).

3. **Slice size and atomicity.** Largest commit of the campaign (~9 files, ~150+ line delta). Atomic by necessity — signature changes propagate through `_t` → wrapper → tool macro. If an unforeseen ripple surfaces (e.g., a tool's `_t` is called from a test elsewhere with the old signature), escalate per the campaign pattern: revert, amend the spec's Deferred section, commit that.

4. **Lookup-tool schema regression.** `Json<Value>` advertises essentially "any object" — less precise than the current absence of an output schema. Clients that ignore output schemas are unaffected; clients that validate against schemas now accept any payload. Acceptable trade-off; the alternative (typed `lookup_*` outputs) requires API redesign and is deferred.

5. **`apply_metadata_update_t` returns `"applied"` via `Content::text` — NOT a migration target.** Note explicitly so the implementer doesn't accidentally touch it (it's in the same file as some migrated handlers).

6. **Existing `render_record_*` tests in `enrichment.rs`.** Three unit tests verify the raw `Value` output of `render_record`. They continue to pass — Slice G changes how the value gets wrapped at the tool boundary, not how `render_record` produces it.

7. **Order of operations during compile-error iteration.** The natural order is: (a) add `JsonSchema` derives → compiles standalone, (b) introduce `WeakMetadataItem` + change return type → compiles after internal callers updated, (c) introduce handler output structs → compiles standalone, (d) migrate `_t` functions and wrappers together (file by file). The implementer can iterate `cargo build` after each pass.

---

## Verification checklist (end of slice)

- [ ] One commit lands on `main` with message format `chore(tools): migrate 22 tools to Json<T> for output schemas + structured_content`.
- [ ] The commit passes `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp`.
- [ ] The new lib-test count is explicitly noted in the commit body (+1 from Slice F's 107 = 108).
- [ ] `git diff --stat` shows the 9 expected files: `crates/zotero-mcp/src/{server.rs, tools/{search.rs, attachments.rs, enrichment.rs}, core/{types.rs, pdf.rs, web.rs, enrichment/mod.rs, enrichment/propose.rs}}`.
- [ ] No tool that returned `Content::text` shows a return-type change. The 13 text-returning tools (`ping`, `get_pdf_path`, `format_citation`, `format_bibliography`, `add_note`, `update_item_fields`, `add_tags`, `remove_tags`, `add_to_collection`, `remove_from_collection`, `delete_item`, `apply_metadata_update`) are byte-identical.
- [ ] Smoke test `output_schemas_emitted_for_json_returning_tools` passes.
- [ ] No `cargo install` and no `launchctl kickstart` executed inside the slice.

---

## Out of scope (deferred)

- **`lookup_doi/isbn/arxiv` typed output.** Their conditional shape (driven by the `format` parameter) needs API redesign — likely splitting into per-format tools or removing `format` entirely and always returning the candidate envelope. Defer to a follow-up.
- **`Content::text` → `Json<T>` migration.** The 13 text-returning tools were intentionally designed as bare strings. If MCP clients later request structured output for, say, `format_citation`, that's a separate decision.
- **Promoting `CreateItemResult` and `AttachmentResult` to `core/types.rs`.** They're handler-shaped, not domain types — co-location with the handler is correct for now. If other code grows to need them, promotion is a one-line move.
- **Slice H.** Per-field `///` doc-comment → schemars schema-description migration on Args structs. Next slice.

---

## Decisions deferred to implementation

- The exact set of internal callers of `find_weak_metadata_items` that need updating (Risk 2). A grep at implementation time enumerates the set.
- Whether to keep or remove `Content` from the rmcp imports in each `tools/*.rs` file — depends on whether the file still has `Content::text` sites.
- Whether `cargo fmt` re-wraps the now-longer return type signatures in any noisy way (similar to the Slice F observation). Acceptable to run `cargo fmt -p zotero-mcp` and include the result in the same commit.
