# Spec: Slice G completion — typed output for the nine list-returning tools

**Status:** Approved by Richard 2026-07-25 (Option 1 of the decision brief). Ready for plan-writing.
**Author:** Richard Lyon (with Claude Opus 5).
**Date:** 2026-07-25.
**Decision brief:** `docs/superpowers/specs/2026-07-25-slice-g-wire-format-decision.md` — read it for the options considered and the rmcp constraints verified.
**Prior work:** `2026-05-13-dependency-upgrades-slice-g-design.md` (10 of 22 tools migrated, 12 stalled).
**Goal:** migrate the nine list-returning tools to a strict typed output via a uniform `ListResult<T>` envelope, so every one of them advertises an `outputSchema` and returns `structured_content`. Leave the three lookup tools untyped by decision, not deferral. End state: 19 of 22 tools strictly typed, and a test that makes the class of failure behind this whole exercise impossible to ship.

---

## Problem

Nine tools return a JSON array: `search_items`, `list_recent_items`, `list_collections`, `list_tags`, `list_attachments`, `list_annotations`, `search_crossref`, `search_semantic_scholar`, `find_weak_metadata_items`. The MCP spec requires a tool's `outputSchema` to have a root `"type": "object"`, and rmcp enforces it by **panicking when the tool router is built** (`handler/server/common.rs:66` → `router/tool/tool_traits.rs:61`). `schemars` renders `Vec<T>` as `"type": "array"`, so none of the nine can be typed as-is.

The blocker in May was not technical but a judgement call: any wrapper changes what those nine put in the response's text content, from `[{…}]` to `{"items":[{…}]}`. That was left unresolved because Cowork's behaviour against the new shape was unverified. **Richard confirmed 2026-07-25 that Cowork is not and never was a client of this server**, which removes the only identified consumer at risk. The remaining clients hand tool output to a model to read, and no code anywhere parses these arrays programmatically.

There is also a second, quieter problem the envelope fixes. `search_items` takes a `limit` and returns a bare array, so a caller cannot tell a complete result set from one the limit cut off. Fifty hits and "fifty hits, and there were more" are the same response today.

---

## Decisions

1. **One uniform envelope for all nine, not nine bespoke ones.** A single generic type, so the model learns one shape rather than nine:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListResult<T> {
    pub items: Vec<T>,
    /// How many items this response carries.
    pub count: usize,
    /// True when `count` reached the limit that was applied, so the library may
    /// hold more matches than are shown here. Ask again with a higher limit or
    /// an offset.
    pub possibly_truncated: bool,
}
```

   Verified: this produces a root `type: object` schema and passes rmcp's check. `items` rather than a per-tool name (`hits`, `collections`, …) because uniformity is the point — a caller that has learned one list tool has learned all nine.

2. **`possibly_truncated` is honest about being a heuristic.** It is `count >= effective_limit`, which cannot distinguish "exactly `limit` matches exist" from "more exist". Hence *possibly*. The alternative — a `COUNT(*)` companion query per call — doubles the query cost of every list tool to sharpen an edge case the caller can resolve by asking for one more row. Not worth it.

3. **The effective limit is named once.** `search_metadata` silently substitutes 50 when the caller passes ≤ 0. The tool layer needs the same number to compute `possibly_truncated`, so the default moves to a `pub const DEFAULT_SEARCH_LIMIT: i64 = 50` in `core/reader/search.rs` and both sites use it. Duplicating the literal would be a bug waiting for someone to change one of them.

4. **Tools with no limit report `possibly_truncated: false`.** `list_collections`, `list_tags`, `list_attachments`, `list_annotations` return everything they find. The field stays present rather than being omitted, because a field that appears only sometimes is worse for a reader than one that is always there and sometimes false.

5. **`find_weak_metadata_items` gets a real element type.** It returns `Vec<(String, Vec<String>)>` today — a tuple pair whose JSON is a positional array, self-documenting to nobody. Introduce, in `core/enrichment/propose.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WeakMetadataItem {
    pub item_key: String,
    pub weak_fields: Vec<String>,
}
```

   This *does* change that tool's element shape from `["ABC123", ["DOI"]]` to `{"item_key": "ABC123", "weak_fields": ["DOI"]}` — a second wire change beyond the envelope, and a strict improvement. Carried in this slice because it is the same class of change and the same file.

6. **`ListResult<T>` lives in a new `tools/wire.rs`.** It is a wire-shape type, not a domain type: nothing in `core/` produces or consumes it, and putting it in `core/types.rs` would imply otherwise. Constructors `ListResult::complete(items)` and `ListResult::with_limit(items, limit)` keep the arithmetic in one place instead of at nine call sites.

7. **The three lookup tools stay on `CallToolResult`, by decision.** `lookup_doi`, `lookup_isbn`, `lookup_arxiv` *could* migrate — `Map<String, Value>` passes the object-root check, contrary to the old spec's assumption — but the schema they would gain says only "returns an object", which tells a client nothing, while migrating would force today's structured `lookup_failed` body (attempt trail plus a `suggestion` the caller branches on) out of tool content and into `ErrorData.data`, whose visibility to the model is unverified for Claude Code. Trading a working capability for an empty schema is a bad trade. **Recorded here as a closed decision so it stops resurfacing as unfinished business.**

8. **The 13 text-returning tools stay unchanged**, as in the original Slice G: `ping`, `get_pdf_path`, `format_citation`, `format_bibliography`, `add_note`, `update_item_fields`, `add_tags`, `remove_tags`, `add_to_collection`, `remove_from_collection`, `delete_item`, `apply_metadata_update`. Their bodies are deliberately bare strings.

9. **Test sufficiency is part of this slice, not a follow-up.** See the next section — this is the half of the work that was missing.

---

## Test sufficiency (the honest assessment)

The existing 185 lib tests plus 39 test binaries cover the domain logic well. For *this* change they were insufficient in two specific ways, both of which map exactly onto how this server has actually failed:

**Gap 1 — nothing enumerated the tool surface.** `tests/schema_shape.rs` checks four hand-picked types. The production incident behind this whole exercise was one malformed schema causing Claude Code's validator to reject the entire `tools/list` response, leaving the server connected with zero usable tools. No test would have caught it. And rmcp's object-root violation is a *panic at router construction*, which no current test triggers, so a bad migration here would compile, pass the suite, and die on startup.

**Gap 2 — nothing asserted the wire shape.** No test checks what a tool response actually contains. The whole subject of this slice is a change to that shape, so shipping it without such a test would mean the reviewable artefact of the change is untested.

Both are closed here, before any tool is migrated:

1. **`tests/tool_surface.rs`** — builds the real tool router and walks *every* registered tool, asserting: an object-rooted input schema; an object-rooted output schema wherever one is present; a non-empty description; annotations present. Plus an expected-tool-count assertion, so a tool silently disappearing from the router fails the build. Building the router is itself the guard: it is the exact operation that panics on a bad output schema.
2. **`tests/tool_wire_shape.rs`** — builds a real `AppState` against the test fixture and calls tools end-to-end through `IntoCallToolResult`, asserting the actual `content[0].text` and `structured_content` for: a migrated list tool (envelope present, `count` correct, `items` an array), an already-typed object tool (unchanged), and a text tool (still bare text, no structured content). This is the test that would have made the May decision an experiment rather than a debate.

Neither test needs a live Zotero: the router is static, and `AppState` builds against the fixture SQLite with `user_id` pinned so no user-id detection call is attempted.

What remains **not** covered afterwards, stated plainly rather than implied: no test exercises a real MCP client against the server, so "Claude Code accepts these schemas" is inferred from spec compliance rather than observed. Closing that needs a client in the loop, which is Richard's reinstall-and-check step, not something a unit test can assert.

---

## Migration table

| Tool | Before | After |
|---|---|---|
| `search_items` | `Vec<SearchHit>` | `Json<ListResult<SearchHit>>`, limit-aware |
| `list_recent_items` | `Vec<SearchHit>` | `Json<ListResult<SearchHit>>`, limit-aware |
| `list_collections` | `Vec<Collection>` | `Json<ListResult<Collection>>` |
| `list_tags` | `Vec<Tag>` | `Json<ListResult<Tag>>` |
| `list_attachments` | `Vec<Attachment>` | `Json<ListResult<Attachment>>` |
| `list_annotations` | `Vec<Annotation>` | `Json<ListResult<Annotation>>` |
| `search_crossref` | `Vec<NormalizedRecord>` | `Json<ListResult<NormalizedRecord>>`, limit-aware |
| `search_semantic_scholar` | `Vec<NormalizedRecord>` | `Json<ListResult<NormalizedRecord>>`, limit-aware |
| `find_weak_metadata_items` | `Vec<(String, Vec<String>)>` | `Json<ListResult<WeakMetadataItem>>`, limit-aware |

All nine element types already derive `JsonSchema` (kept from Slice G deliberately for this purpose), so no `core/types.rs` change is needed.

**Files touched:** `tools/wire.rs` (new), `tools/mod.rs`, `tools/search.rs`, `tools/attachments.rs`, `tools/enrichment.rs`, `core/reader/search.rs` (the const), `core/enrichment/propose.rs` (`WeakMetadataItem`), `server.rs` (nine wrapper signatures), `tests/tool_surface.rs` (new), `tests/tool_wire_shape.rs` (new), `README.md`, `CHANGELOG.md`.

**Not touched:** the transport files (`bearer.rs`, `oauth.rs`, `oauth/token_store.rs`, `http_transport.rs`, `setup.rs`).

---

## Risks

1. **The nine tools' text content changes shape.** Accepted per the decision brief; no identified consumer. The mitigation is that it is now *tested and documented* rather than discovered.
2. **`find_weak_metadata_items` element shape changes too** (Decision 5). Larger change than the envelope alone for that one tool. Its only consumer is the enrichment workflow prose, which reads fields by name.
3. **`ListResult<T>`'s schema title is `ListResult` for every instantiation.** Each tool's output schema is generated independently, so there is no collision on the wire; worth knowing if a future change ever generates one document containing two instantiations.
4. **`AppState::build` in a test could reach the network** if `user_id` were left at 0 (it would try to detect one via the local API). The test pins `user_id = 1`. If a future refactor makes `build` unconditionally contact Zotero, `tests/tool_wire_shape.rs` starts failing on a machine without Zotero running — a loud failure, not a silent one.
5. **A reinstall is required before Claude Code sees any of this**, and the tool list is what changes. If something is wrong with a schema, the symptom will be the familiar "connected, no tools". `tests/tool_surface.rs` is the guard that makes that unlikely, but the first reinstall after this change is worth watching.

---

## Acceptance

- All nine tools return `Json<ListResult<T>>`; `tools/list` carries an `outputSchema` for each.
- `tests/tool_surface.rs` walks every tool and passes; the tool count is asserted.
- `tests/tool_wire_shape.rs` asserts the envelope, the unchanged object shape, and the unchanged text shape.
- `cargo test -p zotero-mcp` fully green; no new clippy warnings; `cargo fmt` clean.
- The three lookup tools and the 13 text tools are byte-identical in behaviour.
- README and CHANGELOG record the new response shape for the nine.
- No commit, no reinstall, no launchd restart — Richard's call.
