# Dependency Upgrades — Slice F Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `annotations(...)` blocks to all 34 `#[tool(...)]` macros in `crates/zotero-mcp/src/server.rs`, setting four MCP-spec hints (`read_only_hint`, `destructive_hint`, `idempotent_hint`, `open_world_hint`) per the classification table in the spec. Add one smoke test that verifies four representative annotations. Lands as one atomic commit on `main`.

**Architecture:** Mechanical macro edits, single file. Each existing `#[tool(description = "...")]` attribute is extended with `annotations(...)` whose contents are dictated by a lookup table (one row per tool). Read-only tools get two fields (`read_only_hint = true` + `open_world_hint`). Mutating tools get four (`read_only_hint = false` + `destructive_hint` + `idempotent_hint` + `open_world_hint`). No behaviour change; the annotations flow into the `tools/list` response so MCP clients can reason about retry/destructive/sandboxing semantics.

**Tech Stack:** Rust, Cargo. rmcp 1.7's `#[tool]` macro (in `rmcp-macros-1.7.0`) — accepts a nested `annotations(...)` block.

**Spec:** `docs/superpowers/specs/2026-05-13-dependency-upgrades-slice-f-design.md` (commit `3bee636`).

---

## File Structure

| File | Purpose | Action |
|---|---|---|
| `crates/zotero-mcp/src/server.rs` | 34 `#[tool(...)]` macros need `annotations(...)`; new `#[cfg(test)] mod tests` block at bottom | Modify (single file, ~120 lines added) |

All other files in the workspace are verify-only — if any other file shows up in `git diff` at the end, that's an out-of-bounds change.

---

## Annotation lookup table

This is the single source of truth for what each of the 34 tools' `annotations(...)` block must contain. Steps 5–9 refer back to this table.

Notation in the table:
- `RO` = `read_only_hint = true, open_world_hint = false`
- `RO+open` = `read_only_hint = true, open_world_hint = true`
- `Mut` = `read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false`
- `Mut+idem` = `read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false`
- Special rows spell out their full block.

| Tool | Pattern |
|---|---|
| `ping` | RO |
| `search_items` | RO |
| `get_item` | RO |
| `list_collections` | RO |
| `list_tags` | RO |
| `list_recent_items` | RO |
| `list_attachments` | RO |
| `get_pdf_path` | RO |
| `get_pdf_text` | RO |
| `get_pdf_first_pages` | RO |
| `list_annotations` | RO |
| `format_citation` | RO |
| `format_bibliography` | RO |
| `find_weak_metadata_items` | RO |
| `propose_metadata_update` | RO |
| `get_webpage_content` | RO+open |
| `lookup_doi` | RO+open |
| `lookup_isbn` | RO+open |
| `lookup_arxiv` | RO+open |
| `search_crossref` | RO+open |
| `search_semantic_scholar` | RO+open |
| `update_item_fields` | Mut+idem |
| `add_tags` | Mut+idem |
| `remove_tags` | Mut+idem |
| `add_to_collection` | Mut+idem |
| `remove_from_collection` | Mut+idem |
| `add_note` | Mut |
| `create_item` | Mut |
| `attach_link` | Mut |
| `attach_file` | Mut |
| `apply_metadata_update` | Mut |
| `refetch_url` | `read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = true` |
| `enrich_item` | `read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = true` |
| `delete_item` | `read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false` |

Counts: 15 × RO, 6 × RO+open, 5 × Mut+idem, 5 × Mut, 2 × open-world mutating (`refetch_url`, `enrich_item`), 1 × destructive (`delete_item`) = 34.

---

## Pre-flight: confirm clean state

**Files:**
- Read-only: working tree

- [ ] **Step 1: Confirm clean tree on `main`**

Run: `cd /Users/rjl/Code/github/zotero-connector && git status`

Expected: `nothing to commit, working tree clean` and branch `main`.

If dirty, stop and resolve before starting the slice.

- [ ] **Step 2: Capture baseline test results**

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo test -p zotero-mcp 2>&1 | grep "^test result:" | sort | uniq -c`

Expected: every line `ok`, no `FAILED`. Lib tests pass at the post-Slice-E baseline. Write the number down — Slice F adds exactly +1 lib test (the new smoke test). The new total gets recorded in the commit body.

- [ ] **Step 3: Record the pre-flight SHA**

Run: `cd /Users/rjl/Code/github/zotero-connector && git rev-parse HEAD`

Expected: `3bee636` (the Slice F spec commit) — or, if the slice is re-run after a checkpoint, whatever HEAD is. Write down the SHA. This is the rollback point if the slice escalates.

---

## Task 1: Annotate the 34 tools and add a smoke test

**Files (all changes land in one commit at Step 13):**
- Modify: `crates/zotero-mcp/src/server.rs`

### Step 1: Read the current server.rs

Read `/Users/rjl/Code/github/zotero-connector/crates/zotero-mcp/src/server.rs`. The 34 tools live between lines 36–278 (current HEAD). Each macro currently has the form:

```rust
#[tool(description = "...")]
pub async fn <name>(...) -> Result<CallToolResult, McpError> { ... }
```

Some descriptions span multiple lines via the `\` line-continuation form — preserve those exactly.

### Step 2: Confirm the rmcp macro syntax

Read `/Users/rjl/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rmcp-1.7.0/tests/test_tool_macro_annotations.rs` (a few dozen lines, ground truth for the annotations syntax). Confirm the example:

```rust
#[tool(
    name = "direct-annotated-tool",
    annotations(title = "Annotated Tool", read_only_hint = true)
)]
```

That is the exact pattern Slice F uses (minus `title`, plus the four boolean fields). Field names: `read_only_hint`, `destructive_hint`, `idempotent_hint`, `open_world_hint` (confirmed in `rmcp-macros-1.7.0/src/tool.rs` lines 244–267 and `rmcp-1.7.0/src/model/tool.rs` lines 122–150).

### Step 3: Worked example for each pattern

The four edit patterns are shown end-to-end below. The rest of the tools follow these examples exactly — only the wrapped tool name and the existing `description = "..."` change.

#### Pattern RO (read-only, closed-world)

```rust
// Before
#[tool(description = "List all collections in the user's library.")]
pub async fn list_collections(
    &self,
    Parameters(args): Parameters<EmptyArgs>,
) -> Result<CallToolResult, McpError> {
    search::list_collections(&self.state, args).await
}

// After
#[tool(
    description = "List all collections in the user's library.",
    annotations(read_only_hint = true, open_world_hint = false)
)]
pub async fn list_collections(
    &self,
    Parameters(args): Parameters<EmptyArgs>,
) -> Result<CallToolResult, McpError> {
    search::list_collections(&self.state, args).await
}
```

#### Pattern RO+open (read-only, open-world)

```rust
// Before
#[tool(description = "Search Semantic Scholar by free-text query; returns normalized candidates.")]
pub async fn search_semantic_scholar(...) { ... }

// After
#[tool(
    description = "Search Semantic Scholar by free-text query; returns normalized candidates.",
    annotations(read_only_hint = true, open_world_hint = true)
)]
pub async fn search_semantic_scholar(...) { ... }
```

#### Pattern Mut+idem (mutating, idempotent — set-style)

```rust
// Before
#[tool(description = "Add tags to an item (deduplicates against existing tags).")]
pub async fn add_tags(&self, Parameters(args): Parameters<TagArgs>) -> Result<CallToolResult, McpError> {
    wr::add_tags_t(&self.state, args).await
}

// After
#[tool(
    description = "Add tags to an item (deduplicates against existing tags).",
    annotations(
        read_only_hint = false,
        destructive_hint = false,
        idempotent_hint = true,
        open_world_hint = false,
    )
)]
pub async fn add_tags(&self, Parameters(args): Parameters<TagArgs>) -> Result<CallToolResult, McpError> {
    wr::add_tags_t(&self.state, args).await
}
```

#### Pattern Mut (mutating, non-idempotent — create-style)

```rust
// Before
#[tool(description = "Attach a markdown/HTML note to a Zotero item (markdown converted to simple HTML).")]
pub async fn add_note(&self, Parameters(args): Parameters<AddNoteArgs>) -> Result<CallToolResult, McpError> {
    wr::add_note_t(&self.state, args).await
}

// After
#[tool(
    description = "Attach a markdown/HTML note to a Zotero item (markdown converted to simple HTML).",
    annotations(
        read_only_hint = false,
        destructive_hint = false,
        idempotent_hint = false,
        open_world_hint = false,
    )
)]
pub async fn add_note(&self, Parameters(args): Parameters<AddNoteArgs>) -> Result<CallToolResult, McpError> {
    wr::add_note_t(&self.state, args).await
}
```

#### Special row: `delete_item`

```rust
// Before
#[tool(
    description = "Move an item (regular item, note, or attachment) to Zotero's trash. \
                   Recoverable: items remain in the library until the trash is emptied. \
                   Use this when the user explicitly asks to delete something."
)]
pub async fn delete_item(&self, Parameters(args): Parameters<DeleteItemArgs>) -> Result<CallToolResult, McpError> {
    wr::delete_item_t(&self.state, args).await
}

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
pub async fn delete_item(&self, Parameters(args): Parameters<DeleteItemArgs>) -> Result<CallToolResult, McpError> {
    wr::delete_item_t(&self.state, args).await
}
```

#### Special rows: `refetch_url` and `enrich_item`

Both have `read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = true`. Same shape as Pattern Mut but with `open_world_hint = true`. Example for `refetch_url`:

```rust
// Before
#[tool(description = "Re-fetch a webpage item live, optionally saving a fresh HTML snapshot as an attachment.")]
pub async fn refetch_url(
    &self,
    Parameters(args): Parameters<RefetchArgs>,
) -> Result<CallToolResult, McpError> {
    att::refetch_url_t(&self.state, args).await
}

// After
#[tool(
    description = "Re-fetch a webpage item live, optionally saving a fresh HTML snapshot as an attachment.",
    annotations(
        read_only_hint = false,
        destructive_hint = false,
        idempotent_hint = false,
        open_world_hint = true,
    )
)]
pub async fn refetch_url(
    &self,
    Parameters(args): Parameters<RefetchArgs>,
) -> Result<CallToolResult, McpError> {
    att::refetch_url_t(&self.state, args).await
}
```

`enrich_item` follows the same shape with its own (longer) description preserved verbatim.

### Step 4: Apply the RO pattern to all 15 read-only closed-world tools

For each tool below, extend its `#[tool(...)]` per the RO pattern shown in Step 3:

`ping`, `search_items`, `get_item`, `list_collections`, `list_tags`, `list_recent_items`, `list_attachments`, `get_pdf_path`, `get_pdf_text`, `get_pdf_first_pages`, `list_annotations`, `format_citation`, `format_bibliography`, `find_weak_metadata_items`, `propose_metadata_update`.

The block to append: `annotations(read_only_hint = true, open_world_hint = false)`.

Preserve the existing `description = "..."` exactly (including the `\` line-continuation form on multi-line ones — `lookup_doi`/`lookup_isbn`/`lookup_arxiv` are in Step 5, not here; `propose_metadata_update` has multi-line description, preserve it). Add a comma after the description and put the `annotations(...)` block on its own line (or wrapped across several lines for the multi-line annotation blocks).

### Step 5: Apply the RO+open pattern to all 6 read-only open-world tools

For each tool below:

`get_webpage_content`, `lookup_doi`, `lookup_isbn`, `lookup_arxiv`, `search_crossref`, `search_semantic_scholar`.

The block to append: `annotations(read_only_hint = true, open_world_hint = true)`.

Preserve the existing (often multi-line) `description = "..."` exactly.

### Step 6: Apply the Mut+idem pattern to all 5 idempotent mutating tools

For each tool below:

`update_item_fields`, `add_tags`, `remove_tags`, `add_to_collection`, `remove_from_collection`.

The block to append:

```rust
annotations(
    read_only_hint = false,
    destructive_hint = false,
    idempotent_hint = true,
    open_world_hint = false,
)
```

### Step 7: Apply the Mut pattern to all 5 non-idempotent mutating tools

For each tool below:

`add_note`, `create_item`, `attach_link`, `attach_file`, `apply_metadata_update`.

The block to append:

```rust
annotations(
    read_only_hint = false,
    destructive_hint = false,
    idempotent_hint = false,
    open_world_hint = false,
)
```

### Step 8: Apply the open-world mutating pattern to `refetch_url` and `enrich_item`

For both, append:

```rust
annotations(
    read_only_hint = false,
    destructive_hint = false,
    idempotent_hint = false,
    open_world_hint = true,
)
```

### Step 9: Apply the destructive pattern to `delete_item`

Append:

```rust
annotations(
    read_only_hint = false,
    destructive_hint = true,
    idempotent_hint = true,
    open_world_hint = false,
)
```

### Step 10: Add the smoke test at the bottom of server.rs

Append the following module immediately after the closing `}` of the `impl ServerHandler for ZoteroServer` block (i.e., at end of file). `server.rs` does not currently have a `#[cfg(test)] mod tests` block, so this introduces one.

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

Notes on the test:
- `<fn_name>_tool_attr()` is generated by the `#[tool]` macro. Returns `rmcp::model::Tool`. Verified in `rmcp-1.7.0/tests/test_tool_macro_annotations.rs:28`.
- `tool.annotations` is `Option<ToolAnnotations>`. `ToolAnnotations.read_only_hint / destructive_hint / idempotent_hint / open_world_hint` are each `Option<bool>`.
- The test deliberately does not assert on the closed-world (false) values to keep the assertion list short and focused on the positive signals.

### Step 11: Build

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo build -p zotero-mcp 2>&1 | tail -40`

Expected: clean build. No warnings about unused fields or unknown attribute keys.

Likely failure modes and fixes:

| Error | Fix |
|---|---|
| `unknown argument 'annotations'` in the macro | Macro syntax mismatch. Re-read `rmcp-macros-1.7.0/src/tool.rs:244–267`. The block is `annotations(...)` nested inside `#[tool(...)]`, not a separate `#[annotations(...)]` attribute. |
| `unknown field 'read_only_hint'` (or any other field) | Typo. Field names are exactly `read_only_hint`, `destructive_hint`, `idempotent_hint`, `open_world_hint`. |
| `expected literal, found identifier 'true'` | The values must be bare bools: `read_only_hint = true`, not `read_only_hint = "true"`. |
| `expected ',' or ')'` | Trailing comma after the last field inside the multi-line `annotations(...)` block is accepted; if the macro parser rejects, drop the trailing comma. (Test the single-line case `annotations(read_only_hint = true, open_world_hint = false)` first to confirm baseline syntax works.) |

Loop build + fix until clean. If a fix requires touching any file other than `crates/zotero-mcp/src/server.rs`, **escalate per the block below** — it's out of bounds.

### Step 12: Test

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo test -p zotero-mcp 2>&1 | tail -30`

Expected:
- All previously passing tests still pass.
- The new `tool_annotations_present_on_representative_tools` test passes.
- Net lib-test delta: +1.

Write down the new lib-test count for the commit body.

Failure modes:
- `ping_tool_attr` (or similar) is not found → the `<fn_name>_tool_attr()` accessor naming is wrong. Most likely it's `<fn_name>_tool_attr` literally, but if the macro shape is different (e.g., generates a `<fn_name>_tool` const), update the test accordingly. Read `rmcp-1.7.0/tests/test_tool_macro_annotations.rs` for ground truth.
- Test panics with `None` on `annotations.expect(...)` → the macro accepted the syntax but didn't propagate annotations into the generated `Tool`. Re-check the macro source — confirm the `annotations(...)` block is inside the outer `#[tool(...)]`, not adjacent to it.

### Step 13: Format (optional)

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo fmt -p zotero-mcp -- --check`

If `--check` exits non-zero, run: `cargo fmt -p zotero-mcp` and include the result in the same commit. Acceptable cosmetic outcome: the now-multi-line `#[tool(...)]` macro calls get rewrapped consistently. Required: consistency across the 34 sites.

### Step 14: Stage and confirm scope

Run:

```bash
cd /Users/rjl/Code/github/zotero-connector && \
git add crates/zotero-mcp/src/server.rs && \
git status --short
```

Expected staged set:
- `M crates/zotero-mcp/src/server.rs`

If anything else shows up (e.g., `Cargo.lock`, a `tools/*.rs`, `lib.rs`), that's an out-of-bounds change — either revert those files or **escalate**.

Spot-check the diff:

```bash
git diff --cached crates/zotero-mcp/src/server.rs | grep -E "^\+.*annotations\(" | wc -l
```

Expected: exactly 34 lines (one `annotations(` opening per tool).

```bash
git diff --cached crates/zotero-mcp/src/server.rs | grep -E "^\+.*destructive_hint = true" | wc -l
```

Expected: exactly 1 line (`delete_item`).

```bash
git diff --cached crates/zotero-mcp/src/server.rs | grep -E "^\+.*open_world_hint = true" | wc -l
```

Expected: exactly 8 lines (the 5 lookups + `get_webpage_content` + `refetch_url` + `enrich_item`).

### Step 15: Commit

Fill the `[N]` placeholder with the actual new lib-test count from Step 12.

```bash
git commit -m "$(cat <<'EOF'
chore(tools): annotate the 34 MCP tools with MCP-spec hints (read_only / destructive / idempotent / open_world)

Adds annotations(...) blocks to every #[tool(...)] in server.rs so
each tool's tools/list entry advertises read_only_hint,
destructive_hint, idempotent_hint, and open_world_hint per the
MCP spec. Clients (Claude Cowork, Claude Desktop) gain reliable
signals for retry semantics, destructive-confirmation prompts, and
sandboxing.

Classification (see spec):
  - 15 read-only, closed-world (search, get_item, list_*, get_pdf_*,
    list_annotations, format_*, find_weak_metadata_items,
    propose_metadata_update, ping)
  - 6 read-only, open-world (get_webpage_content + 5 external
    lookups: lookup_doi/isbn/arxiv, search_crossref/semantic_scholar)
  - 5 idempotent mutating (update_item_fields, add_tags, remove_tags,
    add_to_collection, remove_from_collection)
  - 5 non-idempotent mutating (add_note, create_item, attach_link,
    attach_file, apply_metadata_update)
  - 2 open-world mutating (refetch_url, enrich_item) — refetch_url
    reclassified from the campaign-brief read-only bucket because
    RefetchArgs.save_as_snapshot can create an attachment;
    enrich_item composes external lookups
  - 1 destructive (delete_item, with idempotent_hint = true because
    trash is recoverable and twice-call is a no-op)

No title field, no resource annotations — both deferred.

Smoke test (tool_annotations_present_on_representative_tools) asserts
four representative annotations via the macro-generated
<fn_name>_tool_attr() accessor. Lib-test count: [N] passed; 0 failed
(was [N-1] at Slice E baseline; delta: +1).

REINSTALL + LAUNCHD FLIP DELIBERATELY NOT PERFORMED in this commit.
User triggers `cargo install --path crates/zotero-mcp` and
`launchctl kickstart -k gui/$(id -u)/com.zotero-mcp.http` separately.

Spec: docs/superpowers/specs/2026-05-13-dependency-upgrades-slice-f-design.md
Plan: docs/superpowers/plans/2026-05-13-dependency-upgrades-slice-f.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Step 16: After commit, STOP — do not reinstall or restart launchd

Per the campaign pattern. Report `git rev-parse HEAD` to the controller. Confirm:

- ✅ Build clean.
- ✅ Tests pass at the documented count.
- ✅ Only `crates/zotero-mcp/src/server.rs` changed (per Step 14).
- ✅ 34 `annotations(` openings in the diff (per Step 14).
- ✅ Commit body's `[N]` placeholders filled.
- ❌ Do NOT run `cargo install --path crates/zotero-mcp`. The user will do that separately.

---

## Escalation block

Per spec Risks 1 and 4: the slice is mechanical-only. If any of these surface and resist a same-file fix:

1. The `annotations(...)` macro syntax doesn't accept one of the four field names (e.g., a rmcp-1.7.x release renamed `read_only_hint` to `read_only`). Verify against `rmcp-macros-1.7.0/src/tool.rs:244–267`.
2. The `<fn_name>_tool_attr()` accessor isn't generated, or returns a different type than expected. Verify against `rmcp-1.7.0/tests/test_tool_macro_annotations.rs`.
3. Building the smoke test requires changes to `lib.rs`, `Cargo.toml`, or any other file. The smoke test should compile against the existing crate; if it doesn't, the test design needs revision — not the public crate surface.
4. The macro reorders `description` + `annotations` in a way that breaks the existing description. Tool descriptions must be preserved exactly.

**Revert:**

```bash
cd /Users/rjl/Code/github/zotero-connector && \
git checkout -- crates/zotero-mcp/src/server.rs
```

Then amend the spec at `docs/superpowers/specs/2026-05-13-dependency-upgrades-slice-f-design.md` — append:

```markdown
---

## Deferred (Slice F annotations port)

[Date]: This slice was attempted and reverted. Blocker:

[One paragraph naming the specific macro behaviour, error, and reason
mechanical fixing wasn't possible. Reference the escalation-block
item number.]
```

Commit as: `docs(spec): defer Slice F — <one-line reason>`. Report DONE_WITH_CONCERNS.

---

## Hand-off

After Task 1 lands (commit on `main`, tests green, build clean):

- [ ] Implementer reports `git rev-parse HEAD` and the lib-test count.
- [ ] Two-stage review per the campaign pattern (spec-compliance reviewer, then code-quality reviewer).
- [ ] User decides when to `cargo install --path crates/zotero-mcp` and `launchctl kickstart -k gui/$(id -u)/com.zotero-mcp.http`. Not part of the slice.
- [ ] After install, user spot-checks `tools/list` from Cowork to confirm a known tool (e.g., `delete_item`) shows the new annotation metadata. Optional.

---

## Verification checklist

- [ ] One commit lands on `main` with message format `chore(tools): annotate the 34 MCP tools with MCP-spec hints (read_only / destructive / idempotent / open_world)`.
- [ ] The commit passes `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp`.
- [ ] The new lib-test count is explicitly noted in the commit body (+1 from prior baseline).
- [ ] `git diff --stat HEAD~1 HEAD` shows only `crates/zotero-mcp/src/server.rs` modified.
- [ ] Diff contains exactly 34 `annotations(` openings.
- [ ] Diff contains exactly 1 `destructive_hint = true` (delete_item).
- [ ] Diff contains exactly 8 `open_world_hint = true` (5 lookups + get_webpage_content + refetch_url + enrich_item).
- [ ] Smoke test passes.
- [ ] `cargo install` and `launchctl kickstart` were NOT executed by the implementer.
