# Spec: Dependency upgrades — Slice F (annotate the 34 MCP tools with MCP-spec hints)

**Status:** Approved design, ready for plan-writing.
**Author:** Richard Lyon (with Claude Opus 4.7).
**Date:** 2026-05-13.
**Goal:** Add `annotations(...)` blocks to every `#[tool(...)]` macro in `crates/zotero-mcp/src/server.rs` so each tool's `tools/list` entry advertises whether it is read-only, destructive, idempotent, and/or open-world. Lands as one atomic commit; mechanical edits only.

---

## Problem

`crates/zotero-mcp/src/server.rs` defines 34 `#[tool(...)]` macros (verified by direct file read on 2026-05-13 — the campaign brief's "40" was an approximation). None set `annotations(...)`. Without annotations, clients (Claude Cowork, Claude Desktop, anything that consumes `tools/list`) have no signal about which calls are safe to retry, which mutate state, which are destructive, or which hit external services.

rmcp 1.7's `#[tool]` macro accepts a nested `annotations(...)` block — confirmed via `rmcp-1.7.0/tests/test_tool_macro_annotations.rs` and `rmcp-macros-1.7.0/src/tool.rs` lines 244–267. Fields are:

- `title: Option<String>` — human-readable display name.
- `read_only_hint: Option<bool>` — if true, the tool does not modify state.
- `destructive_hint: Option<bool>` — if true, the mutation is non-additive (only meaningful when `read_only_hint == false`).
- `idempotent_hint: Option<bool>` — if true, repeat calls with same args are no-ops (only meaningful when `read_only_hint == false`).
- `open_world_hint: Option<bool>` — if true, the tool interacts with external/uncontrolled systems.

The boolean hints are cheap to set, mechanical, and per MCP spec are truth-claims clients can rely on for retry / confirmation-prompt / sandboxing behaviour. Slice F sets all four booleans on every tool; `title` is deferred.

---

## Decisions

1. **Set all four boolean hints, no `title`.** Each tool gets an explicit `read_only_hint` and `open_world_hint`. Mutating tools additionally get `destructive_hint` and `idempotent_hint`. Read-only tools omit `destructive_hint` and `idempotent_hint` because the MCP spec says they're "only meaningful when readOnlyHint == false" — keeping them absent matches the spec's intent rather than emitting fields the client should ignore.

2. **`refetch_url`: conservative classification.** Despite the campaign brief listing it under read-only, the tool takes `save_as_snapshot: bool` (verified — `crates/zotero-mcp/src/tools/attachments.rs`, struct `RefetchArgs`). When `save_as_snapshot=true` it creates a new attachment. Conservative classification:
   - `read_only_hint = false`, `destructive_hint = false`, `idempotent_hint = false`, `open_world_hint = true`.

   Rationale: a client that trusts `read_only_hint = true` may retry on failure, double-attaching. Truth-claims beat convenience.

3. **`delete_item`: destructive AND idempotent.**
   - `read_only_hint = false`, `destructive_hint = true`, `idempotent_hint = true`, `open_world_hint = false`.

   Trash is recoverable but the item disappears from the active library — destructive in any client-meaningful sense. Calling twice with the same key is a no-op once the item is in trash — idempotent.

4. **`get_webpage_content` and `enrich_item`: open-world.** `get_webpage_content` with `mode=live` fetches an external URL. `enrich_item` is composite — internally invokes external lookups. Conservative: mark both `open_world_hint = true`.

5. **External-API lookups: open-world read-only.** `lookup_doi`, `lookup_isbn`, `lookup_arxiv`, `search_crossref`, `search_semantic_scholar` all hit external HTTP APIs but mutate nothing. `read_only_hint = true`, `open_world_hint = true`.

6. **Set-style mutations: idempotent.** `add_tags`, `remove_tags`, `add_to_collection`, `remove_from_collection`, `update_item_fields`, `delete_item` — repeat calls with same args converge to the same state. `idempotent_hint = true`.

7. **Create-style mutations: non-idempotent.** `add_note`, `create_item`, `attach_link`, `attach_file`, `apply_metadata_update`, `enrich_item` — each call creates new state or re-applies a proposal. `idempotent_hint = false`.

8. **One smoke test.** Asserts four representative annotations are correctly emitted using the macro-generated `<fn_name>_tool_attr()` accessor (pattern verified in `rmcp-1.7.0/tests/test_tool_macro_annotations.rs`):
   - `ping.read_only_hint == Some(true)`
   - `delete_item.destructive_hint == Some(true)` and `delete_item.idempotent_hint == Some(true)`
   - `lookup_doi.open_world_hint == Some(true)`
   - `add_tags.idempotent_hint == Some(true)`

   `server.rs` does not currently have a `#[cfg(test)] mod tests` block — the test introduces one (~25 lines incl. module scaffolding).

9. **Single atomic commit directly on `main`, no PR, no force push.** Matches the Slice B/C/E pattern.

   Commit message format:

   ```
   chore(tools): annotate the 34 MCP tools with MCP-spec hints (read_only / destructive / idempotent / open_world)
   ```

10. **Test gate.** `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp` both pass. Lib-test count delta: +1 (the new smoke test). Note the new number in the commit body.

11. **Reinstall + launchd restart NOT performed by the implementer.** Consistent with the campaign pattern. After the commit lands and tests pass, the user decides when to `cargo install --path crates/zotero-mcp` + `launchctl kickstart -k gui/$(id -u)/com.zotero-mcp.http`.

---

## Classification table — all 34 tools

`—` means the field is intentionally omitted because the MCP spec says it's only meaningful when `read_only_hint == false`.

| Tool | read_only | destructive | idempotent | open_world |
|---|---|---|---|---|
| `ping` | true | — | — | false |
| `search_items` | true | — | — | false |
| `get_item` | true | — | — | false |
| `list_collections` | true | — | — | false |
| `list_tags` | true | — | — | false |
| `list_recent_items` | true | — | — | false |
| `list_attachments` | true | — | — | false |
| `get_pdf_path` | true | — | — | false |
| `get_pdf_text` | true | — | — | false |
| `get_pdf_first_pages` | true | — | — | false |
| `list_annotations` | true | — | — | false |
| `get_webpage_content` | true | — | — | **true** |
| `refetch_url` | **false** | false | false | true |
| `format_citation` | true | — | — | false |
| `format_bibliography` | true | — | — | false |
| `find_weak_metadata_items` | true | — | — | false |
| `lookup_doi` | true | — | — | **true** |
| `lookup_isbn` | true | — | — | **true** |
| `lookup_arxiv` | true | — | — | **true** |
| `search_crossref` | true | — | — | **true** |
| `search_semantic_scholar` | true | — | — | **true** |
| `propose_metadata_update` | true | — | — | false |
| `add_note` | false | false | false | false |
| `update_item_fields` | false | false | **true** | false |
| `add_tags` | false | false | **true** | false |
| `remove_tags` | false | false | **true** | false |
| `add_to_collection` | false | false | **true** | false |
| `remove_from_collection` | false | false | **true** | false |
| `delete_item` | false | **true** | **true** | false |
| `create_item` | false | false | false | false |
| `attach_link` | false | false | false | false |
| `attach_file` | false | false | false | false |
| `apply_metadata_update` | false | false | false | false |
| `enrich_item` | false | false | false | **true** |

Totals: 21 read-only (15 closed-world + 6 open-world), 13 mutating (6 idempotent + 7 non-idempotent), of which 1 is destructive. The 8 open-world tools comprise the 5 external lookups + `get_webpage_content` + `refetch_url` + `enrich_item`.

---

## Migration plan

The implementation is mechanical: append an `annotations(...)` block to each existing `#[tool(...)]` macro per the classification table. No other files touched.

### 1. `crates/zotero-mcp/src/server.rs`

For each of the 34 tools, extend the `#[tool(...)]` attribute to include `annotations(...)`. Two example transformations:

```rust
// Before — read-only, closed-world
#[tool(description = "List all collections in the user's library.")]
pub async fn list_collections(...) { ... }

// After
#[tool(
    description = "List all collections in the user's library.",
    annotations(read_only_hint = true, open_world_hint = false)
)]
pub async fn list_collections(...) { ... }
```

```rust
// Before — destructive mutation
#[tool(
    description = "Move an item (regular item, note, or attachment) to Zotero's trash. \
                   Recoverable: items remain in the library until the trash is emptied. \
                   Use this when the user explicitly asks to delete something."
)]
pub async fn delete_item(...) { ... }

// After
#[tool(
    description = "Move an item (regular item, note, or attachment) to Zotero's trash. \
                   Recoverable: items remain in the library until the trash is emptied. \
                   Use this when the user explicitly asks to delete something.",
    annotations(
        read_only_hint = false,
        destructive_hint = true,
        idempotent_hint = true,
        open_world_hint = false,
    )
)]
pub async fn delete_item(...) { ... }
```

Preserve every existing `description = "..."` exactly. Multi-line continued descriptions (which use `\` line-continuation) keep their existing form.

### 2. Smoke test

Add a `#[cfg(test)] mod tests` block at the bottom of `server.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::ZoteroServer;

    #[test]
    fn tool_annotations_present_on_representative_tools() {
        let ann = ZoteroServer::ping_tool_attr()
            .annotations
            .expect("ping should carry annotations");
        assert_eq!(ann.read_only_hint, Some(true));

        let ann = ZoteroServer::delete_item_tool_attr()
            .annotations
            .expect("delete_item should carry annotations");
        assert_eq!(ann.destructive_hint, Some(true));
        assert_eq!(ann.idempotent_hint, Some(true));

        let ann = ZoteroServer::lookup_doi_tool_attr()
            .annotations
            .expect("lookup_doi should carry annotations");
        assert_eq!(ann.open_world_hint, Some(true));

        let ann = ZoteroServer::add_tags_tool_attr()
            .annotations
            .expect("add_tags should carry annotations");
        assert_eq!(ann.idempotent_hint, Some(true));
    }
}
```

The `<fn_name>_tool_attr()` accessor is generated by `#[tool]` and returns `rmcp::model::Tool` (verified via `rmcp-1.7.0/tests/test_tool_macro_annotations.rs`).

### 3. No other files

`Cargo.toml`, `Cargo.lock`, every other source file: untouched. If the implementer finds a need to touch anything else, that triggers escalation.

---

## Risks

1. **Annotation-block syntax mismatch.** The expected syntax (`annotations(field = value, ...)` as a nested key inside `#[tool(...)]`) was verified by reading `rmcp-macros-1.7.0/src/tool.rs` lines 244–267 and the live test in `rmcp-1.7.0/tests/test_tool_macro_annotations.rs`. If a compile error surfaces (e.g., wrong field name, missing trailing comma rules, syn parser quirk), the fix is at the macro source above — escalate if mechanical fix is >10 lines or requires touching files outside `server.rs`.

2. **Long-line lint or rustfmt rewrap.** Some of the mutating tools have descriptions that span multiple lines today (e.g., `lookup_doi`, `delete_item`, `enrich_item`). Adding the annotations block on top may push the macro over rustfmt's preferred width. Acceptable: rustfmt will rewrite the macro call across multiple lines — that's a cosmetic, not semantic, change. If `cargo fmt -- --check` (or pre-commit hook) complains, run `cargo fmt -p zotero-mcp` and include the result in the commit.

3. **`#[cfg(test)] mod tests` collides with an existing test elsewhere.** `server.rs` has no test module today (verified). The new module is at the bottom of the file. If a future tool reorganisation moves tests, the module relocates with no semantic impact.

4. **`enrich_item` classification.** I'm marking `enrich_item` as `open_world_hint = true` because internally it can invoke external lookups (per the description and the composite propose+apply behaviour). The conservative truth-claim. If a reader disagrees with this judgement they can argue from the description text — the spec is explicit.

5. **`refetch_url` reclassification away from your brief's "read-only" list.** Brief listed it under read-only; closer inspection of `RefetchArgs` (verified) shows `save_as_snapshot: bool` triggers an attachment write. Spec is conservative — see Decision 2.

---

## Verification checklist (end of slice)

- [ ] One commit lands on `main` with message format `chore(tools): annotate the 34 MCP tools with MCP-spec hints (read_only / destructive / idempotent / open_world)`.
- [ ] The commit passes `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp`.
- [ ] The new lib-test count is explicitly noted in the commit body (delta: +1 from prior Slice E baseline).
- [ ] `git diff --stat` shows only `crates/zotero-mcp/src/server.rs` modified.
- [ ] All 34 `#[tool(...)]` macros carry an `annotations(...)` block; spot-check 3-4 in the diff (one read-only-closed, one read-only-open-world, one set-style mutation, `delete_item`).
- [ ] Smoke test exists at the bottom of `server.rs` and passes.
- [ ] No `cargo install` and no `launchctl kickstart` executed inside the slice.

---

## Out of scope (deferred)

- **`title` field on annotations.** Adds a human-readable display name per tool. Useful when MCP-client UIs render the tool picker (Claude Desktop, future Cowork UIs). Defer to a follow-up if/when those UIs make it visible. Scope: ~34 short strings.
- **Resource annotations.** `list_resources` returns 2 resources via `RawResource::new(...).no_annotation()`. The MCP spec also defines annotations on resources (audience, priority); separate from tool annotations. Out of this slice's bounds.
- **Slice G** — replace `Ok(CallToolResult::success(vec![Content::json(...)]))` boilerplate with `Ok(Json(result))` using `rmcp::handler::server::wrapper::Json`. Touches all 5 `tools/*.rs` files and introduces typed output structs. Next slice.
- **Slice H** — per-field doc comments → schemars schema descriptions. 27 Args structs across the same 5 files. Incremental; can be done in passes. Final slice.

---

## Decisions deferred to implementation

- The exact compile behaviour of `annotations(...)` with trailing commas vs. no trailing commas inside the block. Both are likely accepted by the macro's `syn` parser, but pick one for consistency. (Default to trailing commas — matches rustfmt's preferred style.)
- Whether `cargo fmt` rewraps the now-multi-line macro calls in a way the user finds noisy. Implementer may run fmt and include the formatted result in the same commit, or leave unformatted — but consistency across the 34 sites is required.
