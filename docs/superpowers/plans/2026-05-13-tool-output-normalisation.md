# Tool Output Normalisation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `lookup_doi`, `lookup_isbn`, and `lookup_arxiv` produce flat Zotero JSON by default that can be passed directly to `create_item`, while preserving the existing envelope shape under an opt-in `format="candidate"` parameter and fixing JSON-schema bugs that cause MCP clients to stringify structured arguments.

**Architecture:** Three slices. **Slice A** modifies `core/enrichment/openlibrary.rs` (date parsing + source URL fix), `core/enrichment/mod.rs` (extend `normalized_to_item`), `tools/enrichment.rs` (add `format` parameter + `render_record` helper), and `server.rs` (tool descriptions). **Slice B** changes three argument-field types from `Value` to `serde_json::Map<String, Value>` so `schemars` emits constrained schemas. **Slice C** adds `tests/schema_shape.rs` that asserts the schemars-generated schemas have the right types — the regression guard for Slice B.

**Tech Stack:** Rust, `serde_json`, `schemars`, `wiremock` (mock-server tests for upstream HTTP).

**Spec:** `docs/superpowers/specs/2026-05-13-tool-output-normalisation-design.md`

---

## File Structure

| File | Purpose | Action |
|---|---|---|
| `crates/zotero-mcp/src/core/enrichment/openlibrary.rs` | Add `parse_date` helper; fix `source_url`; use parsed date in `from_book_json` | Modify |
| `crates/zotero-mcp/src/core/enrichment/mod.rs` | Refactor `normalized_to_item`: drop `item_type` arg; rename creators to Zotero camelCase; stash provenance in `extra` | Modify |
| `crates/zotero-mcp/src/tools/enrichment.rs` | Add `format` to `DoiArgs`/`IsbnArgs`/`ArxivArgs`; add `render_record` helper; branch in `lookup_*_t`; change `ProposeArgs.candidates` and `EnrichArgs.candidates` types; adjust `parse_candidates` | Modify |
| `crates/zotero-mcp/src/tools/attachments.rs` | Change `CreateItemArgs.item` type; wrap with `Value::Object` in `create_item_t` | Modify |
| `crates/zotero-mcp/src/server.rs` | Update tool descriptions for `lookup_doi`, `lookup_isbn`, `lookup_arxiv`, `propose_metadata_update`, `enrich_item` | Modify |
| `crates/zotero-mcp/tests/enrich_openlibrary.rs` | Extend existing test with date assertion, `source_url` assertion, and `normalized_to_item` flat-shape assertions | Modify |
| `crates/zotero-mcp/tests/enrich_crossref.rs` | Extend existing test with `normalized_to_item` flat-shape assertions | Modify |
| `crates/zotero-mcp/tests/enrich_arxiv.rs` | Extend existing test with `normalized_to_item` flat-shape assertions | Modify |
| `crates/zotero-mcp/tests/schema_shape.rs` | New: assert schemars-generated schemas declare correct types | Create |

---

## Task 1: OpenLibrary `parse_date` helper

**Files:**
- Modify: `crates/zotero-mcp/src/core/enrichment/openlibrary.rs`

- [ ] **Step 1: Write failing tests**

Append to `crates/zotero-mcp/src/core/enrichment/openlibrary.rs` (after the existing `split_name` function):

```rust
#[cfg(test)]
mod tests {
    use super::parse_date;

    #[test]
    fn parse_date_iso_year_passes_through() {
        assert_eq!(parse_date("2020"), "2020");
    }

    #[test]
    fn parse_date_iso_year_month_passes_through() {
        assert_eq!(parse_date("2020-05"), "2020-05");
    }

    #[test]
    fn parse_date_iso_full_passes_through() {
        assert_eq!(parse_date("1998-09-08"), "1998-09-08");
    }

    #[test]
    fn parse_date_long_month_day_year() {
        assert_eq!(parse_date("March 5, 2020"), "2020-03-05");
    }

    #[test]
    fn parse_date_short_month_day_year() {
        assert_eq!(parse_date("Mar 5, 2020"), "2020-03-05");
    }

    #[test]
    fn parse_date_day_long_month_year() {
        assert_eq!(parse_date("5 March 2020"), "2020-03-05");
    }

    #[test]
    fn parse_date_long_month_year() {
        assert_eq!(parse_date("March 2020"), "2020-03");
    }

    #[test]
    fn parse_date_short_month_year() {
        assert_eq!(parse_date("Mar 2020"), "2020-03");
    }

    #[test]
    fn parse_date_unparseable_passes_through() {
        assert_eq!(parse_date("sometime in 2020"), "sometime in 2020");
    }

    #[test]
    fn parse_date_trims_whitespace() {
        assert_eq!(parse_date("  2020  "), "2020");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p zotero-mcp --lib core::enrichment::openlibrary 2>&1 | tail -20`

Expected: FAIL — `parse_date` is unresolved.

- [ ] **Step 3: Implement `parse_date`**

Add to `crates/zotero-mcp/src/core/enrichment/openlibrary.rs` (above `split_name`):

```rust
const MONTHS: &[(&str, u32)] = &[
    ("january", 1), ("jan", 1),
    ("february", 2), ("feb", 2),
    ("march", 3), ("mar", 3),
    ("april", 4), ("apr", 4),
    ("may", 5),
    ("june", 6), ("jun", 6),
    ("july", 7), ("jul", 7),
    ("august", 8), ("aug", 8),
    ("september", 9), ("sept", 9), ("sep", 9),
    ("october", 10), ("oct", 10),
    ("november", 11), ("nov", 11),
    ("december", 12), ("dec", 12),
];

/// Parse OpenLibrary's freeform `publish_date` into ISO 8601 (YYYY-MM-DD,
/// YYYY-MM, or YYYY). Returns the trimmed input unchanged if the string
/// doesn't cleanly match a known pattern — never drops information.
pub(crate) fn parse_date(s: &str) -> String {
    let trimmed = s.trim();
    if is_iso_date(trimmed) {
        return trimmed.to_string();
    }

    let cleaned = trimmed.to_lowercase().replace(',', " ");
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();

    let mut month: Option<u32> = None;
    let mut day: Option<u32> = None;
    let mut year: Option<u32> = None;
    let mut unrecognised = 0;

    for tok in &tokens {
        if let Some((_, m)) = MONTHS.iter().find(|(name, _)| *name == *tok) {
            if month.is_none() {
                month = Some(*m);
            }
        } else if let Ok(n) = tok.parse::<u32>() {
            if (1000..=9999).contains(&n) {
                year = Some(n);
            } else if (1..=31).contains(&n) {
                day = Some(n);
            } else {
                unrecognised += 1;
            }
        } else {
            unrecognised += 1;
        }
    }

    if unrecognised > 0 {
        return trimmed.to_string();
    }

    match (year, month, day) {
        (Some(y), Some(m), Some(d)) => format!("{:04}-{:02}-{:02}", y, m, d),
        (Some(y), Some(m), None) => format!("{:04}-{:02}", y, m),
        (Some(y), None, None) => format!("{:04}", y),
        _ => trimmed.to_string(),
    }
}

fn is_iso_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.iter().any(|p| !p.chars().all(|c| c.is_ascii_digit())) {
        return false;
    }
    match parts.as_slice() {
        [y] if y.len() == 4 => true,
        [y, m] if y.len() == 4 && m.len() == 2 => true,
        [y, m, d] if y.len() == 4 && m.len() == 2 && d.len() == 2 => true,
        _ => false,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p zotero-mcp --lib core::enrichment::openlibrary 2>&1 | tail -20`

Expected: PASS — 10 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/core/enrichment/openlibrary.rs
git commit -m "$(cat <<'EOF'
feat(enrichment): parse_date helper normalises OpenLibrary dates to ISO 8601

Handles YYYY / YYYY-MM / YYYY-MM-DD pass-through plus common freeform
shapes ("March 5, 2020", "Mar 2020", "5 March 2020"). Returns the input
unchanged when it can't confidently parse, so non-ISO dates survive
rather than being silently dropped.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Fix OpenLibrary `source_url` and apply `parse_date` in `from_book_json`

**Files:**
- Modify: `crates/zotero-mcp/src/core/enrichment/openlibrary.rs`
- Modify: `crates/zotero-mcp/tests/enrich_openlibrary.rs`

- [ ] **Step 1: Update the existing integration test to assert on the fixes**

Replace `crates/zotero-mcp/tests/enrich_openlibrary.rs` with:

```rust
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use tempfile::tempdir;
use zotero_mcp::core::cache::DiskCache;
use zotero_mcp::core::enrichment::openlibrary::OpenLibraryClient;

#[tokio::test]
async fn lookup_isbn_normalizes() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/isbn/9780000000000.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "title": "Some Book",
            "publish_date": "March 5, 2020",
            "publishers": ["BookCo"],
            "authors": [{"key":"/authors/OL1A"}]
        }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/authors/OL1A.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "Jane Doe"
        }))).mount(&server).await;

    let dir = tempdir().unwrap();
    let c = OpenLibraryClient::new(server.uri(), DiskCache::new(dir.path().to_path_buf(), 60), "test/0.1");
    let r = c.lookup_isbn("9780000000000").await.unwrap();

    // Envelope assertions (unchanged).
    assert_eq!(r.fields["title"], "Some Book");
    assert_eq!(r.fields["itemType"], "book");
    assert_eq!(r.creators[0].last_name.as_deref(), Some("Doe"));

    // New: date now normalised to ISO 8601.
    assert_eq!(r.fields["date"], "2020-03-05");
    // New: source_url points at the actual record, not just the base URL.
    let expected_url = format!("{}/isbn/9780000000000", server.uri());
    assert_eq!(r.source_url.as_deref(), Some(expected_url.as_str()));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zotero-mcp --test enrich_openlibrary 2>&1 | tail -20`

Expected: FAIL — `r.fields["date"]` will be `"March 5, 2020"` (raw), and `r.source_url` will end in `/`, not `/isbn/9780000000000`.

- [ ] **Step 3: Apply both fixes in `openlibrary.rs`**

In `crates/zotero-mcp/src/core/enrichment/openlibrary.rs`:

(a) `from_book_json` doesn't currently know the ISBN — pass it down. Edit `lookup_isbn` and `from_book_json`'s signature accordingly:

```rust
pub async fn lookup_isbn(&self, isbn: &str) -> Result<NormalizedRecord> {
    let key = format!("openlibrary:isbn:{}", isbn);
    if let Some(v) = self.cache.get::<Value>(&key).await? {
        return self.from_book_json(v, isbn).await;
    }
    let url = format!("{}/isbn/{}.json", self.base, isbn);
    let resp = self.http.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(Error::Lookup {
            r#source: "openlibrary".into(),
            message: format!("HTTP {}", resp.status()),
        });
    }
    let book: Value = resp.json().await?;
    self.cache.put(&key, &book).await.ok();
    self.from_book_json(book, isbn).await
}

async fn from_book_json(&self, book: Value, isbn: &str) -> Result<NormalizedRecord> {
```

(b) Inside `from_book_json`, run `publish_date` through `parse_date`:

```rust
        if let Some(d) = book.get("publish_date").and_then(|x| x.as_str()) {
            fields.insert("date".into(), Value::String(parse_date(d)));
        }
```

(c) At the end of `from_book_json`, replace the `source_url` construction:

```rust
        Ok(NormalizedRecord {
            source: "openlibrary".into(),
            fields,
            creators,
            source_url: Some(format!("{}/isbn/{}", self.base, isbn)),
        })
```

- [ ] **Step 4: Run all openlibrary tests to verify**

Run: `cargo test -p zotero-mcp --test enrich_openlibrary 2>&1 | tail -10`

Expected: PASS.

Run: `cargo test -p zotero-mcp --lib core::enrichment::openlibrary 2>&1 | tail -10`

Expected: PASS — all `parse_date` tests still green.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/core/enrichment/openlibrary.rs crates/zotero-mcp/tests/enrich_openlibrary.rs
git commit -m "$(cat <<'EOF'
fix(enrichment): OpenLibrary emits ISO 8601 dates and a real source URL

publish_date now runs through parse_date so freeform values like
"March 5, 2020" become "2020-03-05". source_url now points at the
specific /isbn/{isbn} record instead of the base URL with a trailing
slash, so it's actually useful as provenance.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Audit CrossRef and arXiv dates against ISO 8601

**Files:**
- Read only: `crates/zotero-mcp/src/core/enrichment/crossref.rs`, `arxiv.rs`
- Modify: `crates/zotero-mcp/src/core/enrichment/mod.rs` (comment only)

- [ ] **Step 1: Inspect CrossRef date output**

Read `crates/zotero-mcp/src/core/enrichment/crossref.rs:137-155` (`extract_date`). Expected: produces `"YYYY"` from `[YYYY]`, `"YYYY-MM"` from `[YYYY, M]`, `"YYYY-MM-DD"` from `[YYYY, M, D]` — all valid ISO 8601 because `format!("{:02}", n)` pads single-digit months/days.

The existing test in `enrich_crossref.rs` asserts `r.fields["date"] == "2024-03"` for `[2024, 3]` — already correct.

- [ ] **Step 2: Inspect arXiv date output**

Read `crates/zotero-mcp/src/core/enrichment/arxiv.rs:67-69`. Expected: takes `published` (e.g. `"2024-01-01T00:00:00Z"`) and splits at `T` → `"2024-01-01"`. Valid ISO 8601.

- [ ] **Step 3: If audits pass, record the result**

If both audits confirm ISO 8601 output, add a top-of-file comment to `crates/zotero-mcp/src/core/enrichment/mod.rs` (above the `pub mod crossref;` line):

```rust
// Date format audit (2026-05-13): all three sources emit ISO 8601 dates.
// - openlibrary: normalised via parse_date (handles freeform publish_date).
// - crossref: extract_date pads {YYYY, MM, DD} parts to 2-digit width.
// - arxiv: published timestamps split at 'T' (arXiv always sends ISO 8601).
```

If an audit fails (e.g. unpadded digits in CrossRef), fix inline using `format!("{:02}", …)` and add a regression test before continuing.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-mcp/src/core/enrichment/mod.rs
git commit -m "$(cat <<'EOF'
docs(enrichment): record date-format audit; all sources emit ISO 8601

OpenLibrary's parse_date fix (previous commit) was the only gap.
CrossRef already pads month/day digits; arXiv passes through native ISO
8601 timestamps. No further code changes needed.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Refactor `normalized_to_item` — drop `item_type` arg, camelCase creators, stash provenance in `extra`

**Files:**
- Modify: `crates/zotero-mcp/src/core/enrichment/mod.rs`

- [ ] **Step 1: Write failing tests**

Append to `crates/zotero-mcp/src/core/enrichment/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::Creator;
    use serde_json::{Map, Value};

    fn record_with(
        source: &str,
        item_type: &str,
        title: &str,
        source_url: Option<&str>,
    ) -> NormalizedRecord {
        let mut fields = Map::new();
        fields.insert("itemType".into(), Value::String(item_type.into()));
        fields.insert("title".into(), Value::String(title.into()));
        NormalizedRecord {
            source: source.into(),
            fields,
            creators: vec![Creator {
                first_name: Some("Jane".into()),
                last_name: Some("Doe".into()),
                creator_type: "author".into(),
                order_index: 0,
            }],
            source_url: source_url.map(String::from),
        }
    }

    #[test]
    fn flat_output_is_object_with_item_type_from_fields() {
        let r = record_with("openlibrary", "book", "Some Book", None);
        let v = normalized_to_item(&r);
        let obj = v.as_object().expect("top-level object");
        assert_eq!(obj["itemType"], "book");
        assert_eq!(obj["title"], "Some Book");
        assert!(!obj.contains_key("source"));
        assert!(!obj.contains_key("source_url"));
        assert!(!obj.contains_key("fields"));
    }

    #[test]
    fn creators_use_zotero_camel_case() {
        let r = record_with("openlibrary", "book", "x", None);
        let v = normalized_to_item(&r);
        let creators = v["creators"].as_array().expect("creators array");
        let c0 = creators[0].as_object().expect("creator object");
        assert_eq!(c0["creatorType"], "author");
        assert_eq!(c0["firstName"], "Jane");
        assert_eq!(c0["lastName"], "Doe");
        assert!(!c0.contains_key("creator_type"));
        assert!(!c0.contains_key("first_name"));
        assert!(!c0.contains_key("last_name"));
        assert!(!c0.contains_key("orderIndex"));
        assert!(!c0.contains_key("order_index"));
    }

    #[test]
    fn extra_field_stashes_source_and_source_url() {
        let r = record_with("openlibrary", "book", "x", Some("https://openlibrary.org/isbn/9780000000000"));
        let v = normalized_to_item(&r);
        let extra = v["extra"].as_str().expect("extra string");
        assert!(extra.contains("source: openlibrary"), "got: {extra:?}");
        assert!(extra.contains("sourceURL: https://openlibrary.org/isbn/9780000000000"), "got: {extra:?}");
    }

    #[test]
    fn extra_omits_source_url_line_when_none() {
        let r = record_with("arxiv", "preprint", "x", None);
        let v = normalized_to_item(&r);
        let extra = v["extra"].as_str().expect("extra string");
        assert!(extra.contains("source: arxiv"));
        assert!(!extra.contains("sourceURL"), "got: {extra:?}");
    }

    #[test]
    fn extra_appends_to_existing_extra_field() {
        let mut r = record_with("crossref", "journalArticle", "x", Some("https://doi.org/10.1/x"));
        r.fields.insert("extra".into(), Value::String("Citation Key: foo2024".into()));
        let v = normalized_to_item(&r);
        let extra = v["extra"].as_str().expect("extra string");
        assert!(extra.starts_with("Citation Key: foo2024"));
        assert!(extra.contains("source: crossref"));
    }

    #[test]
    fn creator_with_only_last_name_omits_first_name_key() {
        let mut r = record_with("openlibrary", "book", "x", None);
        r.creators[0].first_name = None;
        let v = normalized_to_item(&r);
        let c0 = &v["creators"][0];
        assert!(c0.get("firstName").is_none());
        assert_eq!(c0["lastName"], "Doe");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p zotero-mcp --lib core::enrichment::tests 2>&1 | tail -20`

Expected: FAIL — `normalized_to_item` currently takes a separate `item_type` arg, emits snake-case creator keys, and doesn't write `extra`.

- [ ] **Step 3: Rewrite `normalized_to_item`**

Replace the existing `normalized_to_item` in `crates/zotero-mcp/src/core/enrichment/mod.rs` with:

```rust
/// Flatten a `NormalizedRecord` into a Zotero-shaped item JSON suitable for
/// `core::writer::items::create_item`.
///
/// Reads `itemType` from `record.fields` (every source populates it).
/// Rewrites creators inline using Zotero's wire vocabulary
/// (`creatorType` / `firstName` / `lastName`) — the internal `Creator` struct
/// keeps snake_case names because it is the canonical type used by readers,
/// scoring, and diffing. The wire-shape rename lives only here.
///
/// Stashes provenance (`source`, `source_url`) into Zotero's `extra` field
/// as newline-separated `key: value` lines, appending to any pre-existing
/// `extra` content.
pub fn normalized_to_item(record: &NormalizedRecord) -> Value {
    let mut obj = record.fields.clone();

    if !record.creators.is_empty() {
        let creators: Vec<Value> = record
            .creators
            .iter()
            .map(|c| {
                let mut m = serde_json::Map::new();
                m.insert("creatorType".into(), Value::String(c.creator_type.clone()));
                if let Some(ref first) = c.first_name {
                    m.insert("firstName".into(), Value::String(first.clone()));
                }
                if let Some(ref last) = c.last_name {
                    m.insert("lastName".into(), Value::String(last.clone()));
                }
                Value::Object(m)
            })
            .collect();
        obj.insert("creators".into(), Value::Array(creators));
    }

    let mut extra_lines: Vec<String> = Vec::new();
    if let Some(existing) = obj.get("extra").and_then(|v| v.as_str()) {
        if !existing.is_empty() {
            extra_lines.push(existing.to_string());
        }
    }
    extra_lines.push(format!("source: {}", record.source));
    if let Some(ref url) = record.source_url {
        extra_lines.push(format!("sourceURL: {}", url));
    }
    obj.insert("extra".into(), Value::String(extra_lines.join("\n")));

    Value::Object(obj)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p zotero-mcp --lib core::enrichment::tests 2>&1 | tail -20`

Expected: PASS — 6 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/core/enrichment/mod.rs
git commit -m "$(cat <<'EOF'
refactor(enrichment): normalized_to_item flattens to Zotero wire shape

- Reads itemType from record.fields (every source populates it) — drops
  the redundant item_type argument.
- Rewrites creators with Zotero's wire vocabulary
  ({creatorType, firstName, lastName}); the internal Creator struct keeps
  snake_case because it's the canonical type used by readers/scoring/diff.
- Appends provenance (source, sourceURL) to the Zotero `extra` field as
  newline-separated key:value lines, preserving any existing extra content.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Extend existing core-client integration tests with `normalized_to_item` assertions

**Files:**
- Modify: `crates/zotero-mcp/tests/enrich_openlibrary.rs`
- Modify: `crates/zotero-mcp/tests/enrich_crossref.rs`
- Modify: `crates/zotero-mcp/tests/enrich_arxiv.rs`

Purpose: prove that each lookup's `NormalizedRecord`, when passed through `normalized_to_item`, produces a valid flat Zotero item with provenance in `extra`. End-to-end coverage from upstream HTTP fixture through to the wire-ready shape, without needing an `AppState`.

- [ ] **Step 1: Extend `enrich_openlibrary.rs`**

Replace `crates/zotero-mcp/tests/enrich_openlibrary.rs` with (this supersedes Task 2's version, adding the flat-shape assertions):

```rust
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use tempfile::tempdir;
use zotero_mcp::core::cache::DiskCache;
use zotero_mcp::core::enrichment::normalized_to_item;
use zotero_mcp::core::enrichment::openlibrary::OpenLibraryClient;

#[tokio::test]
async fn lookup_isbn_normalizes() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/isbn/9780000000000.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "title": "Some Book",
            "publish_date": "March 5, 2020",
            "publishers": ["BookCo"],
            "authors": [{"key":"/authors/OL1A"}]
        }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/authors/OL1A.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "Jane Doe"
        }))).mount(&server).await;

    let dir = tempdir().unwrap();
    let c = OpenLibraryClient::new(server.uri(), DiskCache::new(dir.path().to_path_buf(), 60), "test/0.1");
    let r = c.lookup_isbn("9780000000000").await.unwrap();

    // Envelope assertions.
    assert_eq!(r.fields["title"], "Some Book");
    assert_eq!(r.fields["itemType"], "book");
    assert_eq!(r.fields["date"], "2020-03-05");
    assert_eq!(r.creators[0].last_name.as_deref(), Some("Doe"));
    let expected_url = format!("{}/isbn/9780000000000", server.uri());
    assert_eq!(r.source_url.as_deref(), Some(expected_url.as_str()));

    // Flat-shape assertions via normalized_to_item.
    let v = normalized_to_item(&r);
    let obj = v.as_object().expect("top-level object");
    assert_eq!(obj["itemType"], "book");
    assert_eq!(obj["title"], "Some Book");
    assert_eq!(obj["date"], "2020-03-05");
    assert!(!obj.contains_key("source"));
    assert!(!obj.contains_key("source_url"));
    assert!(!obj.contains_key("fields"));

    let c0 = &v["creators"][0];
    assert_eq!(c0["creatorType"], "author");
    assert_eq!(c0["firstName"], "Jane");
    assert_eq!(c0["lastName"], "Doe");
    assert!(c0.get("creator_type").is_none());

    let extra = v["extra"].as_str().expect("extra string");
    assert!(extra.contains("source: openlibrary"), "got: {extra:?}");
    let expected_url_line = format!("sourceURL: {}/isbn/9780000000000", server.uri());
    assert!(extra.contains(&expected_url_line), "got: {extra:?}");
}
```

- [ ] **Step 2: Extend `enrich_crossref.rs`**

Replace `crates/zotero-mcp/tests/enrich_crossref.rs` with:

```rust
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use tempfile::tempdir;
use zotero_mcp::core::cache::DiskCache;
use zotero_mcp::core::enrichment::crossref::CrossrefClient;
use zotero_mcp::core::enrichment::normalized_to_item;

#[tokio::test]
async fn lookup_doi_normalizes_to_zotero_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/works/10.1234/abcd"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "ok",
            "message": {
                "DOI": "10.1234/abcd",
                "title": ["A Paper on Things"],
                "author": [{"given":"Alice","family":"Aardvark"}],
                "issued": {"date-parts": [[2024, 3]]},
                "container-title": ["Journal of Things"],
                "publisher": "ThingPress",
                "type": "journal-article",
                "URL": "https://doi.org/10.1234/abcd",
                "abstract": "Abstract content."
            }
        })))
        .mount(&server).await;

    let dir = tempdir().unwrap();
    let cache = DiskCache::new(dir.path().to_path_buf(), 60);
    let c = CrossrefClient::new(server.uri(), cache, "zotero-mcp/0.1");
    let r = c.lookup_doi("10.1234/abcd").await.unwrap();

    // Envelope assertions.
    assert_eq!(r.fields["title"], "A Paper on Things");
    assert_eq!(r.fields["DOI"], "10.1234/abcd");
    assert_eq!(r.fields["date"], "2024-03");
    assert_eq!(r.fields["itemType"], "journalArticle");
    assert_eq!(r.creators[0].last_name.as_deref(), Some("Aardvark"));

    // Flat-shape assertions.
    let v = normalized_to_item(&r);
    assert_eq!(v["itemType"], "journalArticle");
    assert_eq!(v["DOI"], "10.1234/abcd");
    assert_eq!(v["date"], "2024-03");
    assert_eq!(v["creators"][0]["creatorType"], "author");
    assert_eq!(v["creators"][0]["firstName"], "Alice");
    assert_eq!(v["creators"][0]["lastName"], "Aardvark");

    let extra = v["extra"].as_str().unwrap();
    assert!(extra.contains("source: crossref"));
    assert!(extra.contains("sourceURL: https://doi.org/10.1234/abcd"));
}
```

- [ ] **Step 3: Extend `enrich_arxiv.rs`**

Replace `crates/zotero-mcp/tests/enrich_arxiv.rs` with:

```rust
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use tempfile::tempdir;
use zotero_mcp::core::cache::DiskCache;
use zotero_mcp::core::enrichment::arxiv::ArxivClient;
use zotero_mcp::core::enrichment::normalized_to_item;

const SAMPLE_ATOM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<entry>
  <id>http://arxiv.org/abs/2401.00001v1</id>
  <title>A Cool Preprint</title>
  <summary>Abstract here.</summary>
  <published>2024-01-01T00:00:00Z</published>
  <author><name>Alice Aardvark</name></author>
  <author><name>Bob Baboon</name></author>
</entry>
</feed>"#;

#[tokio::test]
async fn lookup_arxiv_parses_atom() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/query"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE_ATOM))
        .mount(&server).await;
    let dir = tempdir().unwrap();
    let c = ArxivClient::new(server.uri(), DiskCache::new(dir.path().to_path_buf(), 60), "test/0.1");
    let r = c.lookup_arxiv("2401.00001").await.unwrap();

    // Envelope assertions.
    assert_eq!(r.fields["title"], "A Cool Preprint");
    assert_eq!(r.fields["itemType"], "preprint");
    assert_eq!(r.creators.len(), 2);

    // Flat-shape assertions.
    let v = normalized_to_item(&r);
    assert_eq!(v["itemType"], "preprint");
    assert_eq!(v["title"], "A Cool Preprint");
    assert_eq!(v["date"], "2024-01-01");
    assert_eq!(v["creators"].as_array().unwrap().len(), 2);

    let extra = v["extra"].as_str().unwrap();
    assert!(extra.contains("source: arxiv"));
    // arXiv parser does not populate source_url today.
    assert!(!extra.contains("sourceURL"), "got: {extra:?}");
}
```

- [ ] **Step 4: Run all three test files**

Run: `cargo test -p zotero-mcp --test enrich_openlibrary --test enrich_crossref --test enrich_arxiv 2>&1 | tail -15`

Expected: PASS — 3 tests across 3 files.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/tests/enrich_openlibrary.rs crates/zotero-mcp/tests/enrich_crossref.rs crates/zotero-mcp/tests/enrich_arxiv.rs
git commit -m "$(cat <<'EOF'
test(enrichment): assert lookup output flattens to valid Zotero items

Each enrich_*.rs integration test now passes its lookup result through
normalized_to_item and asserts on the flat shape: top-level itemType /
title / date / creators, creators with camelCase keys, and an `extra`
field carrying source + sourceURL provenance. Closes the end-to-end
contract that lookup output is directly usable by create_item.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Add `format` parameter; implement `render_record`; branch `lookup_*_t`

**Files:**
- Modify: `crates/zotero-mcp/src/tools/enrichment.rs`

- [ ] **Step 1: Write failing unit tests for `render_record`**

Append to `crates/zotero-mcp/src/tools/enrichment.rs` (inside the file, in a new `#[cfg(test)] mod tests` block at the end):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::Creator;
    use serde_json::{Map, Value};

    fn sample_record() -> NormalizedRecord {
        let mut fields = Map::new();
        fields.insert("itemType".into(), Value::String("book".into()));
        fields.insert("title".into(), Value::String("X".into()));
        NormalizedRecord {
            source: "openlibrary".into(),
            fields,
            creators: vec![Creator {
                first_name: Some("Jane".into()),
                last_name: Some("Doe".into()),
                creator_type: "author".into(),
                order_index: 0,
            }],
            source_url: Some("https://example.test/x".into()),
        }
    }

    #[test]
    fn render_record_zotero_returns_flat_shape() {
        let r = sample_record();
        let v = render_record(&r, "zotero").unwrap();
        let obj = v.as_object().unwrap();
        assert_eq!(obj["itemType"], "book");
        assert_eq!(obj["title"], "X");
        assert!(!obj.contains_key("source"));
        assert!(!obj.contains_key("fields"));
        let extra = obj["extra"].as_str().unwrap();
        assert!(extra.contains("source: openlibrary"));
    }

    #[test]
    fn render_record_candidate_returns_envelope() {
        let r = sample_record();
        let v = render_record(&r, "candidate").unwrap();
        assert_eq!(v["source"], "openlibrary");
        assert_eq!(v["fields"]["itemType"], "book");
        assert_eq!(v["fields"]["title"], "X");
        assert!(v["creators"].is_array());
    }

    #[test]
    fn render_record_unknown_format_errors() {
        let r = sample_record();
        let err = render_record(&r, "garbage").unwrap_err();
        assert!(err.to_string().contains("format must be"), "got: {err}");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p zotero-mcp --lib tools::enrichment 2>&1 | tail -20`

Expected: FAIL — `render_record` is unresolved.

- [ ] **Step 3: Add `format` field to the three args structs and a default helper**

In `crates/zotero-mcp/src/tools/enrichment.rs`, add the helper above the first struct and the field on each of the three lookup args:

```rust
fn default_format() -> String {
    "zotero".into()
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DoiArgs {
    pub doi: String,
    #[serde(default = "default_format")]
    pub format: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IsbnArgs {
    pub isbn: String,
    #[serde(default = "default_format")]
    pub format: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ArxivArgs {
    pub id: String,
    #[serde(default = "default_format")]
    pub format: String,
}
```

- [ ] **Step 4: Implement `render_record` and update `lookup_*_t`**

In `crates/zotero-mcp/src/tools/enrichment.rs`, add `render_record` (e.g. above `lookup_doi_t`) and update the three lookup tool functions:

```rust
fn render_record(record: &NormalizedRecord, format: &str) -> Result<Value, Error> {
    match format {
        "zotero" => Ok(crate::core::enrichment::normalized_to_item(record)),
        "candidate" => Ok(serde_json::to_value(record).unwrap()),
        other => Err(invalid(format!(
            "format must be 'zotero' or 'candidate' (got '{}')",
            other
        ))),
    }
}

pub async fn lookup_doi_t(s: &AppState, a: DoiArgs) -> Result<CallToolResult, Error> {
    let r = s.crossref.lookup_doi(&a.doi).await.map_err(map_err)?;
    let body = render_record(&r, &a.format)?;
    Ok(CallToolResult::success(vec![Content::json(body)?]))
}

pub async fn lookup_isbn_t(s: &AppState, a: IsbnArgs) -> Result<CallToolResult, Error> {
    let r = s.openlibrary.lookup_isbn(&a.isbn).await.map_err(map_err)?;
    let body = render_record(&r, &a.format)?;
    Ok(CallToolResult::success(vec![Content::json(body)?]))
}

pub async fn lookup_arxiv_t(s: &AppState, a: ArxivArgs) -> Result<CallToolResult, Error> {
    let r = s.arxiv.lookup_arxiv(&a.id).await.map_err(map_err)?;
    let body = render_record(&r, &a.format)?;
    Ok(CallToolResult::success(vec![Content::json(body)?]))
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p zotero-mcp --lib tools::enrichment 2>&1 | tail -10`

Expected: PASS — 3 `render_record` unit tests.

Also re-run the full suite to confirm nothing else broke:

Run: `cargo test -p zotero-mcp 2>&1 | tail -10`

Expected: PASS — everything.

- [ ] **Step 6: Commit**

```bash
git add crates/zotero-mcp/src/tools/enrichment.rs
git commit -m "$(cat <<'EOF'
feat(tools): lookup_*_t honours format; default emits flat Zotero JSON

format='zotero' (default) returns the output of normalized_to_item — a
flat object directly compatible with create_item. format='candidate'
keeps emitting the NormalizedRecord envelope for use with
propose_metadata_update and enrich_item. An unknown format value errors
out before any external HTTP call.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Update tool descriptions in `server.rs`

**Files:**
- Modify: `crates/zotero-mcp/src/server.rs`

- [ ] **Step 1: Update the five tool descriptions**

In `crates/zotero-mcp/src/server.rs`, replace the `#[tool(description = ...)]` attributes for `lookup_doi`, `lookup_isbn`, `lookup_arxiv`, `propose_metadata_update`, and `enrich_item`:

```rust
    #[tool(description = "Look up a DOI via CrossRef. \
                          `format='zotero'` (default) returns a flat Zotero item ready to pass straight to `create_item`; \
                          `format='candidate'` returns an envelope `{source, fields, creators, source_url}` for use with `propose_metadata_update` and `enrich_item`.")]
    pub async fn lookup_doi(&self, #[tool(aggr)] args: DoiArgs) -> Result<CallToolResult, McpError> {
        en::lookup_doi_t(&self.state, args).await
    }

    #[tool(description = "Look up an ISBN via OpenLibrary. \
                          `format='zotero'` (default) returns a flat Zotero item ready to pass straight to `create_item`; \
                          `format='candidate'` returns an envelope `{source, fields, creators, source_url}` for use with `propose_metadata_update` and `enrich_item`.")]
    pub async fn lookup_isbn(&self, #[tool(aggr)] args: IsbnArgs) -> Result<CallToolResult, McpError> {
        en::lookup_isbn_t(&self.state, args).await
    }

    #[tool(description = "Look up an arXiv preprint by ID. \
                          `format='zotero'` (default) returns a flat Zotero item ready to pass straight to `create_item`; \
                          `format='candidate'` returns an envelope `{source, fields, creators, source_url}` for use with `propose_metadata_update` and `enrich_item`.")]
    pub async fn lookup_arxiv(&self, #[tool(aggr)] args: ArxivArgs) -> Result<CallToolResult, McpError> {
        en::lookup_arxiv_t(&self.state, args).await
    }

    #[tool(description = "Score candidate metadata and produce an EnrichmentProposal (does not apply). \
                          Candidates must be lookup results obtained with `format='candidate'`. \
                          Items obtained with the default `format='zotero'` will fail validation because the scoring logic requires the envelope's `source` field.")]
    pub async fn propose_metadata_update(&self, #[tool(aggr)] args: ProposeArgs) -> Result<CallToolResult, McpError> {
        en::propose_metadata_update_t(&self.state, args).await
    }

    #[tool(description = "Compose propose+apply: only auto-applies when confidence >= threshold AND multi-source agreement. \
                          Candidates must be lookup results obtained with `format='candidate'`. \
                          Items obtained with the default `format='zotero'` will fail validation because the scoring logic requires the envelope's `source` field.")]
    pub async fn enrich_item(&self, #[tool(aggr)] args: EnrichArgs) -> Result<CallToolResult, McpError> {
        en::enrich_item_t(&self.state, args).await
    }
```

- [ ] **Step 2: Build to verify**

Run: `cargo build -p zotero-mcp 2>&1 | tail -10`

Expected: PASS.

- [ ] **Step 3: Run full test suite**

Run: `cargo test -p zotero-mcp 2>&1 | tail -10`

Expected: PASS — Slice A complete.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-mcp/src/server.rs
git commit -m "$(cat <<'EOF'
docs(server): describe format param and candidate requirement in tool docs

lookup_doi/isbn/arxiv now document both format values explicitly.
propose_metadata_update and enrich_item explicitly require
format='candidate' candidates, since scoring needs the envelope's source.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Schema-shape regression test

**Files:**
- Create: `crates/zotero-mcp/tests/schema_shape.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/zotero-mcp/tests/schema_shape.rs`:

```rust
use schemars::schema_for;
use zotero_mcp::tools::attachments::CreateItemArgs;
use zotero_mcp::tools::enrichment::{EnrichArgs, ProposeArgs};

fn property_type(schema_json: &serde_json::Value, name: &str) -> String {
    schema_json["properties"][name]["type"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| panic!(
            "property `{}` has no `type`; full schema: {}",
            name,
            serde_json::to_string_pretty(schema_json).unwrap()
        ))
}

#[test]
fn create_item_args_item_is_object_typed() {
    let schema = schema_for!(CreateItemArgs);
    let json = serde_json::to_value(&schema).unwrap();
    assert_eq!(property_type(&json, "item"), "object");
}

#[test]
fn propose_args_candidates_is_array_of_objects() {
    let schema = schema_for!(ProposeArgs);
    let json = serde_json::to_value(&schema).unwrap();
    assert_eq!(property_type(&json, "candidates"), "array");
    assert_eq!(
        json["properties"]["candidates"]["items"]["type"]
            .as_str()
            .expect("candidates.items has no type"),
        "object"
    );
}

#[test]
fn enrich_args_candidates_is_array_of_objects() {
    let schema = schema_for!(EnrichArgs);
    let json = serde_json::to_value(&schema).unwrap();
    assert_eq!(property_type(&json, "candidates"), "array");
    assert_eq!(
        json["properties"]["candidates"]["items"]["type"]
            .as_str()
            .expect("candidates.items has no type"),
        "object"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p zotero-mcp --test schema_shape 2>&1 | tail -20`

Expected: FAIL — current types (`Value`, `Vec<Value>`) produce schemas without explicit `type` on the value/items.

Note: leave the failing test in place; Task 9 makes it pass.

---

## Task 9: Change `CreateItemArgs.item`, `ProposeArgs.candidates`, `EnrichArgs.candidates` types

**Files:**
- Modify: `crates/zotero-mcp/src/tools/attachments.rs`
- Modify: `crates/zotero-mcp/src/tools/enrichment.rs`

- [ ] **Step 1: Edit `CreateItemArgs` and `create_item_t`**

In `crates/zotero-mcp/src/tools/attachments.rs`:

Change the import line `use serde_json::Value;` to:

```rust
use serde_json::{Map, Value};
```

Replace `CreateItemArgs`:

```rust
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateItemArgs {
    /// Zotero-shaped item JSON object. Required key: `itemType` (string).
    /// Other keys pass through to the Zotero Web API. The output of
    /// `lookup_doi`/`lookup_isbn`/`lookup_arxiv` with the default
    /// `format='zotero'` is directly compatible.
    pub item: Map<String, Value>,
    /// Optional collection keys to file the new item under. Equivalent to
    /// setting `collections` inside `item`; the two are unioned.
    #[serde(default)]
    pub collection_keys: Vec<String>,
}
```

Replace `create_item_t`:

```rust
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
```

- [ ] **Step 2: Edit `ProposeArgs`, `EnrichArgs`, and `parse_candidates`**

In `crates/zotero-mcp/src/tools/enrichment.rs`:

Change the import line `use serde_json::Value;` to:

```rust
use serde_json::{Map, Value};
```

Replace `ProposeArgs`:

```rust
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ProposeArgs {
    pub item_key: String,
    /// JSON array of NormalizedRecord objects (lookup_* output with format='candidate').
    pub candidates: Vec<Map<String, Value>>,
}
```

Replace `EnrichArgs`:

```rust
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EnrichArgs {
    pub item_key: String,
    pub candidates: Vec<Map<String, Value>>,
    #[serde(default)]
    pub auto_apply_threshold: Option<f64>,
}
```

Replace `parse_candidates`:

```rust
fn parse_candidates(arr: Vec<Map<String, Value>>) -> Result<Vec<NormalizedRecord>, Error> {
    arr.into_iter()
        .enumerate()
        .map(|(i, m)| {
            serde_json::from_value(Value::Object(m)).map_err(|e| {
                invalid(format!("candidates[{}] invalid NormalizedRecord: {}", i, e))
            })
        })
        .collect()
}
```

- [ ] **Step 3: Run schema-shape tests**

Run: `cargo test -p zotero-mcp --test schema_shape 2>&1 | tail -15`

Expected: PASS — 3 tests.

- [ ] **Step 4: Run full test suite**

Run: `cargo test -p zotero-mcp 2>&1 | tail -15`

Expected: PASS — every test in the crate.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/tools/attachments.rs crates/zotero-mcp/src/tools/enrichment.rs crates/zotero-mcp/tests/schema_shape.rs
git commit -m "$(cat <<'EOF'
fix(schema): structured-object types for create_item.item and candidates

Three argument fields change from raw Value / Vec<Value> to
Map<String, Value> / Vec<Map<String, Value>>:
- CreateItemArgs.item
- ProposeArgs.candidates
- EnrichArgs.candidates

This forces schemars to emit type:object (or type:array with object
items) in the tool's advertised schema, so MCP clients transmit these
arguments as structured values instead of stringified JSON blobs.
New tests/schema_shape.rs locks the contract in.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Verification checklist

After all tasks complete:

- [ ] `lookup_doi`, `lookup_isbn`, `lookup_arxiv` accept a `format` parameter.
- [ ] `format='zotero'` (default) emits a flat Zotero JSON object with `extra` field containing `source: …` and (if applicable) `sourceURL: …` lines.
- [ ] `format='candidate'` emits the original `NormalizedRecord` envelope.
- [ ] `format='garbage'` returns an `invalid_params` error.
- [ ] OpenLibrary's freeform `publish_date` is normalised to ISO 8601 (or passed through unchanged when unparseable).
- [ ] OpenLibrary `source_url` points at `/isbn/{isbn}`, not the base URL.
- [ ] `normalized_to_item` reads `itemType` from `record.fields`, rewrites creators as `{creatorType, firstName, lastName}`, and appends provenance to `extra`.
- [ ] `CreateItemArgs.item`, `ProposeArgs.candidates`, and `EnrichArgs.candidates` produce schemas with explicit `type: object`/`array` (verified by `tests/schema_shape.rs`).
- [ ] Tool descriptions document the `format` parameter on all three lookups and the `candidate` requirement on `propose_metadata_update` / `enrich_item`.
- [ ] `cargo test -p zotero-mcp` is green.
