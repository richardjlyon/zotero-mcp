# Spec: Tool output normalisation for lookup_* and create_item

**Status:** Approved design, ready for plan-writing.
**Author:** Richard Lyon (with Claude Opus 4.7).
**Date:** 2026-05-13.
**Goal:** Make the obvious workflow `lookup_*(...)` → `create_item(item=…)` work end-to-end through the MCP wire without a transform step between, while preserving the existing enrichment scoring pipeline that depends on the candidate envelope shape.

---

## Problem

A user (Richard, writing a skill that drives these tools) observed that the lookup tools and the `create_item` tool advertise compatible shapes but in practice do not. Three independent issues compound:

### Issue 1: Lookup tools return an envelope; `create_item` expects a flat Zotero item

`lookup_doi`, `lookup_isbn`, and `lookup_arxiv` (the three tool wrappers in `crates/zotero-mcp/src/tools/enrichment.rs`) all return `NormalizedRecord` — defined in `crates/zotero-mcp/src/core/enrichment/mod.rs:13-20`:

```rust
pub struct NormalizedRecord {
    pub source: String,
    pub fields: serde_json::Map<String, Value>,
    pub creators: Vec<Creator>,
    pub source_url: Option<String>,
}
```

The actual book/article metadata is nested one level inside `fields` and `creators`. Provenance lives next to it in `source` and `source_url`.

`create_item` (`crates/zotero-mcp/src/tools/attachments.rs:131-149` and `crates/zotero-mcp/src/core/writer/items.rs:58-125`) accepts a Zotero-shaped flat JSON object:

```json
{
  "itemType": "book",
  "title": "Some Book",
  "creators": [
    {"creatorType": "author", "firstName": "Jane", "lastName": "Doe"}
  ],
  "date": "2020",
  "publisher": "BookCo"
}
```

The `create_item` tool description in `crates/zotero-mcp/src/server.rs:195` claims *"For metadata-discovery flows, lookup_doi / search_crossref return the JSON shape directly compatible with this tool"*. That claim is currently false for all three lookup tools.

There is already a helper `normalized_to_item()` (`crates/zotero-mcp/src/core/enrichment/mod.rs:36-49`) that flattens a record, but no tool wrapper calls it, and it discards `source` and `source_url` entirely.

### Issue 2: `CreateItemArgs.item` has an empty JSON schema

`tools/attachments.rs:134` declares the field as raw `serde_json::Value`:

```rust
pub struct CreateItemArgs {
    pub item: Value,
    ...
}
```

`schemars` emits an empty schema (`{}`) for `Value`, meaning the advertised tool schema for `item` carries no type. Some MCP clients (and proxies) interpret an untyped argument by transmitting it as a stringified JSON blob — i.e. `{"item": "{\"itemType\":\"book\"}"}` (the value of `item` is a JSON-encoded string) rather than `{"item": {"itemType": "book"}}` (a structured object). The server then receives `Value::String("{...}")` instead of `Value::Object(...)`, and `create_item` rejects it with `"item must be a JSON object"` — a confusing error far from the real cause.

The same untyped-array-items issue applies to `ProposeArgs.candidates: Vec<Value>` (`tools/enrichment.rs:117`) and `EnrichArgs.candidates: Vec<Value>` (`tools/enrichment.rs:176`).

(`UpdateFieldsArgs.fields: BTreeMap<String, Value>` and `ApplyArgs.proposal: BTreeMap<String, Value>` are already correctly typed by `schemars` as `{type: object, additionalProperties: true}` — they are not in scope.)

### Issue 3: No regression test for the advertised contract

There is no test today that calls `lookup_*` and feeds its literal output into `create_item`. Both halves are tested in isolation; the contract between them is implicit and currently broken.

---

## Goals

1. The output of `lookup_doi`, `lookup_isbn`, and `lookup_arxiv` can, by default, be passed directly as the `item` argument to `create_item` with no transform. Provenance information is preserved in Zotero's `extra` field.
2. The existing enrichment workflow — `lookup_* → propose_metadata_update` / `enrich_item` — continues to work, with the caller opting in via a `format: "candidate"` parameter on the lookup tools.
3. The MCP-advertised schema for `create_item.item`, `propose_metadata_update.candidates[]`, and `enrich_item.candidates[]` declares `type: object` so MCP clients transmit them as structured values, not stringified JSON.
4. An end-to-end test proves the contract: lookup result → create_item, over the MCP wire (or as close to it as the test infrastructure can get).

## Non-goals

- Renaming the internal `Creator` struct's field names from snake_case to camelCase (it is used by readers, scoring, and the diff machinery; the rename is kept local to the flattener).
- Adding new lookup sources, or changing `search_crossref` / `search_semantic_scholar` output shapes. The `format` parameter design applies to those if wanted later but is not implemented now.
- Changes to `create_item`'s core writer logic or to its non-`item` parameters.
- Changes to the `Item` reader struct, `get_item`, or any other tool not mentioned in this spec.
- A typed `ZoteroItemInput` struct for `create_item`. Zotero's vocabulary is large and per-item-type; a free-form `Map<String, Value>` with `additionalProperties: true` is the right shape.

---

## Decisions

1. **Add a `format` parameter** to `lookup_doi`, `lookup_isbn`, `lookup_arxiv`. Values: `"zotero"` (default) and `"candidate"`. `"zotero"` returns the flat Zotero-shaped JSON ready for `create_item`. `"candidate"` returns today's envelope shape for use with `propose_metadata_update` / `enrich_item`.
2. **Default to `"zotero"`** because it matches the advertised `create_item` contract and is the more common standalone use. This is a breaking change for any caller that previously chained `lookup_* → propose_metadata_update` without specifying `format`; mitigated by explicit mention in the propose/enrich tool descriptions. The project is pre-1.0 (0.2.0); breaking change is acceptable.
3. **Stash provenance in Zotero's `extra` field** when emitting flat output. Format: newline-separated `key: value` lines, matching Zotero's existing convention (e.g. BBT's `Citation Key:` line):
   ```
   source: openlibrary
   sourceURL: https://openlibrary.org/isbn/9780000000000
   ```
4. **Normalise OpenLibrary's `publish_date` to ISO 8601.** Audit CrossRef and arXiv against ISO 8601 too; fix only where they diverge. (Initial inspection suggests CrossRef produces `"YYYY"`, `"YYYY-MM"`, or `"YYYY-MM-DD"` correctly; arXiv splits `"2024-01-01T00:00:00Z"` at `T` to produce `"2024-01-01"`. Both already valid. Confirm during implementation.)
5. **Fix the JSON-schema bugs by changing types**, not by adding `#[schemars(schema_with = …)]` attributes:
   - `CreateItemArgs.item: Value` → `serde_json::Map<String, Value>`
   - `ProposeArgs.candidates: Vec<Value>` → `Vec<serde_json::Map<String, Value>>`
   - `EnrichArgs.candidates: Vec<Value>` → `Vec<serde_json::Map<String, Value>>`
6. **Test the round-trip end-to-end through an in-memory MCP transport.** Use `tokio::io::duplex()` paired with `rmcp::serve_server` and a client peer. Fall back to direct `ZoteroServer::call_tool(...)` invocations plus a separate `schema_shape.rs` test if the in-memory transport doesn't compose cleanly with rmcp 0.1.5.

---

## Architecture

Three independent slices, in dependency order:

```
┌──────────────────────────────────────────────────────────────────┐
│ Slice A: Output shape switch                                      │
│   core/enrichment/openlibrary.rs                                  │
│     ├─ parse_date helper: freeform -> ISO 8601                    │
│     └─ fix source_url to point at the ISBN record                 │
│   core/enrichment/mod.rs                                          │
│     └─ extend normalized_to_item() to stash source + sourceURL    │
│        as `extra` lines, and rename creator fields to camelCase   │
│   tools/enrichment.rs                                             │
│     └─ DoiArgs/IsbnArgs/ArxivArgs gain `format` field             │
│     └─ lookup_*_t functions branch on format                      │
│   server.rs                                                       │
│     └─ tool descriptions updated for all five affected tools      │
│                                                                   │
│ Slice B: Schema audit                                             │
│   tools/attachments.rs::CreateItemArgs.item                       │
│     Value -> serde_json::Map<String, Value>                       │
│   tools/enrichment.rs::ProposeArgs.candidates                     │
│   tools/enrichment.rs::EnrichArgs.candidates                      │
│     Vec<Value> -> Vec<serde_json::Map<String, Value>>             │
│   Downstream wrapping in create_item_t / parse_candidates         │
│                                                                   │
│ Slice C: End-to-end MCP roundtrip test                            │
│   tests/lookup_to_create_roundtrip.rs (new)                       │
│     - in-memory MCP transport pair                                │
│     - mocked OpenLibrary / CrossRef / arXiv / Zotero Local API    │
│     - for each of isbn/doi/arxiv: CallTool(lookup_*) then         │
│       CallTool(create_item, item=<previous response>)             │
│   tests/schema_shape.rs (new, also serves as fallback)            │
│     - asserts schemars-generated schemas have correct types       │
└──────────────────────────────────────────────────────────────────┘
```

Boundaries:
- `core/enrichment/*` remains the source of truth for `NormalizedRecord`. The envelope shape is still produced internally and is still what `format="candidate"` returns on the wire.
- `tools/enrichment.rs` is the only place that picks between flat and envelope output. The scoring code in `core/enrichment/scoring.rs` and `propose.rs` continues to work with `NormalizedRecord`.
- Schema audit is purely a `JsonSchema`/type-shape concern in `tools/*.rs`; no behavioural change.

---

## Slice A: Output shape switch

### `core/enrichment/openlibrary.rs`

**Add** an internal `parse_date(s: &str) -> String` helper. Behaviour:

| Input | Output |
|---|---|
| `"2020"` | `"2020"` |
| `"March 5, 2020"`, `"Mar 5, 2020"`, `"5 March 2020"` | `"2020-03-05"` |
| `"March 2020"`, `"Mar 2020"` | `"2020-03"` |
| `"1998-09-08"` | `"1998-09-08"` (pass-through) |
| Any unparseable string | input string unchanged (do not drop the field) |

Implementation strategy: try a small set of explicit format patterns in order (`%Y-%m-%d`, `%Y-%m`, `%Y`, `%B %d, %Y`, `%b %d, %Y`, `%d %B %Y`, `%d %b %Y`, `%B %Y`, `%b %Y`). Fall through to the original string on the last failure.

**Fix** `source_url`. Today (`openlibrary.rs:79`):
```rust
source_url: Some(format!("{}{}", self.base, "/")),
```
Change to:
```rust
source_url: Some(format!("{}/isbn/{}", self.base, isbn)),
```
so the URL actually identifies the looked-up record.

### `core/enrichment/mod.rs`

`normalized_to_item(record, item_type)` currently writes `record.fields` and a `creators` array built from `serde_json::to_value(c)` (which uses the snake_case `Creator` field names — wrong for Zotero). Extend the function:

1. **Read `itemType` from `record.fields`** rather than taking it as a separate argument. All three sources populate `fields["itemType"]` (openlibrary: `"book"`; crossref: mapped from `type`; arxiv: `"preprint"`). Simplifies callers and removes a redundant argument. The function signature changes to `normalized_to_item(record: &NormalizedRecord) -> Value`.
2. **Rewrite creators inline** with Zotero's vocabulary: `{creatorType, firstName, lastName}`, omitting `orderIndex` (Zotero infers order from array position) and any `None` first/last name fields.
3. **Append provenance to `extra`**. Build a string with `source: {record.source}\n` and (if `source_url.is_some()`) `sourceURL: {url}\n`. If `record.fields["extra"]` exists, append to it (preserving any existing content); otherwise insert.

The internal `Creator` struct in `crates/zotero-mcp/src/core/types.rs:24-32` keeps its snake_case `firstName`/`lastName` field naming because it is used by readers, scoring, diffing, and existing tests. The Zotero-wire rename lives only inside `normalized_to_item`.

### `core/enrichment/crossref.rs` and `arxiv.rs`

Audit during implementation. Expected outcome: no changes needed because CrossRef's `extract_date` already produces ISO 8601 (`"2024"` / `"2024-03"` / `"2024-03-15"`) and arXiv's parser already splits at `T`. If the audit surfaces a divergence (e.g. CrossRef emits `"2024-3"` instead of `"2024-03"`), fix locally; do not refactor into a shared helper.

### `tools/enrichment.rs`

**Args structs** gain a `format` field:

```rust
fn default_format() -> String { "zotero".into() }

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DoiArgs {
    pub doi: String,
    #[serde(default = "default_format")]
    pub format: String,
}
// Same for IsbnArgs and ArxivArgs.
```

**Tool functions** branch on `format`:

```rust
pub async fn lookup_isbn_t(s: &AppState, a: IsbnArgs) -> Result<CallToolResult, Error> {
    let record = s.openlibrary.lookup_isbn(&a.isbn).await.map_err(map_err)?;
    let body = match a.format.as_str() {
        "candidate" => serde_json::to_value(&record).unwrap(),
        "zotero" => normalized_to_item(&record),
        other => return Err(invalid(format!(
            "format must be 'zotero' or 'candidate' (got '{}')", other
        ))),
    };
    Ok(CallToolResult::success(vec![Content::json(body)?]))
}
```

Same pattern for `lookup_doi_t` and `lookup_arxiv_t`.

### Tool descriptions (`crates/zotero-mcp/src/server.rs`)

Update descriptions for:

- **`lookup_doi`**: document both formats explicitly. Example: *"Look up a DOI via CrossRef. `format='zotero'` (default) returns a flat Zotero item ready for `create_item`; `format='candidate'` returns an envelope `{source, fields, creators, source_url}` for use with `propose_metadata_update` and `enrich_item`."*
- **`lookup_isbn`**, **`lookup_arxiv`**: parallel wording.
- **`propose_metadata_update`** and **`enrich_item`**: add *"Candidates must be lookup results obtained with `format='candidate'`. Items obtained with the default `format='zotero'` will fail validation because the scoring logic requires the envelope's `source` field."*
- **`create_item`**: keep the existing description; its claim about lookup compatibility is now true for the default flow.

---

## Slice B: Schema audit

Three type changes. Each forces `schemars` to emit a constrained schema, which in turn forces MCP clients to transmit the value as a structured object rather than a stringified blob.

| File | Field | Before | After |
|---|---|---|---|
| `tools/attachments.rs:134` | `CreateItemArgs.item` | `Value` | `serde_json::Map<String, Value>` |
| `tools/enrichment.rs:117` | `ProposeArgs.candidates` | `Vec<Value>` | `Vec<serde_json::Map<String, Value>>` |
| `tools/enrichment.rs:176` | `EnrichArgs.candidates` | `Vec<Value>` | `Vec<serde_json::Map<String, Value>>` |

Downstream call-site impact:

- **`create_item_t`** (`tools/attachments.rs:141-149`): currently passes `&a.item` to `create_item(item: &Value, ...)`. Change to `&Value::Object(a.item.clone())`. One-line wrap.
- **`parse_candidates`** (`tools/enrichment.rs:120-129`): signature changes from `Vec<Value>` to `Vec<serde_json::Map<String, Value>>`. The body wraps each `Map` in `Value::Object(map)` before calling `serde_json::from_value::<NormalizedRecord>(...)`.
- **No existing tests touch this path.** A grep for `parse_candidates`, `ProposeArgs`, and `EnrichArgs` across `crates/zotero-mcp/tests/` returns no matches. The schema-audit change has zero ripple into existing tests.

No `#[schemars(...)]` attributes are used. The type system carries the schema constraint.

---

## Slice C: End-to-end MCP roundtrip test

### Primary plan: in-memory MCP transport pair

New file `crates/zotero-mcp/tests/lookup_to_create_roundtrip.rs`. Test infrastructure:

```rust
async fn spawn_server_and_client() -> (ClientPeer, MockServer, MockServer, MockServer, MockServer)
```

1. Start four `wiremock::MockServer` instances (OpenLibrary, CrossRef, arXiv, Zotero Local API).
2. Build an `AppState` that points the enrichment clients and Local API at the mock URIs.
3. Create paired streams with `tokio::io::duplex(64 * 1024)`.
4. Spawn the server side: `rmcp::serve_server(ZoteroServer::new(state), server_half).await`.
5. Spawn the client side: `rmcp::serve_client(...)` (or its equivalent in rmcp 0.1.5) on the client half, returning a client peer with `.call_tool(...)`.
6. Return the client peer and the four mock servers so subtests can program responses and assert request bodies.

Three subtests, each parameterising the lookup tool, fixture, and expected `itemType`:

```rust
#[tokio::test]
async fn lookup_isbn_to_create_item_roundtrip() {
    // program OpenLibrary mock to return an ISBN fixture
    // program Zotero mock to accept POST /items and return {successful:{0:{key,version}}}
    let r1 = client.call_tool("lookup_isbn", json!({"isbn": "9780000000000"})).await?;
    let item = extract_json_content(r1);
    let r2 = client.call_tool("create_item", json!({"item": item})).await?;
    // assert r2 has a key and version
    // assert Zotero mock received a body where the first element has "itemType":"book"
}
```

Same for `lookup_doi_to_create_item_roundtrip` and `lookup_arxiv_to_create_item_roundtrip`.

What this catches that nothing else does: the JSON-RPC serialise→deserialise round trip. If a regression reintroduced the empty schema, the client side would (depending on which MCP client we're testing against) potentially stringify the `item` argument, and `create_item_t` would fail to deserialise into `Map<String, Value>`. The roundtrip is the only test that sees this layer.

### Spike step before writing the three subtests

Before authoring the full test, write a one-liner that just calls `ping` over the in-memory transport. Goals:

- Verify `serve_server` + a duplex stream compose with rmcp 0.1.5.
- Verify the client peer's `call_tool` returns a usable `CallToolResult`.
- Establish baseline latency (target: <100ms per subtest).

If the spike works, write the three subtests in the same module.

### Fallback if the spike does not compose

If `tokio::io::duplex` + rmcp's serve loops cannot be wired up without significant test-infra work, fall back to:

1. **Direct dispatch**: call `ZoteroServer::call_tool(CallToolRequestParam {...}, ctx).await` with a hand-constructed `RequestContext`. Skips the JSON-RPC wire but still exercises the rmcp tool macro dispatch and the schemars-driven argument parsing. (The `RequestContext` requires a `Peer<RoleServer>`, which itself requires a transport — confirm during the spike whether this fallback is even available without spinning up a transport.)
2. **Schema audit guard**: `crates/zotero-mcp/tests/schema_shape.rs` (new). Uses `schemars::schema_for!(CreateItemArgs)`, `ProposeArgs`, `EnrichArgs` and asserts the generated schemas have `type: object` / `type: array` with `items.type: object` at the right paths. Fast, self-contained, no transport. Validates Slice B independently of the roundtrip test.

The `schema_shape.rs` test is written regardless — it adds the fastest possible regression guard for Slice B at near-zero cost.

---

## Testing strategy

### Existing tests — keep passing

| Test | Notes |
|---|---|
| `tests/enrich_openlibrary.rs::lookup_isbn_normalizes` | Asserts on the envelope via the core client. Unchanged. |
| `tests/enrich_crossref.rs::lookup_doi_normalizes_to_zotero_fields` | Asserts on the envelope via the core client. Unchanged. |
| `tests/enrich_arxiv.rs::lookup_arxiv_parses_atom` | Asserts on the envelope via the core client. Unchanged. |
| `tests/writer_create_item.rs` (4 tests) | Mocked Zotero, calls `create_item()` core. Unchanged. |
| `tests/enrich_propose.rs` | Exercises `compute_diff` directly; does not touch `parse_candidates` or the tool layer. Unchanged. |

### New tests

1. **`tests/enrich_openlibrary.rs` — extend**
   - Five date-parser subtests (one per row in the table above).
   - `source_url` subtest: asserts the URL now points at `/isbn/{isbn}`.
   - Flat-shape subtest: calls `lookup_isbn_t` with `format: "zotero"`, parses `CallToolResult.content[0]` as JSON, asserts top-level has `itemType`/`title`/`creators`/`extra`; `creators[0]` has `creatorType`/`firstName`/`lastName` (no underscores, no `orderIndex`); `extra` contains `source: openlibrary` and `sourceURL: …`; no `source`, `source_url`, or `fields` keys at the top level.

2. **`tests/enrich_crossref.rs` — extend**
   - Flat-shape subtest, structurally parallel to (1)'s flat-shape case. Asserts `extra` contains `source: crossref` and a `sourceURL` line (since CrossRef populates `source_url` from the `URL` field).

3. **`tests/enrich_arxiv.rs` — extend**
   - Flat-shape subtest, structurally parallel. arXiv's `source_url` is `None` today, so the test asserts `extra` contains `source: arxiv` and no `sourceURL` line.

4. **`tests/lookup_to_create_roundtrip.rs` — new** (Slice C primary)
   - Spike test: `ping` over in-memory transport.
   - `lookup_isbn → create_item` roundtrip.
   - `lookup_doi → create_item` roundtrip.
   - `lookup_arxiv → create_item` roundtrip.
   - Each subtest uses `wiremock::matchers::body_partial_json` on the Zotero mock to assert that the received body's first element has the expected `itemType` — which transitively proves the JSON was transmitted as a structured object, not stringified.

5. **`tests/schema_shape.rs` — new**
   - `schemars::schema_for!(CreateItemArgs)` → asserts `properties.item.type == "object"`.
   - `schemars::schema_for!(ProposeArgs)` → asserts `properties.candidates.type == "array"` AND `properties.candidates.items.type == "object"`.
   - Same for `EnrichArgs`.

### Coverage matrix

| Concern | Test |
|---|---|
| OpenLibrary date parsing edge cases | `enrich_openlibrary.rs` extensions |
| OpenLibrary `source_url` points at the record | `enrich_openlibrary.rs` extension |
| Flat Zotero shape from `lookup_*_t` | `enrich_{openlibrary,crossref,arxiv}.rs` flat-shape subtests |
| `extra` field provenance stashing | `enrich_{openlibrary,crossref,arxiv}.rs` flat-shape subtests |
| Creator camelCase rename | `enrich_*.rs` flat-shape subtests |
| Tool schemas have `type: object` / `type: array` with `items.type: object` | `schema_shape.rs` |
| End-to-end lookup→create over MCP wire | `lookup_to_create_roundtrip.rs` |
| Existing envelope-consuming flows | Existing `enrich_propose.rs` (unchanged) |

---

## Implementation order

1. **Slice A** — output shape switch + tool descriptions. Self-contained; tests 1, 2, 3 cover it.
2. **Slice B** — schema type changes. Independently mergeable from A but easier to debug afterwards. Test 5 (`schema_shape.rs`) is written here.
3. **Slice C** — roundtrip test. Spike first, then three subtests if the spike succeeds. Validates A and B together.

Each slice ships as its own commit. A and B can land in either order; C blocks on both.

---

## Risks

1. **Breaking change for chained callers.** Any caller previously doing `lookup_* → propose_metadata_update` without an explicit `format` will start sending flat Zotero items where envelopes are expected. `parse_candidates` will fail with `invalid_params: candidates[N] invalid NormalizedRecord: missing field 'source'`. The error is clear and surfaces at the right boundary, but it *is* breaking. Mitigated by explicit mention in the propose/enrich tool descriptions. Pre-1.0 project, version is 0.2.0; breaking change is acceptable.

2. **Roundtrip test infra may not compose with rmcp 0.1.5.** The library's own tests do not exercise the embedded-server+client pattern. Mitigated by:
   - Spike step (build `ping`-only roundtrip first).
   - `schema_shape.rs` fallback that keeps Slice B validated even if Slice C reduces to direct `call_tool()` invocations or is dropped entirely.

3. **OpenLibrary's `publish_date` has more freeform variants than the documented five cases.** Real-world data includes `"c. 2020"`, `"[2020]"`, `"2020 [reprinted 2024]"`, abbreviated months in non-English locales, etc. The parser returns the original string on failure rather than dropping the field, so worst case is a non-ISO date survives — which is exactly what happens today. No regression.

4. **Internal `Creator` snake_case vs Zotero camelCase.** The rename lives only inside `normalized_to_item`. If anyone later assumes the wire shape and the internal struct share field names, they will be wrong. Mitigation: a comment on `normalized_to_item` documenting the rename and why the internal struct does not change.

---

## Decisions deferred to implementation

- Exact `parse_date` implementation. Probably explicit format-string attempts in order; falls back to original string. If `chrono` is not already a dep, evaluate adding it vs writing a small custom parser. (Inspection of `Cargo.toml` suggests `chrono` is not in the workspace dependencies. The custom parser is preferred to avoid a new dep for what is a small set of patterns.)
- Whether the spike for Slice C uses `rmcp::serve_server` directly or a higher-level helper. Confirm by reading rmcp 0.1.5 source during the spike.
- Whether `serde_json::Map` or `BTreeMap` is the right concrete type for the schema-audit changes. Both produce `type: object`. `serde_json::Map` preserves insertion order which is nicer for debugging and round-tripping. Default to `serde_json::Map`.
