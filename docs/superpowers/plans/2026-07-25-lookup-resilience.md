# `lookup_*` resilience — Implementation Plan

**Goal:** retry once on transient upstream failures, try the alternate ISBN form, normalise pasted identifiers, and return a structured failure the model can branch on. Tests first.

**Spec:** `docs/superpowers/specs/2026-07-25-lookup-resilience-design.md`.

**Constraint:** transport files are out of bounds and not needed. `core/isbn.rs` is shared with the `find_duplicates` change — implement it once, in whichever change lands first.

---

## Task 1: Tests (red)

- [ ] **Step 1 — unit tests** in `core/isbn.rs`, `core/identifier.rs`, `core/enrichment/resilience.rs` (`#[cfg(test)] mod tests` in each): spec tests 1–4. Include the literal 2026-05-13 pair `9781844674879 ↔ 1844674878`.
- [ ] **Step 2 — `tests/lookup_resilience.rs`**, new file: spec tests 5–10, `wiremock` per the existing `tests/enrich_openlibrary.rs` style. Each test asserts on `server.received_requests().len()` so a missing or extra retry fails loudly.
- [ ] **Step 3 — confirm red.**

## Task 2: Implementation (green)

- [ ] **Step 4 — `core/isbn.rs`** (if not already landed by the `find_duplicates` change).

- [ ] **Step 5 — `core/identifier.rs`:** `normalise_doi`, `normalise_arxiv_id`, `normalise_isbn` per spec Decision 2. Pure.

- [ ] **Step 6 — `core/enrichment/resilience.rs`:**

```rust
pub enum AttemptStatus { Ok, HttpStatus(u16), ConnectionError, Timeout, DecodeError }
pub struct LookupAttempt { pub identifier: String, pub status: String, pub detail: String, pub transient: bool }
pub struct LookupFailure { pub error: String, pub source: String, pub identifier: String,
                           pub attempts: Vec<LookupAttempt>, pub suggestion: String }
pub fn is_transient(status: &AttemptStatus) -> bool
pub fn retry_after_delay(headers: &reqwest::header::HeaderMap) -> Duration
pub fn suggestion_for(attempts: &[LookupAttempt]) -> &'static str
```

  Plus the driver both callers share:

```rust
/// Run one HTTP GET with a single retry on transient failure, appending an
/// attempt record per try. Returns the successful response, or None with the
/// attempts recorded.
pub async fn get_with_retry(
    http: &reqwest::Client,
    url: &str,
    identifier: &str,
    attempts: &mut Vec<LookupAttempt>,
) -> Option<reqwest::Response>
```

  All types derive `Serialize + Deserialize + JsonSchema` so Slice G can adopt `LookupFailure` unchanged.

- [ ] **Step 7 — `core/error.rs`:** add

```rust
#[error("{source} lookup failed for {identifier} after {attempt_count} attempts")]
LookupFailed(Box<LookupFailure>),
```

  Boxed to keep `Error`'s size down. (Implement the `Display` by hand-formatting from the boxed struct's fields if thiserror's field interpolation gets awkward — the message is a fallback for humans; the JSON body is the real payload.)

- [ ] **Step 8 — `openlibrary.rs::lookup_isbn`:** normalise → cache check → for each form in `isbn_variants(normalised)`: `get_with_retry`; on success, parse and return; on 404 or exhausted retries, continue to the next form. When all forms fail, `Err(Error::LookupFailed(..))` with `suggestion_for(&attempts)`.

- [ ] **Step 9 — `crossref.rs::lookup_doi`:** normalise → cache check → `get_with_retry` (single form) → structured failure. No alternate form.

- [ ] **Step 10 — `arxiv.rs::lookup_arxiv`:** same as CrossRef with `normalise_arxiv_id`.

- [ ] **Step 11 — `tools/enrichment.rs`:** the three `lookup_*_t` fns intercept the structured failure before `map_err`:

```rust
fn lookup_result(r: Result<NormalizedRecord, CoreError>, format: &str)
    -> Result<CallToolResult, Error> {
    match r {
        Ok(rec) => Ok(CallToolResult::success(vec![Content::json(render_record(&rec, format)?)?])),
        Err(CoreError::LookupFailed(f)) => Ok(CallToolResult::error(vec![
            Content::json(serde_json::to_value(&*f).unwrap())?,
        ])),
        Err(e) => Err(map_err(e)),
    }
}
```

  One helper, three call sites. Keeps the existing success shape byte-identical.

- [ ] **Step 12 — register modules** in `core/mod.rs` and `core/enrichment/mod.rs`.

- [ ] **Step 13 — build, test, fmt, clippy** green.

## Task 3: Docs

- [ ] **Step 14 — README:** in the lookup tools' rows, note retry + alternate-ISBN + structured failure; add a troubleshooting entry for `lookup_failed` explaining `suggestion`.
- [ ] **Step 15 — CHANGELOG:** `[Unreleased] → Changed`, naming the OpenLibrary outage case.

## Hand-off

- [ ] Report to Richard. **No commit, no `cargo install`, no launchd restart.**
