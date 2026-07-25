# Slice G completion (list envelope) — Implementation Plan

**Goal:** the nine list-returning tools return `Json<ListResult<T>>`; every tool's schema is walked by a test; the wire shape is asserted. Tests before implementation, and the two new tests come *first* because they are the guard for everything after.

**Spec:** `docs/superpowers/specs/2026-07-25-slice-g-list-envelope-design.md`.
**Decision brief:** `docs/superpowers/specs/2026-07-25-slice-g-wire-format-decision.md`.

**Constraint:** transport files (`bearer.rs`, `oauth.rs`, `oauth/token_store.rs`, `http_transport.rs`, `setup.rs`) are out of bounds and not needed.

---

## Task 1: The guard tests (written and passing BEFORE any migration)

These two must pass against the *current* code first. That proves they work, and it means any failure after the migration is caused by the migration.

- [ ] **Step 1 — `tests/tool_surface.rs`.** Build the real router (`ZoteroServer::tool_router()`, which is where rmcp evaluates output schemas and panics on a bad one) and `list_all()`. For every tool assert:
  - `input_schema["type"] == "object"`;
  - `output_schema`, when present, has `["type"] == "object"`;
  - a non-empty `description`;
  - `annotations.is_some()` with `read_only_hint` set.

  Plus `assert_eq!(tools.len(), 35)` with a comment saying to update it deliberately when adding a tool. Also assert a couple of named tools exist, so a rename is visible.

- [ ] **Step 2 — `tests/tool_wire_shape.rs`.** Build an `AppState` against the fixture:

```rust
let f = fixtures::build_fixture::build();
let mut cfg = Config::default();
cfg.zotero.data_dir = f.dir.path().to_string_lossy().into_owned();
cfg.zotero.user_id = 1; // pinned: 0 would trigger a local-API detection call
let state = AppState::build(cfg).await.expect("fixture AppState");
```

  Then call tools directly and convert via `rmcp::handler::server::tool::IntoCallToolResult`, asserting on the real `CallToolResult`:
  - `search_items` — `content[0].text` parses as an object with `items` (array), `count`, `possibly_truncated`; `structured_content` is the same object. **Before migration this test asserts the bare-array shape**; the migration flips the expectation, and that flip is the reviewable record of the wire change.
  - `get_item` — unchanged object shape, `structured_content` present (regression guard for the 10 already-migrated tools).
  - `format_citation` or `get_pdf_path` — still bare text, `structured_content` is `None` (regression guard for the 13 text tools).

- [ ] **Step 3 — run both; both must pass on unmodified code.** If `AppState::build` can't be constructed against the fixture, stop and report — the wire test is not optional in this slice.

## Task 2: Migration (red → green)

- [ ] **Step 4 — flip the wire-test expectation** for `search_items` to the envelope shape. It now fails. That is the red.

- [ ] **Step 5 — `core/reader/search.rs`:** add `pub const DEFAULT_SEARCH_LIMIT: i64 = 50;` and use it in `search_metadata`'s `if params.limit <= 0` branch instead of the literal.

- [ ] **Step 6 — `tools/wire.rs`** (new):

```rust
pub struct ListResult<T> { items, count, possibly_truncated }
impl<T> ListResult<T> {
    /// Everything the library holds — no limit was applied.
    pub fn complete(items: Vec<T>) -> Self
    /// A limited query: flags possible truncation when the row count reached the limit.
    pub fn with_limit(items: Vec<T>, limit: i64) -> Self
}
```

  Unit-test both constructors, including the `count == limit` boundary and `limit <= 0`.

- [ ] **Step 7 — `core/enrichment/propose.rs`:** add `WeakMetadataItem { item_key, weak_fields }`, change `find_weak_metadata_items` to `Result<Vec<WeakMetadataItem>>`, update its internal callers and any test that reads the tuple shape.

- [ ] **Step 8 — migrate the nine `_t` functions** in `tools/search.rs`, `tools/attachments.rs`, `tools/enrichment.rs`. Pattern:

```rust
pub async fn search_items(s: &AppState, a: SearchArgs) -> Result<Json<ListResult<SearchHit>>, Error> {
    let limit = a.limit;
    let hits = search_metadata(...).await.map_err(map_err)?;
    Ok(Json(ListResult::with_limit(hits, limit)))
}
```

  Limit-aware: `search_items`, `list_recent_items`, `search_crossref`, `search_semantic_scholar`, `find_weak_metadata_items`. `ListResult::complete`: `list_collections`, `list_tags`, `list_attachments`, `list_annotations`.

- [ ] **Step 9 — migrate the nine `server.rs` wrappers** to `Result<Json<ListResult<T>>, McpError>`, importing the element types. Descriptions and `annotations(...)` blocks unchanged. Drop `Content` from a file's imports only if no `Content::text` site remains in it.

- [ ] **Step 10 — build; then run `tests/tool_surface.rs` first.** It is the startup-panic guard: if a schema is wrong, this is where it surfaces, and the panic message names the offending type.

- [ ] **Step 11 — full `cargo test -p zotero-mcp`, `cargo fmt -p zotero-mcp`, `cargo clippy -p zotero-mcp --all-targets`.** Green, formatted, and no clippy warning pointing at a file this slice touched.

## Task 3: Docs

- [ ] **Step 12 — README:** note the response shape for the nine list tools (`{items, count, possibly_truncated}`) in the Tools section, once, rather than repeating it per row. Note that `find_weak_metadata_items` items are now named objects.
- [ ] **Step 13 — CHANGELOG:** `[Unreleased] → Changed`, describing the shape change, why (the object-root requirement), and the truncation signal it buys. Flag it as a response-shape change for those nine tools so it is not missed at release time.
- [ ] **Step 14 — mark the old Slice G spec resolved:** append a short note to `2026-05-13-dependency-upgrades-slice-g-design.md` pointing at the decision brief and this spec, so the "deferred pending wire-format policy" section is not read as still open.

## Hand-off

- [ ] Report to Richard: tools migrated, test counts, and what the first reinstall should show (`tools/list` with 35 tools, `outputSchema` on 19).
- [ ] **No commit, no `cargo install`, no launchd restart.** Richard's call. The first reinstall after this change alters the tool list, so it is worth watching that Claude Code still shows the tools.
