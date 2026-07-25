# `find_duplicates` — Implementation Plan

**Goal:** ship a read-only `find_duplicates` MCP tool that runs the whole dedup gate deterministically and returns a triage. Tests first.

**Spec:** `docs/superpowers/specs/2026-07-25-find-duplicates-design.md`.

**Constraint:** transport files (`bearer.rs`, `oauth.rs`, `oauth/token_store.rs`, `http_transport.rs`, `setup.rs`) are out of bounds and not needed.

---

## Task 1: Fixture (prerequisite for the failing tests)

- [ ] **Step 1 — extend `tests/fixtures/build_fixture.rs`:**
  - Add `CREATE TABLE deletedItems (itemID INTEGER PRIMARY KEY, dateDeleted TIMESTAMP)`.
  - Add fields `ISBN` (fieldID 11) and `place` (fieldID 7) to `fields` / `fieldsCombined`.
  - Add an ISBN (`9781844674879`) to the existing Rabkin book (item 1, key `JGF2UTMW`, title `What is Modern Israel?`, creator first name deliberately misspelt `Yakob`).
  - Add item 7, key `GAZA0001`: book `Gaza: An inquest into its martyrdom`, 2018, creator `Rabkin` — **no attachment**. Gives both the punctuation case and the same-author-flooding case.
  - Add item 8, key `TRSH0001`: book `What is Modern Israel?` by `Rabkin`, and a `deletedItems` row for it. Gives the trash case.
  - Keep every existing row and key unchanged.

- [ ] **Step 2 — `cargo test -p zotero-mcp`** to confirm the fixture additions break nothing.

## Task 2: Tests (red)

- [ ] **Step 3 — `tests/dedup.rs`**, new file, tests 1–11 from the spec's test list. Unit tests for the pure functions go in `#[cfg(test)] mod tests` inside `core/dedup.rs` and `core/isbn.rs`; the integration tests (3, 4, 5, 6, 7, 8, 9) go in `tests/dedup.rs` against `fixtures::build_fixture::build()`, following the `reader_search.rs` style.
- [ ] **Step 4 — confirm red** (compile failure counts).

## Task 3: Implementation (green)

- [ ] **Step 5 — `core/isbn.rs`:** `normalise_isbn`, `isbn10_to_13`, `isbn13_to_10`, `isbn_variants` (given form → deduped list of plausible forms). Pure, no I/O.

- [ ] **Step 6 — `core/reader/trash.rs`:** `trashed_keys(pool, library_id, keys: &[String]) -> Result<HashSet<String>>`, one `SELECT i.key FROM deletedItems d JOIN items i ON i.itemID = d.itemID WHERE i.libraryID = ?` filtered in Rust (the key list is small). Tolerate a missing `deletedItems` table by returning an empty set — some minimal fixtures won't have it.

- [ ] **Step 7 — `core/dedup.rs`, pure layer:**
  - `normalise_title(&str) -> String` (spec Decision 4 order).
  - `significant_tokens(&str) -> Vec<String>` (len ≥ 4, stop-words dropped, deduped, longest-first).
  - `title_similarity(a_tokens, b_tokens) -> f64` (Jaccard over token sets; 1.0 for two empty sets is meaningless, so return 0.0 if either side is empty).
  - `shared_token_count(a_tokens, b_tokens) -> usize`.

- [ ] **Step 8 — `core/dedup.rs`, orchestration:** `find_duplicates(pool, library_id, storage_dir, input) -> Result<FindDuplicatesResult>`:
  1. Normalise the title, derive tokens.
  2. Pass A: up to `MAX_TITLE_QUERIES` (6) `search_metadata` calls, longest token first; union by key; keep hits with `shared_token_count ≥ MIN_SHARED_TOKENS`. (Raised from 3 during implementation — see the spec's Decision 6 note.)
  3. Pass B: `search_metadata(surname)`; when a title was given, drop hits sharing no significant token.
  4. Pass C: one `search_metadata` per identifier variant.
  5. Union all by key, recording `found_by`; drop trashed keys; drop attachment/note item types (already excluded by `search_metadata`).
  6. Per candidate: `get_item_by_key` + `list_attachments`; compute `title_similarity`, `triage`, `triage_reason`, `metadata_diff` (triage `ii` only), `default_action`.
  7. Resolve `recommendation` by precedence; fill `possible_stub_duplicates` when a `"i"` wins.
  Record every query in `queries_run` with `result_count` and `kept`.

- [ ] **Step 9 — `tools/dedup.rs`:** `FindDuplicatesArgs` (with per-field doc comments — they become the schema descriptions the model reads), the result types deriving `Serialize + JsonSchema`, and `find_duplicates_t` returning `Json<FindDuplicatesResult>`. Validate "at least one of title/author_surname/identifier" → `invalid_params`.

- [ ] **Step 10 — register:** `core/mod.rs` (+`dedup`, `isbn`, `identifier` later), `core/reader/mod.rs` (+`trash`), `tools/mod.rs` (+`dedup`), and `server.rs`:

```rust
#[tool(
    description = "Check whether a work is already in the library BEFORE creating an item. \
        Runs three passes over the local library — title tokens, author surname, and \
        identifier (DOI/ISBN/arXiv, all plausible forms) — unions the results, excludes \
        trashed items, and returns a triage per candidate with a recommended action. \
        Call this first when adding any reference. Input: { title?, author_surname?, \
        identifier?, input_kind: \"pdf\"|\"url\"|\"name\", limit? } — at least one of \
        title / author_surname / identifier. Returns { queries_run, candidates, \
        recommendation }: `abort` (already there, same kind of attachment), \
        `attach_to_existing` (record exists but lacks your file), `ask` (weak match — \
        put it to the user), `create_new` (nothing found).",
    annotations(read_only_hint = true, open_world_hint = false)
)]
```

- [ ] **Step 11 — build, test, fmt, clippy.** Then `cargo test -p zotero-mcp` fully green.

## Task 4: Docs

- [ ] **Step 12 — README:** add `find_duplicates` to the Read tool table (it reads only), with one line on why it exists. Tool count 34 → 35 wherever a count appears.
- [ ] **Step 13 — CHANGELOG:** `[Unreleased] → Added`, naming the two historical failures it closes.

## Hand-off

- [ ] Report to Richard. **No commit, no `cargo install`, no launchd restart.**
