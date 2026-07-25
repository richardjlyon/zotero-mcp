# Spec: `lookup_*` resilience — retry, alternate ISBN forms, structured failures

**Status:** Approved design, ready for plan-writing.
**Author:** Richard Lyon (with Claude Opus 5).
**Date:** 2026-07-25.
**Provenance:** the 2026-05-13 `adding-references` eval pass. Original design note: `~/Archive/pre-vault-cowork/project/obsidian/zotero-lookup-resilience.spec.md`. Open Question #6 on the `Zotero MCP` hub.
**Goal:** `lookup_doi`, `lookup_isbn` and `lookup_arxiv` stop giving up on the first upstream hiccup or the wrong identifier form, and when they do fail they say so in a shape the model can act on rather than a raw HTTP string.

---

## Problem

All three lookups make exactly one HTTP attempt and return either a record or the bare error. OpenLibrary, CrossRef and arXiv all have transient failures — 503s, connection resets, DNS blips, brief 404s on records that do exist. The 2026-05-13 eval pass hit one: `lookup_isbn` on a perfectly valid paperback ISBN returned

```
http error: error sending request for url (https://openlibrary.org/isbn/9781844674879.json)
```

The skill had no procedure for it, so the agent improvised — retried the alternate ISBN form (also failed), then hand-built the record. Both halves of that are wrong to leave in prose: the retry-and-alternate-form work is deterministic and should not be re-derived at variable thoroughness on every invocation, and mixing it into the same step muddies the one genuine judgement call (hand-build, or stop and ask Richard).

Three specific gaps in the current code (`core/enrichment/{openlibrary,crossref,arxiv}.rs`):

- **No retry.** A single `self.http.get(&url).send().await?` — one connection error ends the lookup.
- **No identifier normalisation.** A DOI pasted as `https://doi.org/10.1234/abcd` is sent to CrossRef verbatim as a path segment; an arXiv id pasted as `arXiv:2401.12345` likewise. Both fail as "not found" when the identifier is perfectly good.
- **No alternate ISBN form.** OpenLibrary indexes some editions under the ISBN-10 and some under the ISBN-13; the caller has whichever one is printed on the book.

---

## Decisions

1. **Retry once, on transient errors only.** Classifier: transient = connection error, timeout, DNS error, HTTP 5xx, HTTP 429. Permanent = every other 4xx. A 404 from CrossRef means CrossRef does not have that DOI; retrying is pointless and slow. Backoff 200 ms; for a 429 honour `Retry-After` when present and parseable, otherwise 1 s.

   *One* retry, not a ladder. These calls sit inside an interactive tool call and a human is waiting; the value is in surviving a blip, not in outlasting an outage.

2. **Normalise identifiers before the first attempt.**
   - DOI: trim; strip a leading `https://doi.org/`, `http://doi.org/`, `https://dx.doi.org/`, `http://dx.doi.org/`, or `doi:`; lowercase the registrant prefix (`10.xxxx`) and leave the suffix alone, because DOI suffixes are case-sensitive in principle even though most registrars fold them.
   - arXiv: trim; strip a leading `arXiv:` (any case); accept both new-style `2401.12345` (with optional `v2`) and old-style `hep-th/9901001`.
   - ISBN: trim; strip hyphens and spaces; uppercase a trailing `x` check digit.

3. **ISBN alternate form, tried automatically.** After the retried primary attempt fails — including on a 404, which for OpenLibrary genuinely can mean "indexed under the other form" — convert ISBN-10 ↔ ISBN-13 and try again (with its own single retry). Conversions are mechanical: 10→13 prepends `978` and recomputes the mod-10 check digit; 13→10 strips a `978` prefix and recomputes the mod-11 check digit (with `X` for 10). A 979-prefixed ISBN-13 has no ISBN-10 equivalent and is not converted.

4. **Structured failure, and it stays an error.** When every attempt fails the tool returns an MCP result with `is_error = true` whose content is a JSON object:

```jsonc
{
  "error": "lookup_failed",
  "source": "openlibrary",
  "identifier": "9781844674879",
  "attempts": [
    { "identifier": "9781844674879", "status": "connection_error", "detail": "error sending request", "transient": true },
    { "identifier": "9781844674879", "status": "connection_error", "detail": "error sending request", "transient": true },
    { "identifier": "1844674878",    "status": "http_404",         "detail": "HTTP 404 Not Found", "transient": false }
  ],
  "suggestion": "fall_back_to_hand_built"
}
```

   Two things are deliberate here. It keeps `is_error = true`, because the lookup *did* fail and a client that treats it as success would be misled. And the detail is JSON, not prose, so the model reads `suggestion` instead of parsing an error string. `suggestion` is `"fall_back_to_hand_built"` when every attempt was transient or a plain not-found, and `"stop_and_ask"` when something structurally odd happened (a 401/403, a malformed identifier that could not be normalised) — the distinction being "the catalogue doesn't have it / wasn't reachable" versus "something is wrong with the request or our access".

   The tool still does **not** decide to hand-build. That stays the caller's judgement, which is the whole point of making the signal machine-readable.

5. **Success shape unchanged.** A successful lookup returns exactly what it returns today (`format: "zotero"` flat item, or `"candidate"` envelope). This spec adds no fields to the success path, so it does not pre-empt the Slice-G wire-format decision — these three tools are on that migration list and the failure envelope is defined as a typed struct (`LookupFailure`, deriving `JsonSchema`) so that migration can adopt it as-is.

6. **Attempts are logged.** Each attempt logs at `debug` with source, identifier form, and outcome; a final failure logs at `warn` with the attempt count. Triaging "the lookup failed" reports later needs the trail.

7. **The cache is untouched.** A successful lookup still caches; failures never cache. Retry happens on the network side of the cache check, so a cached hit costs nothing.

8. **Out of scope:** falling back to other catalogues (Google Books, WorldCat) — that would be new `lookup_*` tools; retry on `search_crossref` / `search_semantic_scholar` (search is exploratory, a blip there is cheap to re-ask, and adding retries to a fan-out changes its cost profile); caching failures; a configurable retry count.

---

## New / changed files

| File | Change |
|---|---|
| `crates/zotero-mcp/src/core/enrichment/resilience.rs` | **New.** `Transience` classifier, `retry_once` helper, `LookupAttempt` / `LookupFailure` types, `AttemptStatus` |
| `crates/zotero-mcp/src/core/identifier.rs` | **New.** `normalise_doi`, `normalise_arxiv_id`, `normalise_isbn` |
| `crates/zotero-mcp/src/core/isbn.rs` | **New** (shared with `find_duplicates`). `isbn_variants`, `isbn10_to_13`, `isbn13_to_10`, check-digit maths |
| `crates/zotero-mcp/src/core/enrichment/openlibrary.rs` | `lookup_isbn`: normalise → retry → alternate form → structured failure |
| `crates/zotero-mcp/src/core/enrichment/crossref.rs` | `lookup_doi`: normalise → retry → structured failure |
| `crates/zotero-mcp/src/core/enrichment/arxiv.rs` | `lookup_arxiv`: normalise → retry → structured failure |
| `crates/zotero-mcp/src/core/error.rs` | New `Error::LookupFailed(Box<LookupFailure>)` variant carrying the structured detail |
| `crates/zotero-mcp/src/tools/enrichment.rs` | The three `lookup_*_t` fns map `Error::LookupFailed` to `CallToolResult::error(json)`; everything else keeps mapping through `map_err` |
| `crates/zotero-mcp/tests/lookup_resilience.rs` | **New.** `wiremock`-driven tests per source |
| `README.md`, `CHANGELOG.md` | Document the behaviour |

**Not touched:** the transport files (`bearer.rs`, `oauth.rs`, `oauth/token_store.rs`, `http_transport.rs`, `setup.rs`).

---

## Tests (before implementation)

Unit:

1. `isbn10_to_13_computes_check_digit` / `isbn13_to_10_computes_check_digit` / round-trip; `isbn13_with_979_prefix_has_no_isbn10`; `isbn_variants_dedupes`.
2. `normalise_doi_strips_url_and_prefix`; `normalise_doi_lowercases_registrant_only`; `normalise_arxiv_strips_prefix_and_keeps_old_style`; `normalise_isbn_strips_hyphens_and_uppercases_x`.
3. `transience_classifier` — 500/502/503/429 and connection errors transient; 400/401/403/404 permanent.
4. `suggestion_is_stop_and_ask_on_forbidden` vs `fall_back_to_hand_built_on_not_found`.

Integration (`wiremock`, matching the crate's existing enrichment test style):

5. `openlibrary_transient_failure_retries_once_then_succeeds` — first response 503, second 200; one record returned, two attempts recorded.
6. `openlibrary_404_tries_alternate_isbn_form` — the ISBN-13 path 404s, the derived ISBN-10 path 200s; the record comes back. **The OpenLibrary outage path from 2026-05-13 is this test plus the next one.**
7. `openlibrary_total_failure_returns_structured_error` — every path 503; result has `is_error = true`, three attempts, `suggestion = "fall_back_to_hand_built"`.
8. `crossref_404_does_not_retry` — exactly one request reaches the server (permanent error, no retry).
9. `crossref_normalises_doi_url_form` — a `https://doi.org/10.x/y` input hits the `/works/10.x/y` path.
10. `arxiv_retries_then_succeeds` and `arxiv_normalises_prefixed_id`.

---

## Risks

1. **Latency on a genuine outage.** Worst case for `lookup_isbn` is 4 requests (2 forms × 2 attempts) plus ~400 ms of backoff. Bounded and acceptable; the alternative is the agent hand-rolling the same sequence.
2. **`is_error = true` plus a JSON body may render awkwardly in some MCP client.** The MCP spec's own recommendation for tool-level failures is exactly this (error flag plus content the model can read), and Claude Code surfaces error content to the model. Noted rather than mitigated.
3. **ISBN check-digit maths is easy to get subtly wrong.** Hence unit tests with known-good pairs, including the 2026-05-13 ISBN (9781844674879 ↔ 1844674878) as a literal test case.
4. **Reduced test hermeticity if a retry test races.** All integration tests use `wiremock` with explicit response sequences and assert on the received-request count, so a stray retry fails the test loudly rather than passing by luck.
5. **`Error::LookupFailed` boxes a struct**, which is a slightly unusual error variant. Boxed to keep `Error`'s size down (clippy `result_large_err` otherwise).

---

## Acceptance

- A transient failure is retried once; a permanent one is not (asserted by request count, not by timing).
- A valid ISBN indexed under the other form is found without the caller doing anything.
- A total failure returns `is_error = true` with `attempts` and a `suggestion` the skill can branch on.
- DOIs and arXiv ids pasted in URL or prefixed form work.
- `cargo test -p zotero-mcp` green; no new clippy warnings; transport files untouched.
- No commit, no reinstall, no launchd restart.
