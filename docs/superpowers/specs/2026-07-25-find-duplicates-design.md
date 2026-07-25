# Spec: `find_duplicates` — a composite dedup gate

**Status:** Approved design, ready for plan-writing.
**Author:** Richard Lyon (with Claude Opus 5).
**Date:** 2026-07-25.
**Provenance:** the 2026-05-13 `adding-references` eval pass. Original design note:
`~/Archive/pre-vault-cowork/project/obsidian/zotero-find-duplicates.spec.md`; failure modes recorded in the vault under *Skills* in `Projects/Obsidian Research Vaults.md`. Open Question #6 on the `Zotero MCP` hub.
**Goal:** one read-only MCP tool that performs the whole "is this already in the library?" check deterministically and returns a triage, so adding a paper that is already present is caught rather than silently duplicated.

---

## Problem

Step 2 of the `adding-references` skill is the dedup gate before any item is created, and it lives as procedural prose: strip filename noise, derive a title fragment, derive the author surname, run two `search_items` queries, union, fetch `get_item` + `list_attachments` per candidate, classify into three sub-cases, pick a default action. That is a lot of deterministic work asked of a model's memory, and on 2026-05-13 it failed: a duplicate of Rabkin's *What is Modern Israel?* slipped past because the existing record had the author's first name misspelt "Yakob". The skill was hardened with a mandatory two-pass search, but the weakness is structural — the procedure is skippable because it is prose.

### Correction to the original design note (verified 2026-07-25)

The archived note attributes the second failure — `search_items("Gaza An Inquest Into Its Martyrdom")` returning nothing against a record titled `Gaza: An inquest into its martyrdom` — to "Zotero's punctuation-sensitive tokeniser", and prescribes stripping punctuation from the query side.

**That diagnosis is wrong and the prescribed fix would not have worked.** `search_items` does not tokenise at all. `core/reader/search.rs:52` builds a single SQL `LIKE '%<whole query>%'` against `itemDataValues.value`. The match fails because the *stored* value contains a colon that the query string does not — and stripping punctuation from the query changes nothing, because the colon is on the database side. Any single-substring query is defeated by one punctuation difference anywhere in the title.

The fix that does work is **token matching**: normalise the input title, split it into significant words, search on the most selective of those words alone, then verify the remaining words against each candidate's own normalised title in Rust. Punctuation on either side becomes irrelevant. This spec adopts that instead.

The "Yakob" case needs no correction: a surname search (`cr.lastName LIKE '%Rabkin%'`) is unaffected by a misspelt *first* name, so the two-pass union catches it. The tool's job is to make that pass non-optional.

---

## Decisions

1. **One tool, `find_duplicates`, read-only, open-world false.** It reads the local SQLite library only — no external catalogue calls (those are the `lookup_*` tools). It never writes and never prompts; the model keeps the decision, the tool produces the triage.

2. **Typed output from birth.** `Json<FindDuplicatesResult>` with a derived `JsonSchema` and an object root — no `CallToolResult` text, no wrapper problem. This tool does not add to the Slice-G backlog.

3. **Inputs:** `title?`, `author_surname?`, `identifier?`, `input_kind` (`"pdf" | "url" | "name"`), `limit?` (per-pass SQL row cap, default 200). At least one of `title` / `author_surname` / `identifier` must be present, else `invalid_params`. `input_kind` drives the attachment comparison and defaults to `"name"`.

4. **Title normalisation, in order:** strip a known ebook extension (`.pdf`, `.epub`, `.mobi`, `.azw3`); strip trailing copy counters (`-1`, `_1`, `(1)`, ` copy`, ` copy 2`); strip parenthetical and bracketed groups (which is also what removes the `(z-lib.org)` / `nodrm` / `1lib.sk` shadow-library noise); replace `: ; — – , ? ! " ' ( ) [ ] / \ _ -` with spaces; collapse whitespace; lowercase. No truncation to N words — the token step supersedes it.

5. **Significant tokens** are normalised words of length ≥ 4 that are not in a small English stop-word list. Length ≥ 4 rather than the more usual 3 is deliberate: it drops "its", "was", "the" without needing an exhaustive list, and matches the original note's rule.

6. **Pass A (title) — selective-token search, up to six queries.** Take the significant tokens longest-first (a single long word is the most selective thing we can hand a `LIKE`) and run one `search_metadata` query per token, up to six. Union the hits.

   Then keep a hit only if it shares at least `MIN_SHARED_TOKENS` (2, or 1 when the input has only one significant token) with the input title, comparing normalised token sets. Every query and its row count is recorded in `queries_run`, so the tool's output *is* the echo the skill used to have to produce by hand.

   *Revised during implementation from three queries to six.* Three failed a real case: an input of "Modern Israel Studies Handbook Companion" spends its three longest words (companion, handbook, studies) on words the target record does not contain, and so misses "What is Modern Israel?" entirely — the words they share are the short ones. Each query is one `LIKE` scan against a local SQLite file, so the ceiling is about restraint rather than cost.

7. **Pass B (author surname) — with the title filter.** `search_metadata(surname)`, then discard any candidate that shares no significant token with the input title *when a title was supplied*. Without this filter a surname search floods the result set with the author's other books — the observed case being an input of "What is Modern Israel" surfacing "Gaza: An Inquest into its Martyrdom" (no shared content word).

8. **Pass C (identifier) — all plausible forms.** DOI: strip a leading `https://doi.org/`, `http://dx.doi.org/` or `doi:` and lowercase. ISBN: try the given form, the digits-only form, and the ISBN-10 ↔ ISBN-13 conversion (shared with the `lookup_isbn` work — see the sister spec — so one implementation serves both). arXiv: strip a leading `arXiv:`. One query per distinct form.

9. **Trash is excluded.** Nothing in the reader layer filters `deletedItems` today, so a trashed item can currently surface as a duplicate and trigger a spurious abort. `find_duplicates` drops trashed hits via a new small reader helper. The fixture gains the `deletedItems` table (it does not have one) plus a trashed row, so the exclusion is tested.

10. **Triage, per candidate:**
    - `"i"` — the candidate already has the same *kind* of attachment as the input. `input_kind = "pdf"` and any attachment is a PDF (`imported_file` or `linked_file` with `application/pdf`); or `input_kind = "url"` and any attachment is a snapshot (`imported_url`) or `linked_url`; or `input_kind = "name"` and the title similarity is strong (the item plainly exists and there is nothing to add). Default action `abort`.
    - `"ii"` — a same-kind attachment is absent, and the titles agree. Default action `attach_to_existing`.
    - `"iii"` — weak agreement: token-set Jaccard similarity below `0.5`, **or** both sides carry an identifier and they differ. Default action `ask`.

11. **`recommendation` resolves contradictions, never contradicts itself.** Any `"i"` → `abort`, and the other candidates are listed under `possible_stub_duplicates`. Else any `"ii"` → `attach_to_existing`, chosen against the richest candidate (most populated fields). Else `ask`. Empty candidate list → `create_new`.

12. **`metadata_diff` on `"ii"` candidates only, and it is explicitly a sparseness heuristic.** The tool does not know what the caller has in hand, so it reports what the *existing record* lacks: `missing` names absent fields from a fixed list (`ISBN`, `DOI`, `publisher`, `place`, `date`, `abstractNote`, `language`, `numPages`, `url`), and `thin` flags a year-only `date`. Documented as a prompt for the model to compare against its own extracted metadata, not as a diff.

13. **Union by `item_key`; every candidate appears exactly once**, with `found_by` recording which passes surfaced it (useful when triaging a report later, and it makes the Yakob case legible: `found_by: ["author"]`).

14. **Out of scope:** deciding attachment mode; writing to Zotero; calling external catalogues; prompting the user; splitting into `find_duplicates_by_title` / `..._by_identifier` (the original note's open question — one call with all three passes is cheap because the passes are SQL against a local file, and one call is the point).

---

## Output shape

```jsonc
{
  "queries_run": [
    { "pass": "title", "query": "martyrdom", "result_count": 1, "kept": 1 },
    { "pass": "author", "query": "Rabkin", "result_count": 2, "kept": 1 },
    { "pass": "identifier", "query": "9781844674879", "result_count": 0, "kept": 0 }
  ],
  "candidates": [
    {
      "item_key": "JGF2UTMW",
      "citation_key": "rabkinWhatModernIsrael2016",
      "title": "What is Modern Israel?",
      "year": "2016",
      "creators_short": "Rabkin",
      "item_type": "book",
      "found_by": ["author"],
      "title_similarity": 1.0,
      "attachments": [
        { "link_mode": "imported_file", "content_type": "application/pdf", "filename": "rabkin.pdf" }
      ],
      "triage": "ii",
      "triage_reason": "candidate has no PDF attachment; input is a pdf",
      "metadata_diff": { "missing": ["ISBN", "place", "language"], "thin": ["date"] },
      "default_action": "attach_to_existing"
    }
  ],
  "possible_stub_duplicates": [],
  "recommendation": "attach_to_existing",
  "next_step_if_empty": "no candidates — safe to create a new item"
}
```

`triage` ∈ {`i`, `ii`, `iii`}; `default_action` and `recommendation` ∈ {`abort`, `attach_to_existing`, `ask`, `create_new`}.

---

## New / changed files

| File | Change |
|---|---|
| `crates/zotero-mcp/src/core/dedup.rs` | **New.** Pure text functions (`normalise_title`, `significant_tokens`, `title_similarity`, `shares_tokens`) plus the `find_duplicates` orchestration over the reader pool |
| `crates/zotero-mcp/src/core/isbn.rs` | **New.** `isbn_variants`, ISBN-10 ↔ 13 conversion with check digits. Shared with the `lookup_isbn` resilience work |
| `crates/zotero-mcp/src/core/reader/trash.rs` | **New.** `trashed_keys(pool, library_id, keys)` → the subset that is in the trash |
| `crates/zotero-mcp/src/core/mod.rs`, `core/reader/mod.rs` | Register the new modules |
| `crates/zotero-mcp/src/tools/dedup.rs` | **New.** `FindDuplicatesArgs`, the result types, `find_duplicates_t` |
| `crates/zotero-mcp/src/tools/mod.rs` | Register |
| `crates/zotero-mcp/src/server.rs` | Register the tool with a description and `annotations(read_only_hint = true, open_world_hint = false)`. Tool count 34 → 35 |
| `crates/zotero-mcp/tests/fixtures/build_fixture.rs` | Add the `deletedItems` table; add a "Gaza: An inquest into its martyrdom" book by Rabkin (no attachment), a trashed near-duplicate, and an ISBN on the Rabkin book |
| `crates/zotero-mcp/tests/dedup.rs` | **New.** Unit tests for the pure functions; integration tests for the three named failure cases against the fixture |
| `README.md`, `CHANGELOG.md` | Document the tool |

**Not touched:** the transport files (`bearer.rs`, `oauth.rs`, `oauth/token_store.rs`, `http_transport.rs`, `setup.rs`).

---

## Tests (before implementation)

Named for the cases they encode:

1. `normalise_title_strips_extension_counters_and_shadow_library_noise`.
2. `normalise_title_flattens_punctuation` — `"Gaza: An inquest into its martyrdom"` and `"Gaza An Inquest Into Its Martyrdom"` normalise to the same string.
3. `gaza_case_token_search_finds_the_colon_titled_record` — integration, against the fixture: `find_duplicates(title: "Gaza An Inquest Into Its Martyrdom", input_kind: "pdf")` returns the record stored as `Gaza: An inquest into its martyrdom`. **This is the case a query-side-only punctuation fix would still fail.**
4. `yakob_case_author_pass_finds_record_with_misspelt_first_name` — integration: `find_duplicates(title: "What is Modern Israel", author_surname: "Rabkin")` returns `JGF2UTMW` (stored first name "Yakob"), with `found_by` including `"author"`.
5. `author_pass_discards_unrelated_book_by_same_author` — the Gaza record must not appear as a candidate for a "What is Modern Israel" input despite sharing the surname.
6. `trashed_items_are_never_candidates`.
7. `triage_i_when_candidate_already_has_pdf_and_input_is_pdf` / `triage_ii_when_candidate_has_no_pdf` / `triage_iii_on_weak_similarity`.
8. `recommendation_precedence_i_beats_ii_beats_iii`.
9. `empty_candidates_recommends_create_new`.
10. `at_least_one_input_required` — all-none input is `invalid_params`.
11. `identifier_pass_tries_both_isbn_forms` (ISBN unit tests live in the sister spec's test list).

---

## Risks

1. **Recall/precision balance on `MIN_SHARED_TOKENS = 2`.** Two shared significant words is a low bar; a busy library will surface some unrelated candidates. Deliberate: this is a *gate*, and a false candidate costs the model one glance at `title_similarity`, while a missed duplicate costs a polluted library. `title_similarity` is on every candidate so the model can see how weak a match is.
2. **A one-word title** (e.g. *Orientalism*) drops to `MIN_SHARED_TOKENS = 1`, which is exactly a substring search and will surface anything containing that word. Acceptable; `title_similarity` again carries the signal.
3. **Up to six token queries per call**, plus one author query and one per identifier form. Each is a `LIKE '%word%'` scan over `itemDataValues` — the same cost shape as today's `search_items`, repeated, against a local file. Not a concern at library scale; noted so nobody is surprised by a handful of queries in a log.
4. **Fixture edits could disturb existing reader tests.** All additions are new rows and one new table; existing tests assert on specific keys. `cargo test` is the check.
5. **`citation_key` requires Better BibTeX.** `SearchHit.citation_key` is `None` from SQL; hydration needs the BBT client, which may be absent. The field is `Option` and stays `None` when BBT is unavailable — not an error.

---

## Acceptance

- `find_duplicates` is registered, annotated read-only, and returns `Json<FindDuplicatesResult>`.
- The three named historical failures (Gaza punctuation, Yakob surname, same-author flooding) each have a passing test.
- Trashed items never appear as candidates.
- `recommendation` is never contradictory.
- `cargo test -p zotero-mcp` green; no new clippy warnings; transport files untouched.
- No commit, no reinstall, no launchd restart.
