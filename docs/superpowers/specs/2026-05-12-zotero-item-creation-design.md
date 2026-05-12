# Zotero Item Creation — Design

**Date:** 2026-05-12
**Status:** Approved (design phase)
**Author:** rjl
**Component:** `zotero-mcp` — three new write tools (`create_item`, `attach_file`, `attach_link`)

## 1. Overview

`zotero-mcp` can mutate existing Zotero items (notes, tags, fields, collection
membership, delete) but cannot create new ones. This design adds three
primitives so Claude can build a Zotero library from scratch via MCP — given
a PDF on disk, a DOI, a URL, or just a metadata blob.

### 1.1 Goals

- One primitive to create any Zotero item (`create_item`), taking the
  Zotero-shaped JSON that `lookup_doi` / `search_crossref` etc. already
  return. No new schema to learn.
- One primitive to attach a local file (`attach_file`), supporting both of
  Zotero's local-file modes: `imported_file` (bytes go to Zotero cloud) and
  `linked_file` (BYO-storage / Resilio / Syncthing).
- One primitive to attach a URL as a child link (`attach_link`).
- Each primitive small, sharp, testable; workflow composition stays Claude-side.

### 1.2 Non-goals

- An opinionated `ingest_pdf` composite. The 'PDF on disk → item' flow has
  too many branches (DOI extraction failure, multi-candidate matches,
  needs-user-input cases) to bake into a single MCP call.
- Dedup logic inside `create_item`. Callers that care call `search_items`
  by DOI/title first.
- `imported_url` (Web Clipper snapshot) mode for attachments. Out of scope;
  the primary use case is local files, not server-fetched snapshots.
- PDF metadata extraction from binary content (`extract_pdf_identifiers`).
  Captured as a future optional helper, not in this design.

## 2. Verified Environment

Confirmed during design on 2026-05-12:

- Existing `LocalApi` writer client at `crates/zotero-mcp/src/core/writer/client.rs`
  already routes writes through `api.zotero.org/users/<id>/...` with
  `Bearer <api_key>` auth.
- `NormalizedRecord { source, fields: Map<String, Value>, creators:
  Vec<Creator>, source_url }` is the existing wire shape from `lookup_doi` and
  friends — already Zotero-flavoured.
- Existing write pattern: `api.write_request(Method, path)?` → send →
  `handle_write_response`. 412 → version conflict, else `Error::LocalApi`.
- The user's Zotero is configured with `linked_file` attachments, base dir
  under a Resilio-synced folder. They explicitly want the published crate to
  serve both storage models cleanly.

## 3. API Surface

Three new MCP tools, conforming to existing write-tool patterns.

### 3.1 `create_item`

```
Input:  { item: ZoteroItemJson, collection_keys?: [string] }
Output: { item_key: string, version: i64 }
```

`item` is a Zotero-shaped object — same shape as `NormalizedRecord.fields`
plus `itemType` and `creators`. Required: `itemType`. Everything else
optional and pass-through.

`collection_keys`, if provided, are inlined into the item's `collections`
field on creation — saves a follow-up `add_to_collection` round-trip.

### 3.2 `attach_file`

```
Input:  {
  parent_key: string,
  file_path: string,
  mode?: "imported_file" | "linked_file",
  filename?: string,
  content_type?: string,
}
Output: { attachment_key: string }
```

`file_path` is an absolute local path. `filename` defaults to the basename;
`content_type` defaults to mime-guess by extension. `mode` defaults to
`cfg.zotero.attachment_mode` (see §6 Configuration).

### 3.3 `attach_link`

```
Input:  { parent_key: string, url: string, title?: string }
Output: { attachment_key: string }
```

Single POST. Creates a child attachment row with `linkMode: linked_url`,
storing the URL only. No bytes transfer.

## 4. Item JSON Shape

`create_item` accepts a Zotero-shaped JSON object. The shape is exactly what
the Zotero Web API expects on POST:

```json
{
  "itemType": "journalArticle",
  "title": "Roles of Earth's Albedo Variations...",
  "creators": [
    { "creatorType": "author", "firstName": "Ned", "lastName": "Nikolov" },
    { "creatorType": "author", "firstName": "Karl F.", "lastName": "Zeller" }
  ],
  "date": "2024-08-20",
  "publicationTitle": "Geomatics",
  "volume": "4",
  "issue": "3",
  "pages": "311-341",
  "DOI": "10.3390/geomatics4030017",
  "ISSN": "2673-7418",
  "url": "https://www.mdpi.com/2673-7418/4/3/17",
  "abstractNote": "...",
  "language": "en",
  "tags": [{ "tag": "albedo" }, { "tag": "climate-sensitivity" }],
  "collections": ["NQF36WE7"]
}
```

Decisions:

- **Pass-through.** The tool doesn't strip or rename fields. Whatever the
  caller supplies goes to Zotero. Unknown fields are ignored by Zotero (a
  documented behaviour); we don't filter pre-emptively.
- **No client-side schema validation** beyond `itemType` being present and
  a string. Zotero is the schema authority — invalid fields surface as
  `Error::LocalApi { status: 400, body }` from the API. Cheaper than
  re-implementing Zotero's schema in Rust.
- **`collections` field on the item** does the same job as a follow-up
  `add_to_collection` call. Both supported; whichever Claude finds more
  convenient.
- **`tags` is an array of objects** (`[{ "tag": "x" }]`), not strings —
  that's Zotero's native shape. Documented in the tool description so
  callers don't have to look it up.
- **`creators`** uses Zotero's vocabulary (`creatorType`: author, editor,
  translator, etc.). The existing `Creator` struct already matches.

**Helper conversion (in-process, non-MCP):** `normalized_to_item(record:
&NormalizedRecord, item_type: &str) -> Value` flattens
`record.fields` to top level, appends `creators`, adds `itemType`. Used
internally by integration tests and exposed for callers who already hold a
`NormalizedRecord`. Not a separate MCP tool — it's one of those one-liner
conveniences.

## 5. File Upload Protocol

`attach_file` branches on `mode`. The two paths share the parent-exists
check and the post-response error handling; they diverge entirely on the
upload mechanism.

### 5.1 `mode = "imported_file"` (Zotero's default)

The Zotero documented three-step path against `api.zotero.org`.

**Step 5.1a — Register the attachment item:**

```
POST /users/<userID>/items
[
  {
    "itemType": "attachment",
    "parentItem": "<parent_key>",
    "linkMode": "imported_file",
    "title": "<filename>",
    "filename": "<filename>",
    "contentType": "application/pdf",
    "charset": "",
    "md5": "<hex>",
    "mtime": <unix-ms>
  }
]
```

The `md5` and `mtime` fields are populated up-front from the local file
(computed once at the start of `attach_file`). Response → new `attachment_key`.

**Step 5.1b — Authorize the upload:**

```
POST /users/<userID>/items/<attachment_key>/file
Content-Type: application/x-www-form-urlencoded
If-None-Match: *
Body: md5=<hex>&filename=<name>&filesize=<bytes>&mtime=<unix-ms>
```

Two possible responses:

- `{ "exists": 1 }` — Zotero already has this byte-identical file by md5.
  **Short-circuit done**; no upload performed. Patch the attachment with
  md5/mtime/filename and return.
- `{ "url", "contentType", "prefix", "suffix", "uploadKey" }` — signed URL
  for the upload.

**Step 5.1c — Upload the bytes (only when needed):**

```
PUT <url>
Content-Type: <returned content type>
Body: prefix + file_bytes + suffix
```

Then register the completed upload:

```
POST /users/<userID>/items/<attachment_key>/file?upload=<uploadKey>
If-None-Match: *
```

A 204 means done.

**Practical points:**

- File md5 computed once at the start; reused in 5.1a / 5.1b / 5.1c.
- `prefix` and `suffix` are S3 multipart-form bookends concatenated raw with
  the file bytes. Not headers — literal body bytes. Easy to get wrong; the
  unit tests for this branch capture a real PUT body and assert byte exactness.
- 50 MB hard cap on `file_path` size for sanity. Configurable via
  `cfg.zotero.max_attachment_bytes` (default `50 * 1024 * 1024`).
- The whole sequence is **idempotent on md5**: re-running an interrupted
  attach with the same file produces `"exists": 1` at step 5.1b.

### 5.2 `mode = "linked_file"` (BYO storage)

Single POST. No upload, no md5 negotiation.

```
POST /users/<userID>/items
[
  {
    "itemType": "attachment",
    "parentItem": "<parent_key>",
    "linkMode": "linked_file",
    "title": "<filename>",
    "path": "attachments:<rel_to_base_dir>",
    "contentType": "application/pdf"
  }
]
```

The `path` field uses Zotero's `attachments:<rel>` convention when the file
lives inside `cfg.zotero.linked_attachment_base_dir`. Computation:

```
rel = file_path.strip_prefix(linked_attachment_base_dir)?
```

If `strip_prefix` fails, the file isn't under the configured base — return
`Error::AttachmentOutsideBaseDir { file_path, base_dir }`. **We do not move
files automatically.** Explicit user action keeps the data layout
predictable.

If `linked_attachment_base_dir` is unset and mode is `linked_file`, the tool
falls back to storing an absolute path (`path: "<absolute>"` with no
`attachments:` prefix) and emits a `WARN` log noting the file won't
replicate to other Zotero clients. Zotero allows absolute paths but doesn't
recommend them — this preserves the published-crate use case for users who
prefer absolute paths and accept the trade-off.

### 5.3 Alternative considered: local-storage shortcut

For users running Zotero desktop locally, we could skip the documented S3
path entirely for `imported_file` mode:

1. Create the attachment item via Web API (just step 5.1a, with the md5).
2. Drop the file in `~/Zotero/storage/<attachment_key>/<filename>` directly.
3. Let Zotero desktop pick it up on next sync.

Faster (no S3 upload), but **rejected** because: (a) undocumented Zotero
behaviour, (b) requires Zotero desktop running on the host, (c) skips
Zotero's own indexer pipeline so full-text search may not pick the file up
until next desktop pass, (d) doesn't generalise to remote / cloud-only
users of the crate. The documented path is solid and matches what every
other Zotero client uses.

## 6. Configuration

Additions to the `[zotero]` section of `config.toml`:

```toml
# Storage model for attachments created via attach_file. Default mirrors
# Zotero's own default behaviour. Set to "linked_file" for BYO-storage
# users (Resilio Sync, Syncthing, NAS-backed Zotero data dirs).
attachment_mode = "imported_file"

# Required when attachment_mode = "linked_file". Absolute path to the
# Zotero "Linked Attachment Base Directory" (Zotero Preferences →
# Advanced → Files & Folders). Files attached via attach_file must live
# inside this directory.
linked_attachment_base_dir = "/Users/rjl/Resilio/Zotero-Attachments"

# Per-file size ceiling for attach_file. Anything larger is rejected
# pre-flight. 50 MB is generous for academic PDFs.
max_attachment_bytes = 52428800
```

All three are optional with sensible defaults. The `linked_file` mode is
fully usable without `linked_attachment_base_dir` (falls back to absolute
paths with a WARN log) — the config knob is for cross-device replication.

## 7. Error Model

New variants on `core::error::Error`:

```rust
#[error("attachment file not found: {0}")]
AttachmentFileNotFound(PathBuf),

#[error(
    "attachment file {file_path} is not inside the configured \
     linked_attachment_base_dir ({base_dir}). Move it in first, or pass \
     mode = \"imported_file\" for this call."
)]
AttachmentOutsideBaseDir { file_path: PathBuf, base_dir: PathBuf },

#[error("zotero file upload failed at {stage}: {detail}")]
UploadFailed { stage: &'static str, detail: String },

#[error("attachment file {file_path} exceeds max_attachment_bytes ({limit})")]
AttachmentTooLarge { file_path: PathBuf, limit: usize },
```

`stage` takes one of three string literals — `"authorize"`, `"s3_put"`,
`"register"` — so callers (and humans) can tell which step of the
three-step upload tripped.

**Reused (no change):**

- `Error::WriteApiKeyMissing` — when `cfg.zotero.api_key` is `None` and a
  write is attempted. Pre-existing; covers `create_item`, `attach_file`,
  `attach_link` uniformly.
- `Error::LocalApi { status, body }` — for anything else the Zotero API
  rejects (400 = malformed item, 403 = permission, 409 = parent item
  missing, 5xx). Body string carries Zotero's own error message,
  truncated at 500 chars to bound logspam from giant HTML 5xx pages.
- `Error::Io(_)` — file open / read failures, md5 computation, etc.

**Behaviour:**

- Cheap validation (file exists, `itemType` present, base-dir relativity,
  size check) happens **before** any network call. Catches dumb mistakes
  without burning a Zotero API round-trip.
- No retry logic. Zotero `429` rate-limit responses bubble up as
  `LocalApi { status: 429, body }`; callers (or Claude) decide whether to
  back off. Adding retries inside the tool would mask transient infra
  issues and complicate testing.

## 8. Telemetry

Using the existing `tracing` setup:

- `INFO` once per successful `create_item` / `attach_file` / `attach_link`,
  with the resulting key. Matches the existing write-tool log pattern.
- `WARN` if `attach_file(mode=linked_file)` falls back to an absolute path
  because `linked_attachment_base_dir` isn't configured.
- `DEBUG` for per-step upload progression on `imported_file` (authorize →
  s3_put → register) — useful when debugging an interrupted upload.

## 9. Testing Strategy

### 9.1 Unit tests with `wiremock`

Each tool gets unit tests that stand a wiremock server in for
`api.zotero.org`. Coverage:

**`create_item`:**

- Sends a single-item array; receives back the new key and version.
- Item with `collections` field flows through to the request body unchanged.
- `collection_keys` parameter is merged into the item's `collections` array.
- 400 from the server surfaces as `Error::LocalApi { status: 400, body }`.
- Missing API key → `Error::WriteApiKeyMissing` (no network call made).

**`attach_file (mode = "imported_file")`:**

- Three sequential mocks (authorize → s3 PUT → register); test asserts each
  is called in order with the right md5/filename/filesize.
- `"exists": 1` short-circuit path: only steps 5.1a + 5.1b are hit; 5.1c is
  asserted *not* called.
- md5 computed from a fixture file matches what's sent in 5.1b.
- Failure at each stage maps to `UploadFailed { stage: ..., detail: ... }`
  with the right stage label.
- Body validation: PUT body equals `prefix + file_bytes + suffix` (one
  assertion against a captured request).
- Size cap: a file larger than `max_attachment_bytes` returns
  `Error::AttachmentTooLarge` with no network call.

**`attach_file (mode = "linked_file")`:**

- File inside base dir → POST body has `linkMode: linked_file` and
  `path: "attachments:<rel>"`. No upload calls made.
- File outside base dir → `Error::AttachmentOutsideBaseDir`, no network call.
- Base dir unset + absolute path → POST sent, WARN log emitted (asserted via
  `tracing-test`).

**`attach_link`:**

- One POST; body has `linkMode: linked_url`, `url`, no `path`. Returns the
  new attachment key.

### 9.2 Integration test (gated, opt-in)

One end-to-end test against the real Zotero Web API. Gated by env vars
(`ZOTERO_MCP_LIVE_API_KEY`, `ZOTERO_MCP_LIVE_USER_ID`) — none committed.
Behaviour:

1. **Pre-flight:**
   - Assert a `_zotero-mcp-test` collection exists in the user's library.
     Fail with a clear setup message if missing. Pinning the test scope
     to a collection keeps it from polluting real data.
   - Assert `cfg.zotero.api_key` is set with `library:write` permission
     (existing config check).

2. **`create_item`**: post a junk `journalArticle` with title "zotero-mcp
   test", DOI `10.99999/test.<uuid>`, and the test collection key.

3. **Roundtrip check**: immediately call `get_item` on the returned key and
   verify the metadata came back unchanged (title, DOI, collections).
   Catches "wrote something, server accepted it, but it wasn't what we
   meant" bugs.

4. **`attach_file (imported_file)`**: attach `tests/fixtures/hello.pdf`.
   Then `list_attachments` and verify the attachment is registered with
   the expected `linkMode` and `filename`.

5. **`attach_file (linked_file)` round-trip**: write a fixture into a temp
   directory configured as a base dir, attach via `linked_file`, then call
   `list_attachments` and verify the `path` came back with the
   `attachments:<rel>` form. Confirms the encoding works on a real Zotero
   install — not just our wiremock view of it.

6. **`attach_link`**: attach `https://example.com/test` to the parent.
   Verify via `list_attachments`.

7. **Teardown**: `delete_item` on the parent (Zotero auto-trashes children).
   Confirm via `get_item` that the parent is gone.

Test runs in under 10 s, makes 8-12 Zotero API calls, leaves the library
exactly as it started.

### 9.3 CI

No `.github/workflows/` exists in the repo. The integration test is
opt-in via env var and runs locally on demand. Documented in README.

## 10. Definition of Done

This work is not done — and shall not be merged — until **all** of the
following are true:

1. All unit tests pass (`cargo test -p zotero-mcp`).
2. Both attachment modes have passing wiremock tests.
3. **The live integration test (§9.2) has been run against the user's real
   Zotero library and passed.** Not "the test exists." Run. End to end.
4. **Visual verification in Zotero UI:** after the integration test runs and
   before teardown, the user opens Zotero, navigates to the
   `_zotero-mcp-test` collection, and confirms the created item +
   attachment + link are visible and correct. The prior PDF-fallback ship
   had stub tests pass while a real-world bug lurked; this gate exists
   specifically to break that pattern.
5. README updated with the new tools, `attachment_mode` / base-dir config,
   and the integration-test env-var setup instructions.
6. All three MCP tool descriptions clearly state which Zotero attachment
   mode they produce (so AI agents inspecting the tool list don't invent
   manual workarounds, as happened with `get_pdf_text` pre-clarification).

## 11. Files Touched (estimated)

- `crates/zotero-mcp/src/core/error.rs` — four new variants.
- `crates/zotero-mcp/src/core/config.rs` — three new `[zotero]` knobs.
- `crates/zotero-mcp/src/core/writer/items.rs` — `create_item` function.
- `crates/zotero-mcp/src/core/writer/attachments.rs` (new) — `attach_file`,
  `attach_link`, the md5 helper, the three-step upload state machine.
- `crates/zotero-mcp/src/core/writer/mod.rs` — module wire-up.
- `crates/zotero-mcp/src/tools/attachments.rs` — three new tool entry points.
- `crates/zotero-mcp/src/server.rs` — `#[tool]` declarations and descriptions.
- `crates/zotero-mcp/Cargo.toml` — possibly `md-5` or reuse `sha2`'s sibling.
- `crates/zotero-mcp/tests/writer_create_item.rs` (new).
- `crates/zotero-mcp/tests/writer_attach_file.rs` (new).
- `crates/zotero-mcp/tests/writer_attach_link.rs` (new).
- `crates/zotero-mcp/tests/writer_live_zotero.rs` (new, gated).
- `README.md` — new tools, config knobs, integration-test instructions.

## 12. Out-of-Scope / Future Work

- `extract_pdf_identifiers(file_path)` helper — regex DOIs/ISBNs/arXiv IDs
  out of first-page text. Convenience over composing `get_pdf_first_pages`
  + Claude. Add if it earns its keep.
- `imported_url` attachment mode — Zotero-managed URL-to-file fetch.
  Mostly a Web Clipper concern.
- Bulk creation (`create_items` plural) — Zotero's API supports batches of
  up to 50 items per POST. Defer until a real use case demands it.
- File-move semantics for `linked_file` (auto-move into base dir). Explicit
  rejection here keeps behaviour predictable.
