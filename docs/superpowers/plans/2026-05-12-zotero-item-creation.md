# Zotero Item Creation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add three new MCP write primitives — `create_item`, `attach_file` (supporting both `imported_file` and `linked_file` Zotero attachment modes), and `attach_link` — so Claude can build a Zotero library from scratch via the MCP.

**Architecture:** Reuse the existing `LocalApi` writer client. New write functions live in `core::writer::items` (for `create_item`) and a new `core::writer::attachments` module (for `attach_file` and `attach_link`). Three new MCP tool wrappers in `tools/attachments.rs` + three `#[tool]` declarations in `server.rs`. Config knobs gate `attachment_mode` and the `linked_attachment_base_dir`. The integration test against a real Zotero library is a non-optional pre-merge gate.

**Tech Stack:** Rust, `reqwest` (HTTP), `tokio`, `wiremock` (mock-server tests), `md-5` (new dep — file hashing for Zotero upload protocol), `mime_guess` (new dep — content-type from extension).

**Spec:** `docs/superpowers/specs/2026-05-12-zotero-item-creation-design.md`

---

## File Structure

| File | Purpose | Action |
|---|---|---|
| `crates/zotero-mcp/Cargo.toml` | Add `md-5`, `mime_guess` | Modify |
| `crates/zotero-mcp/src/core/config.rs` | Add `attachment_mode`, `linked_attachment_base_dir`, `max_attachment_bytes` to `ZoteroConfig` | Modify |
| `crates/zotero-mcp/src/core/error.rs` | Add 4 new error variants | Modify |
| `crates/zotero-mcp/src/core/writer/items.rs` | Add `create_item` function | Modify |
| `crates/zotero-mcp/src/core/writer/attachments.rs` | New module: `attach_file`, `attach_link`, md5 helper, 3-step upload state machine | Create |
| `crates/zotero-mcp/src/core/writer/mod.rs` | Add `pub mod attachments` | Modify |
| `crates/zotero-mcp/src/core/enrichment/mod.rs` | Add `normalized_to_item` helper | Modify |
| `crates/zotero-mcp/src/tools/attachments.rs` | Three new MCP tool wrappers | Modify |
| `crates/zotero-mcp/src/server.rs` | Three new `#[tool]` declarations | Modify |
| `crates/zotero-mcp/tests/writer_create_item.rs` | Wiremock unit tests for `create_item` | Create |
| `crates/zotero-mcp/tests/writer_attach_link.rs` | Wiremock unit tests for `attach_link` | Create |
| `crates/zotero-mcp/tests/writer_attach_file.rs` | Wiremock unit tests for `attach_file` both modes | Create |
| `crates/zotero-mcp/tests/writer_live_zotero.rs` | Gated end-to-end integration test against real Zotero | Create |
| `README.md` | Document new tools, config, integration-test env vars | Modify |

---

## Task 1: Add config knobs to `ZoteroConfig`

**Files:**
- Modify: `crates/zotero-mcp/src/core/config.rs`

- [ ] **Step 1: Write failing tests**

In `crates/zotero-mcp/src/core/config.rs`, append to the existing `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn attachment_mode_defaults_to_imported_file() {
        let c = Config::default();
        assert_eq!(c.zotero.attachment_mode, "imported_file");
        assert!(c.zotero.linked_attachment_base_dir.is_none());
        assert_eq!(c.zotero.max_attachment_bytes, 50 * 1024 * 1024);
    }

    #[test]
    fn attachment_mode_parses_from_toml() {
        let toml = r#"
[zotero]
attachment_mode = "linked_file"
linked_attachment_base_dir = "/Users/rjl/Resilio/Zotero-Attachments"
max_attachment_bytes = 104857600
"#;
        let c: Config = toml::from_str(toml).unwrap();
        assert_eq!(c.zotero.attachment_mode, "linked_file");
        assert_eq!(
            c.zotero.linked_attachment_base_dir.as_deref(),
            Some("/Users/rjl/Resilio/Zotero-Attachments")
        );
        assert_eq!(c.zotero.max_attachment_bytes, 104857600);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p zotero-mcp --lib core::config::tests::attachment_mode -- --nocapture`

Expected: FAIL with "no field `attachment_mode` on type `ZoteroConfig`" (compile error).

- [ ] **Step 3: Add fields to `ZoteroConfig`**

In `core/config.rs`, find the `ZoteroConfig` struct (around lines 27-55). Add three fields before the closing brace, after `pdftotext_fallback`:

```rust
    pub pdftotext_fallback: bool,

    /// Storage model for attachments created via `attach_file`. Default
    /// mirrors Zotero's own default behaviour. Set to `"linked_file"` for
    /// BYO-storage users (Resilio Sync, Syncthing, NAS-backed Zotero data dirs).
    #[serde(default = "default_attachment_mode")]
    pub attachment_mode: String,

    /// Required when `attachment_mode = "linked_file"`. Absolute path to the
    /// Zotero "Linked Attachment Base Directory" (Zotero Preferences →
    /// Advanced → Files & Folders). Files attached via `attach_file` must
    /// live inside this directory.
    #[serde(default)]
    pub linked_attachment_base_dir: Option<String>,

    /// Per-file size ceiling for `attach_file`. Anything larger is rejected
    /// pre-flight. Default: 50 MB.
    #[serde(default = "default_max_attachment_bytes")]
    pub max_attachment_bytes: usize,
}
```

Add the two default helpers above the `impl Default for ZoteroConfig` block (next to the existing `default_true`):

```rust
fn default_attachment_mode() -> String {
    "imported_file".into()
}

fn default_max_attachment_bytes() -> usize {
    50 * 1024 * 1024
}
```

- [ ] **Step 4: Update `Default for ZoteroConfig`**

In the same file, find the `Default` impl for `ZoteroConfig` and add the new fields at the end of the struct literal:

```rust
            pdftotext_path: None,
            pdftotext_fallback: true,
            attachment_mode: "imported_file".into(),
            linked_attachment_base_dir: None,
            max_attachment_bytes: 50 * 1024 * 1024,
        }
```

- [ ] **Step 5: Run tests to verify pass**

Run: `cargo test -p zotero-mcp --lib core::config::tests -- --nocapture`

Expected: PASS (all config tests, including the new ones).

- [ ] **Step 6: Commit**

```bash
git add crates/zotero-mcp/src/core/config.rs
git commit -m "feat(config): add attachment_mode, linked_attachment_base_dir, max_attachment_bytes

Config surface for the upcoming attach_file tool, supporting both Zotero
attachment modes (imported_file default; linked_file for BYO-storage users)."
```

---

## Task 2: Add error variants

**Files:**
- Modify: `crates/zotero-mcp/src/core/error.rs`

- [ ] **Step 1: Write failing tests**

In `crates/zotero-mcp/src/core/error.rs`, append to the `#[cfg(test)] mod tests` block:

```rust
    use std::path::PathBuf;

    #[test]
    fn attachment_file_not_found_message_includes_path() {
        let e = Error::AttachmentFileNotFound(PathBuf::from("/tmp/missing.pdf"));
        let s = e.to_string();
        assert!(s.contains("/tmp/missing.pdf"));
    }

    #[test]
    fn attachment_outside_base_dir_message_includes_hint() {
        let e = Error::AttachmentOutsideBaseDir {
            file_path: PathBuf::from("/var/tmp/x.pdf"),
            base_dir: PathBuf::from("/Users/rjl/Resilio/Zotero-Attachments"),
        };
        let s = e.to_string();
        assert!(s.contains("/var/tmp/x.pdf"));
        assert!(s.contains("/Users/rjl/Resilio/Zotero-Attachments"));
        assert!(s.contains("imported_file"));
    }

    #[test]
    fn upload_failed_carries_stage_and_detail() {
        let e = Error::UploadFailed { stage: "s3_put", detail: "connection reset".into() };
        let s = e.to_string();
        assert!(s.contains("s3_put"));
        assert!(s.contains("connection reset"));
    }

    #[test]
    fn attachment_too_large_includes_limit() {
        let e = Error::AttachmentTooLarge {
            file_path: PathBuf::from("/tmp/big.pdf"),
            limit: 50 * 1024 * 1024,
        };
        let s = e.to_string();
        assert!(s.contains("/tmp/big.pdf"));
        assert!(s.contains(&(50 * 1024 * 1024).to_string()));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p zotero-mcp --lib core::error::tests::attachment -- --nocapture`

Expected: FAIL — variants don't exist yet.

- [ ] **Step 3: Add the variants**

In `core/error.rs`, find the existing `Error` enum. Add these variants immediately after `Pdf(String)` (or anywhere in the enum body — order is cosmetic):

```rust
    #[error("attachment file not found: {0}")]
    AttachmentFileNotFound(std::path::PathBuf),

    #[error(
        "attachment file {file_path} is not inside the configured \
         linked_attachment_base_dir ({base_dir}). Move it in first, or pass \
         mode = \"imported_file\" for this call.",
        file_path = file_path.display(),
        base_dir = base_dir.display(),
    )]
    AttachmentOutsideBaseDir {
        file_path: std::path::PathBuf,
        base_dir: std::path::PathBuf,
    },

    #[error("zotero file upload failed at {stage}: {detail}")]
    UploadFailed { stage: &'static str, detail: String },

    #[error(
        "attachment file {file_path} exceeds max_attachment_bytes ({limit})",
        file_path = file_path.display(),
    )]
    AttachmentTooLarge {
        file_path: std::path::PathBuf,
        limit: usize,
    },
```

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p zotero-mcp --lib core::error::tests`

Expected: PASS for all tests, new and existing.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/core/error.rs
git commit -m "feat(error): add AttachmentFileNotFound, AttachmentOutsideBaseDir, UploadFailed, AttachmentTooLarge

New variants for the upcoming attach_file write tool. UploadFailed carries
a stage label so callers can tell which step of the 3-step Zotero upload
protocol tripped."
```

---

## Task 3: Add `md-5` and `mime_guess` dependencies

**Files:**
- Modify: `crates/zotero-mcp/Cargo.toml`

- [ ] **Step 1: Add dependencies**

In `crates/zotero-mcp/Cargo.toml`, in the `[dependencies]` section, add (alphabetical placement):

```toml
md-5 = "0.10"
mime_guess = "2"
```

Place `md-5` near the existing `sha2 = "0.10"` (they're siblings in the RustCrypto family) and `mime_guess` after.

- [ ] **Step 2: Verify build**

Run: `cargo build -p zotero-mcp 2>&1 | tail -10`

Expected: builds; both crates downloaded and compiled.

- [ ] **Step 3: Commit**

```bash
git add crates/zotero-mcp/Cargo.toml Cargo.lock
git commit -m "build: add md-5 and mime_guess for attach_file upload protocol

md-5 is required by Zotero's file-upload protocol (md5 sent in the
authorize step). mime_guess maps filename extensions to content types
for the attachment metadata."
```

---

## Task 4: Implement `create_item`

**Files:**
- Modify: `crates/zotero-mcp/src/core/writer/items.rs`
- Modify: `crates/zotero-mcp/src/core/enrichment/mod.rs`
- Create: `crates/zotero-mcp/tests/writer_create_item.rs`

- [ ] **Step 1: Write failing unit tests**

Create `crates/zotero-mcp/tests/writer_create_item.rs`:

```rust
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::error::Error;
use zotero_mcp::core::writer::client::LocalApi;
use zotero_mcp::core::writer::items::create_item;

fn api(server_uri: &str) -> LocalApi {
    LocalApi::new("http://unused", 93338)
        .unwrap()
        .with_web_base(server_uri)
        .with_api_key("test-key")
}

#[tokio::test]
async fn creates_item_and_returns_key_and_version() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(header("Zotero-API-Version", "3"))
        .and(body_partial_json(json!([{
            "itemType": "journalArticle",
            "title": "Test paper"
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "NEWK0001", "version": 42 } }
        })))
        .mount(&server)
        .await;

    let item = json!({
        "itemType": "journalArticle",
        "title": "Test paper"
    });
    let (key, version) = create_item(&api(&server.uri()), &item, &[]).await.unwrap();
    assert_eq!(key, "NEWK0001");
    assert_eq!(version, 42);
}

#[tokio::test]
async fn merges_collection_keys_into_item() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(body_partial_json(json!([{
            "itemType": "book",
            "collections": ["COLL0001", "COLL0002"]
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "BOOKK001", "version": 1 } }
        })))
        .mount(&server)
        .await;

    let item = json!({ "itemType": "book", "title": "x" });
    let _ = create_item(
        &api(&server.uri()),
        &item,
        &["COLL0001".into(), "COLL0002".into()],
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn surfaces_zotero_400_as_localapi_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad itemType"))
        .mount(&server)
        .await;

    let item = json!({ "itemType": "nonsense" });
    let err = create_item(&api(&server.uri()), &item, &[]).await.unwrap_err();
    match err {
        Error::LocalApi { status, body } => {
            assert_eq!(status, 400);
            assert!(body.contains("bad itemType"));
        }
        other => panic!("expected LocalApi(400), got {:?}", other),
    }
}

#[tokio::test]
async fn missing_api_key_returns_write_api_key_missing() {
    // No web base / api key configured — write_request errors before any send.
    let api = LocalApi::new("http://unused", 93338).unwrap();
    let item = json!({ "itemType": "journalArticle" });
    let err = create_item(&api, &item, &[]).await.unwrap_err();
    assert!(matches!(err, Error::WriteApiKeyMissing));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p zotero-mcp --test writer_create_item 2>&1 | tail -20`

Expected: FAIL — `create_item` function doesn't exist yet.

- [ ] **Step 3: Implement `create_item`**

In `crates/zotero-mcp/src/core/writer/items.rs`, append:

```rust
/// Create a new Zotero item.
///
/// `item` is a Zotero-shaped JSON object: must have `itemType`; everything
/// else optional and pass-through. `collection_keys` are merged into the
/// item's `collections` field on creation (caller may also set `collections`
/// directly on `item`; both are unioned).
///
/// Returns `(item_key, version)` on success. Errors map to:
/// - `Error::WriteApiKeyMissing` if no api_key configured.
/// - `Error::LocalApi { status, body }` for any 4xx/5xx from Zotero.
pub async fn create_item(
    api: &LocalApi,
    item: &Value,
    collection_keys: &[String],
) -> Result<(String, i64)> {
    // Merge collection_keys into the item (unioned with any existing field).
    let mut item_obj = item
        .as_object()
        .ok_or_else(|| Error::LocalApi {
            status: 0,
            body: "create_item: item must be a JSON object".into(),
        })?
        .clone();

    if !collection_keys.is_empty() {
        let mut existing: Vec<String> = item_obj
            .get("collections")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        for k in collection_keys {
            if !existing.contains(k) {
                existing.push(k.clone());
            }
        }
        item_obj.insert("collections".into(), json!(existing));
    }

    let body = json!([Value::Object(item_obj)]);
    let resp = api
        .write_request(Method::POST, "/items")?
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    let v: Value = resp.json().await?;
    if !status.is_success() {
        return Err(Error::LocalApi {
            status: status.as_u16(),
            body: v.to_string(),
        });
    }
    let entry = v
        .get("successful")
        .and_then(|s| s.get("0"))
        .ok_or_else(|| Error::LocalApi {
            status: status.as_u16(),
            body: v.to_string(),
        })?;
    let key = entry
        .get("key")
        .and_then(|k| k.as_str())
        .ok_or_else(|| Error::LocalApi {
            status: status.as_u16(),
            body: v.to_string(),
        })?
        .to_string();
    let version = entry
        .get("version")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    Ok((key, version))
}
```

The existing imports at the top of `items.rs` need `json` — add it if not present:

```rust
use serde_json::{json, Value};
```

(The current file imports `Value`; add `json` to the same line.)

- [ ] **Step 4: Add `normalized_to_item` helper**

In `crates/zotero-mcp/src/core/enrichment/mod.rs`, append below the existing `openlibrary_like_split` function:

```rust
/// Flatten a `NormalizedRecord` into a Zotero-shaped item JSON suitable for
/// `core::writer::items::create_item`. Caller supplies `item_type` because
/// enrichment sources don't always identify it.
pub fn normalized_to_item(record: &NormalizedRecord, item_type: &str) -> Value {
    let mut obj = record.fields.clone();
    obj.insert("itemType".into(), Value::String(item_type.into()));
    let creators: Vec<Value> = record
        .creators
        .iter()
        .map(|c| serde_json::to_value(c).unwrap_or(Value::Null))
        .filter(|v| !v.is_null())
        .collect();
    if !creators.is_empty() {
        obj.insert("creators".into(), Value::Array(creators));
    }
    Value::Object(obj)
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p zotero-mcp --test writer_create_item 2>&1 | tail -20`

Expected: PASS — all 4 tests.

Also run the whole suite to confirm no regressions:

Run: `cargo test -p zotero-mcp 2>&1 | grep "test result"`

Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/zotero-mcp/src/core/writer/items.rs \
        crates/zotero-mcp/src/core/enrichment/mod.rs \
        crates/zotero-mcp/tests/writer_create_item.rs
git commit -m "feat(writer): add create_item + normalized_to_item helper

create_item posts a single Zotero-shaped JSON object to /items via the
Web API, merges optional collection_keys into the body, and returns
(item_key, version). normalized_to_item flattens an enrichment
NormalizedRecord into create_item input shape."
```

---

## Task 5: Wire `create_item` into the MCP tool layer

**Files:**
- Modify: `crates/zotero-mcp/src/tools/attachments.rs`
- Modify: `crates/zotero-mcp/src/server.rs`

- [ ] **Step 1: Add the tool wrapper in `tools/attachments.rs`**

In `crates/zotero-mcp/src/tools/attachments.rs`, add at the bottom of the file:

```rust
use crate::core::writer::items::create_item;
use serde_json::Value;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateItemArgs {
    /// Zotero-shaped item JSON. Required: itemType (string). Everything else
    /// pass-through to the Zotero Web API.
    pub item: Value,
    /// Optional collection keys to file the new item under. Equivalent to
    /// setting `collections` inside `item`; the two are unioned.
    #[serde(default)]
    pub collection_keys: Vec<String>,
}

pub async fn create_item_t(s: &AppState, a: CreateItemArgs) -> Result<CallToolResult, Error> {
    let (key, version) = create_item(&s.api, &a.item, &a.collection_keys)
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::json!({
        "item_key": key,
        "version": version,
    }))?]))
}
```

The `Deserialize`, `Serialize`, `JsonSchema` imports are already present from existing structs in this file.

- [ ] **Step 2: Add the `#[tool]` declaration in `server.rs`**

In `crates/zotero-mcp/src/server.rs`, find the existing block of write tools (around `add_note`, `update_item_fields`). Add this declaration nearby:

```rust
    #[tool(description = "Create a new Zotero item. Input: { item: <Zotero-shaped JSON object with required itemType field>, collection_keys?: [string] }. Returns { item_key, version }. Tags are an array of objects: [{\"tag\": \"x\"}]. Creators use Zotero's creatorType vocabulary (author/editor/translator/etc). For metadata-discovery flows, lookup_doi / search_crossref return the JSON shape directly compatible with this tool.")]
    pub async fn create_item(
        &self,
        #[tool(aggr)] args: att::CreateItemArgs,
    ) -> Result<CallToolResult, McpError> {
        att::create_item_t(&self.state, args).await
    }
```

(Find the right namespace prefix — the existing tools call `att::add_note_t` etc., so the alias `att` for `tools::attachments` is already imported. Confirm by reading the top of `server.rs`.)

- [ ] **Step 3: Build + test**

Run: `cargo build -p zotero-mcp 2>&1 | tail -5`

Expected: clean.

Run: `cargo test -p zotero-mcp 2>&1 | grep "test result"`

Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-mcp/src/tools/attachments.rs \
        crates/zotero-mcp/src/server.rs
git commit -m "feat(mcp): expose create_item as an MCP tool

Tool description documents the Zotero-shaped input contract: required
itemType, tags as object array, creators with creatorType. Compatible
with the JSON shape returned by lookup_doi and search_crossref."
```

---

## Task 6: Implement `attach_link`

**Files:**
- Create: `crates/zotero-mcp/src/core/writer/attachments.rs`
- Modify: `crates/zotero-mcp/src/core/writer/mod.rs`
- Create: `crates/zotero-mcp/tests/writer_attach_link.rs`

- [ ] **Step 1: Write failing unit test**

Create `crates/zotero-mcp/tests/writer_attach_link.rs`:

```rust
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::writer::attachments::attach_link;
use zotero_mcp::core::writer::client::LocalApi;

fn api(server_uri: &str) -> LocalApi {
    LocalApi::new("http://unused", 93338)
        .unwrap()
        .with_web_base(server_uri)
        .with_api_key("test-key")
}

#[tokio::test]
async fn attach_link_posts_linked_url_attachment_and_returns_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(header("Zotero-API-Version", "3"))
        .and(body_partial_json(json!([{
            "itemType": "attachment",
            "parentItem": "PARENT01",
            "linkMode": "linked_url",
            "url": "https://example.com/test",
            "title": "Example page"
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "LINKK001", "version": 7 } }
        })))
        .mount(&server)
        .await;

    let key = attach_link(
        &api(&server.uri()),
        "PARENT01",
        "https://example.com/test",
        Some("Example page"),
    )
    .await
    .unwrap();
    assert_eq!(key, "LINKK001");
}

#[tokio::test]
async fn attach_link_uses_url_as_title_when_omitted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(body_partial_json(json!([{
            "linkMode": "linked_url",
            "url": "https://example.com/page",
            "title": "https://example.com/page"
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "K", "version": 1 } }
        })))
        .mount(&server)
        .await;

    let _ = attach_link(&api(&server.uri()), "PARENT01", "https://example.com/page", None)
        .await
        .unwrap();
}
```

- [ ] **Step 2: Run test to verify fail**

Run: `cargo test -p zotero-mcp --test writer_attach_link 2>&1 | tail -10`

Expected: FAIL — `attachments` module doesn't exist.

- [ ] **Step 3: Create the attachments module**

Create `crates/zotero-mcp/src/core/writer/attachments.rs`:

```rust
//! Attachment-creation primitives.
//!
//! - [`attach_link`]: single POST that creates a `linked_url` child attachment
//!   (URL only, no bytes).
//! - [`attach_file`]: file-on-disk attachment, supporting both `imported_file`
//!   (3-step upload to Zotero's cloud) and `linked_file` (path reference only).

use crate::core::error::{Error, Result};
use crate::core::writer::client::LocalApi;
use reqwest::Method;
use serde_json::{json, Value};

/// Attach a URL as a `linked_url` child to an existing parent item.
///
/// One POST; no bytes transfer. Returns the new attachment item key.
pub async fn attach_link(
    api: &LocalApi,
    parent_key: &str,
    url: &str,
    title: Option<&str>,
) -> Result<String> {
    let title = title.unwrap_or(url);
    let body = json!([{
        "itemType": "attachment",
        "parentItem": parent_key,
        "linkMode": "linked_url",
        "url": url,
        "title": title,
        "tags": [],
        "relations": {}
    }]);
    let resp = api
        .write_request(Method::POST, "/items")?
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    let v: Value = resp.json().await?;
    if !status.is_success() {
        return Err(Error::LocalApi {
            status: status.as_u16(),
            body: v.to_string(),
        });
    }
    v.get("successful")
        .and_then(|s| s.get("0"))
        .and_then(|i| i.get("key"))
        .and_then(|k| k.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::LocalApi {
            status: status.as_u16(),
            body: v.to_string(),
        })
}
```

- [ ] **Step 4: Wire the module**

In `crates/zotero-mcp/src/core/writer/mod.rs`, append:

```rust
pub mod attachments;
```

- [ ] **Step 5: Run test to verify pass**

Run: `cargo test -p zotero-mcp --test writer_attach_link 2>&1 | tail -10`

Expected: PASS — both tests.

- [ ] **Step 6: Commit**

```bash
git add crates/zotero-mcp/src/core/writer/attachments.rs \
        crates/zotero-mcp/src/core/writer/mod.rs \
        crates/zotero-mcp/tests/writer_attach_link.rs
git commit -m "feat(writer): add attach_link for linked_url attachments

Single POST; creates a linked_url child attachment carrying just the URL.
No bytes transfer. Returns the new attachment key."
```

---

## Task 7: Wire `attach_link` into the MCP tool layer

**Files:**
- Modify: `crates/zotero-mcp/src/tools/attachments.rs`
- Modify: `crates/zotero-mcp/src/server.rs`

- [ ] **Step 1: Add tool wrapper**

In `crates/zotero-mcp/src/tools/attachments.rs`, add:

```rust
use crate::core::writer::attachments::attach_link;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AttachLinkArgs {
    pub parent_key: String,
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
}

pub async fn attach_link_t(s: &AppState, a: AttachLinkArgs) -> Result<CallToolResult, Error> {
    let key = attach_link(&s.api, &a.parent_key, &a.url, a.title.as_deref())
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::json!({
        "attachment_key": key,
    }))?]))
}
```

- [ ] **Step 2: Add `#[tool]` declaration**

In `crates/zotero-mcp/src/server.rs`, near the `create_item` declaration:

```rust
    #[tool(description = "Attach a URL as a child of a Zotero item (linkMode: linked_url). No bytes transfer; Zotero stores just the URL. Use this for online resources you want listed alongside an item without downloading them. Input: { parent_key, url, title? }. Returns { attachment_key }.")]
    pub async fn attach_link(
        &self,
        #[tool(aggr)] args: att::AttachLinkArgs,
    ) -> Result<CallToolResult, McpError> {
        att::attach_link_t(&self.state, args).await
    }
```

- [ ] **Step 3: Build + test**

Run: `cargo build -p zotero-mcp 2>&1 | tail -5 && cargo test -p zotero-mcp 2>&1 | grep "test result"`

Expected: clean build, all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-mcp/src/tools/attachments.rs \
        crates/zotero-mcp/src/server.rs
git commit -m "feat(mcp): expose attach_link as an MCP tool"
```

---

## Task 8: Implement `attach_file` — `linked_file` mode

**Files:**
- Modify: `crates/zotero-mcp/src/core/writer/attachments.rs`
- Create: `crates/zotero-mcp/tests/writer_attach_file.rs`

- [ ] **Step 1: Write failing unit tests for linked_file mode**

Create `crates/zotero-mcp/tests/writer_attach_file.rs`:

```rust
use serde_json::json;
use std::path::PathBuf;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::error::Error;
use zotero_mcp::core::writer::attachments::{attach_file, AttachFileOptions, AttachmentMode};
use zotero_mcp::core::writer::client::LocalApi;

fn api(server_uri: &str) -> LocalApi {
    LocalApi::new("http://unused", 93338)
        .unwrap()
        .with_web_base(server_uri)
        .with_api_key("test-key")
}

fn write_fixture(dir: &std::path::Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

#[tokio::test]
async fn linked_file_inside_base_dir_posts_attachments_prefix_path() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("papers");
    std::fs::create_dir_all(&sub).unwrap();
    let file_path = write_fixture(&sub, "foo.pdf", b"%PDF-1.4\n");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(body_partial_json(json!([{
            "itemType": "attachment",
            "parentItem": "PARENT01",
            "linkMode": "linked_file",
            "path": "attachments:papers/foo.pdf",
            "contentType": "application/pdf"
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "LFK00001", "version": 3 } }
        })))
        .mount(&server)
        .await;

    let opts = AttachFileOptions {
        mode: AttachmentMode::LinkedFile,
        linked_attachment_base_dir: Some(dir.path().to_path_buf()),
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let key = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap();
    assert_eq!(key, "LFK00001");
}

#[tokio::test]
async fn linked_file_outside_base_dir_errors_without_network() {
    let dir = tempfile::tempdir().unwrap();
    let base_dir = dir.path().join("base");
    std::fs::create_dir_all(&base_dir).unwrap();
    let outside = write_fixture(dir.path(), "elsewhere.pdf", b"%PDF-1.4\n");

    // No mocks — if the tool makes a network call, the test fails the assertion below.
    let server = MockServer::start().await;

    let opts = AttachFileOptions {
        mode: AttachmentMode::LinkedFile,
        linked_attachment_base_dir: Some(base_dir.clone()),
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let err = attach_file(&api(&server.uri()), "PARENT01", &outside, &opts)
        .await
        .unwrap_err();
    match err {
        Error::AttachmentOutsideBaseDir { file_path, base_dir: b } => {
            assert_eq!(file_path, outside);
            assert_eq!(b, base_dir);
        }
        other => panic!("expected AttachmentOutsideBaseDir, got {:?}", other),
    }
}

#[tokio::test]
async fn linked_file_without_base_dir_uses_absolute_path() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = write_fixture(dir.path(), "x.pdf", b"%PDF-1.4\n");
    let abs = file_path.to_string_lossy().into_owned();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(body_partial_json(json!([{
            "linkMode": "linked_file",
            "path": abs,
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "ABS00001", "version": 1 } }
        })))
        .mount(&server)
        .await;

    let opts = AttachFileOptions {
        mode: AttachmentMode::LinkedFile,
        linked_attachment_base_dir: None,
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let key = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap();
    assert_eq!(key, "ABS00001");
}

#[tokio::test]
async fn attach_file_returns_not_found_for_missing_path() {
    let server = MockServer::start().await;
    let opts = AttachFileOptions {
        mode: AttachmentMode::LinkedFile,
        linked_attachment_base_dir: None,
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let err = attach_file(
        &api(&server.uri()),
        "PARENT01",
        std::path::Path::new("/nonexistent/path.pdf"),
        &opts,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::AttachmentFileNotFound(_)));
}

#[tokio::test]
async fn attach_file_returns_too_large_when_over_limit() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = write_fixture(dir.path(), "big.pdf", &vec![0u8; 200]);

    let server = MockServer::start().await;
    let opts = AttachFileOptions {
        mode: AttachmentMode::LinkedFile,
        linked_attachment_base_dir: None,
        max_attachment_bytes: 100, // tiny ceiling to force the check
        filename: None,
        content_type: None,
    };
    let err = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap_err();
    match err {
        Error::AttachmentTooLarge { file_path: p, limit } => {
            assert_eq!(p, file_path);
            assert_eq!(limit, 100);
        }
        other => panic!("expected AttachmentTooLarge, got {:?}", other),
    }
}
```

- [ ] **Step 2: Run tests to verify fail**

Run: `cargo test -p zotero-mcp --test writer_attach_file 2>&1 | tail -10`

Expected: FAIL — `attach_file` and types don't exist.

- [ ] **Step 3: Add types and linked_file path implementation**

In `crates/zotero-mcp/src/core/writer/attachments.rs`, append:

```rust
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentMode {
    ImportedFile,
    LinkedFile,
}

impl AttachmentMode {
    /// Parse from the config string. Returns ImportedFile for unknown values
    /// with a warn-level log; this matches the "graceful default" stance of
    /// the rest of the config layer.
    pub fn from_config(s: &str) -> Self {
        match s {
            "linked_file" => AttachmentMode::LinkedFile,
            "imported_file" => AttachmentMode::ImportedFile,
            other => {
                tracing::warn!(
                    value = other,
                    "unknown attachment_mode in config; falling back to imported_file"
                );
                AttachmentMode::ImportedFile
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct AttachFileOptions {
    pub mode: AttachmentMode,
    pub linked_attachment_base_dir: Option<PathBuf>,
    pub max_attachment_bytes: usize,
    pub filename: Option<String>,
    pub content_type: Option<String>,
}

/// Attach a local file to a Zotero parent item.
///
/// `mode` selects between Zotero's `imported_file` (bytes uploaded to
/// Zotero cloud) and `linked_file` (path reference only). Pre-flight
/// validation (file exists, size ≤ max_attachment_bytes, base-dir
/// relativity for linked_file) happens before any network call.
pub async fn attach_file(
    api: &LocalApi,
    parent_key: &str,
    file_path: &Path,
    opts: &AttachFileOptions,
) -> Result<String> {
    // Pre-flight: existence + size cap (cheap, no network).
    let meta = tokio::fs::metadata(file_path)
        .await
        .map_err(|_| Error::AttachmentFileNotFound(file_path.to_path_buf()))?;
    let size = meta.len() as usize;
    if size > opts.max_attachment_bytes {
        return Err(Error::AttachmentTooLarge {
            file_path: file_path.to_path_buf(),
            limit: opts.max_attachment_bytes,
        });
    }

    let filename = opts.filename.clone().unwrap_or_else(|| {
        file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment")
            .to_string()
    });
    let content_type = opts.content_type.clone().unwrap_or_else(|| {
        mime_guess::from_path(file_path)
            .first_or_octet_stream()
            .to_string()
    });

    match opts.mode {
        AttachmentMode::LinkedFile => {
            attach_file_linked(api, parent_key, file_path, &filename, &content_type, opts).await
        }
        AttachmentMode::ImportedFile => {
            // Implemented in Task 9.
            Err(Error::UploadFailed {
                stage: "init",
                detail: "imported_file mode not yet implemented (Task 9)".into(),
            })
        }
    }
}

async fn attach_file_linked(
    api: &LocalApi,
    parent_key: &str,
    file_path: &Path,
    filename: &str,
    content_type: &str,
    opts: &AttachFileOptions,
) -> Result<String> {
    let path_value = match opts.linked_attachment_base_dir.as_ref() {
        Some(base) => {
            let rel = file_path.strip_prefix(base).map_err(|_| {
                Error::AttachmentOutsideBaseDir {
                    file_path: file_path.to_path_buf(),
                    base_dir: base.clone(),
                }
            })?;
            format!("attachments:{}", rel.display())
        }
        None => {
            tracing::warn!(
                file = %file_path.display(),
                "linked_attachment_base_dir not configured; storing absolute path. \
                 File will not replicate to other Zotero clients."
            );
            file_path.display().to_string()
        }
    };

    let body = json!([{
        "itemType": "attachment",
        "parentItem": parent_key,
        "linkMode": "linked_file",
        "title": filename,
        "path": path_value,
        "contentType": content_type,
        "tags": [],
        "relations": {}
    }]);
    let resp = api
        .write_request(Method::POST, "/items")?
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    let v: Value = resp.json().await?;
    if !status.is_success() {
        return Err(Error::LocalApi {
            status: status.as_u16(),
            body: v.to_string(),
        });
    }
    v.get("successful")
        .and_then(|s| s.get("0"))
        .and_then(|i| i.get("key"))
        .and_then(|k| k.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::LocalApi {
            status: status.as_u16(),
            body: v.to_string(),
        })
}
```

Also add `tempfile` to dev-deps if not already there. Check `crates/zotero-mcp/Cargo.toml`:

```toml
[dev-dependencies]
tempfile = "3"
wiremock.workspace = true
```

(Likely already present from earlier tasks — verify before touching.)

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p zotero-mcp --test writer_attach_file 2>&1 | tail -10`

Expected: PASS — all five tests in this file pass. The `ImportedFile` path is stubbed with `Err("not yet implemented")` so any test against that mode would fail; we won't write those tests until Task 9.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/core/writer/attachments.rs \
        crates/zotero-mcp/tests/writer_attach_file.rs
git commit -m "feat(writer): attach_file linked_file mode + pre-flight validation

linked_file mode: one POST, no upload. Computes path relative to
linked_attachment_base_dir when configured, falls back to absolute path
with a WARN log otherwise. Pre-flight checks (file exists, size <=
max_attachment_bytes, base-dir relativity) run before any network call.
imported_file branch stubbed pending Task 9."
```

---

## Task 9: Implement `attach_file` — `imported_file` mode (3-step upload)

**Files:**
- Modify: `crates/zotero-mcp/src/core/writer/attachments.rs`
- Modify: `crates/zotero-mcp/tests/writer_attach_file.rs`

- [ ] **Step 1: Write failing tests for imported_file mode**

Append to `crates/zotero-mcp/tests/writer_attach_file.rs`:

```rust
use wiremock::matchers::{body_string_contains, query_param};

const HELLO_PDF: &[u8] = include_bytes!("fixtures/hello.pdf");

fn write_hello(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("hello.pdf");
    std::fs::write(&p, HELLO_PDF).unwrap();
    p
}

fn md5_hex(bytes: &[u8]) -> String {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut s = String::with_capacity(32);
    for b in digest { s.push_str(&format!("{:02x}", b)); }
    s
}

#[tokio::test]
async fn imported_file_md5_exists_short_circuits_upload() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = write_hello(dir.path());
    let md5 = md5_hex(HELLO_PDF);

    let server = MockServer::start().await;

    // Step 5.1a: create attachment item.
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(body_partial_json(json!([{
            "itemType": "attachment",
            "parentItem": "PARENT01",
            "linkMode": "imported_file",
            "filename": "hello.pdf",
            "contentType": "application/pdf",
            "md5": md5
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "ATT00001", "version": 1 } }
        })))
        .mount(&server)
        .await;

    // Step 5.1b: authorize -> exists:1 short-circuit.
    Mock::given(method("POST"))
        .and(path("/users/93338/items/ATT00001/file"))
        .and(header("If-None-Match", "*"))
        .and(body_string_contains(format!("md5={md5}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "exists": 1 })))
        .mount(&server)
        .await;

    // No step-5.1c PUT/register mocks — they must not be invoked.

    let opts = AttachFileOptions {
        mode: AttachmentMode::ImportedFile,
        linked_attachment_base_dir: None,
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let key = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap();
    assert_eq!(key, "ATT00001");
}

#[tokio::test]
async fn imported_file_full_three_step_upload_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = write_hello(dir.path());
    let md5 = md5_hex(HELLO_PDF);

    let server = MockServer::start().await;
    let s3 = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "ATT00002", "version": 1 } }
        })))
        .mount(&server)
        .await;

    // Step 5.1b: authorize -> returns upload URL pointing at the s3 mock.
    let upload_url = format!("{}/upload", s3.uri());
    Mock::given(method("POST"))
        .and(path("/users/93338/items/ATT00002/file"))
        .and(body_string_contains(format!("md5={md5}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "url": upload_url,
            "contentType": "application/octet-stream",
            "prefix": "PFX",
            "suffix": "SFX",
            "uploadKey": "UPLOADKEY"
        })))
        .mount(&server)
        .await;

    // Step 5.1c.PUT: receives prefix + file_bytes + suffix.
    let mut expected_body = b"PFX".to_vec();
    expected_body.extend_from_slice(HELLO_PDF);
    expected_body.extend_from_slice(b"SFX");
    Mock::given(method("PUT"))
        .and(path("/upload"))
        .and(header("Content-Type", "application/octet-stream"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s3)
        .await;

    // Step 5.1c.register: POST with ?upload=<key>.
    Mock::given(method("POST"))
        .and(path("/users/93338/items/ATT00002/file"))
        .and(query_param("upload", "UPLOADKEY"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let opts = AttachFileOptions {
        mode: AttachmentMode::ImportedFile,
        linked_attachment_base_dir: None,
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let key = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap();
    assert_eq!(key, "ATT00002");
}

#[tokio::test]
async fn imported_file_s3_put_failure_maps_to_upload_failed_stage_s3_put() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = write_hello(dir.path());

    let server = MockServer::start().await;
    let s3 = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "ATT00003", "version": 1 } }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items/ATT00003/file"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "url": format!("{}/upload", s3.uri()),
            "contentType": "application/octet-stream",
            "prefix": "",
            "suffix": "",
            "uploadKey": "K"
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/upload"))
        .respond_with(ResponseTemplate::new(500).set_body_string("S3 boom"))
        .mount(&s3)
        .await;

    let opts = AttachFileOptions {
        mode: AttachmentMode::ImportedFile,
        linked_attachment_base_dir: None,
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let err = attach_file(&api(&server.uri()), "PARENT01", &file_path, &opts)
        .await
        .unwrap_err();
    match err {
        Error::UploadFailed { stage, detail } => {
            assert_eq!(stage, "s3_put");
            assert!(detail.contains("500") || detail.contains("S3 boom"));
        }
        other => panic!("expected UploadFailed(stage=s3_put), got {:?}", other),
    }
}
```

The test file's existing imports may need extending. Make sure `md5` is referenced via the `md-5` crate — its crate name in code is `md5`. Add to the top of the test file (if needed):

```rust
extern crate md5;
```

(Newer Rust 2021 doesn't need `extern crate`; `use md5::{Digest, Md5};` inside the helper covers it.)

- [ ] **Step 2: Run tests to verify fail**

Run: `cargo test -p zotero-mcp --test writer_attach_file imported_file 2>&1 | tail -15`

Expected: 3 tests fail (the `imported_file` branch still returns the "not yet implemented" stub).

- [ ] **Step 3: Implement the 3-step upload**

In `crates/zotero-mcp/src/core/writer/attachments.rs`, replace the `AttachmentMode::ImportedFile` arm with the real implementation. First, add the helper function at the top of the file (just below the use statements):

```rust
fn md5_hex(bytes: &[u8]) -> String {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(bytes);
    let d = h.finalize();
    let mut s = String::with_capacity(32);
    for b in d {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
```

Then replace the `AttachmentMode::ImportedFile` arm of `attach_file`:

```rust
        AttachmentMode::ImportedFile => {
            let bytes = tokio::fs::read(file_path).await.map_err(|e| {
                Error::UploadFailed {
                    stage: "read",
                    detail: format!("reading {}: {}", file_path.display(), e),
                }
            })?;
            attach_file_imported(api, parent_key, &bytes, &filename, &content_type).await
        }
```

Add the `attach_file_imported` function after `attach_file_linked`:

```rust
async fn attach_file_imported(
    api: &LocalApi,
    parent_key: &str,
    bytes: &[u8],
    filename: &str,
    content_type: &str,
) -> Result<String> {
    let md5 = md5_hex(bytes);
    let mtime = unix_ms_now();
    let filesize = bytes.len();

    // Step 5.1a: create the attachment row.
    let attach_key = create_imported_attachment_row(
        api, parent_key, filename, content_type, &md5, mtime,
    )
    .await?;

    // Step 5.1b: authorize upload.
    match authorize_upload(api, &attach_key, &md5, filename, filesize, mtime).await? {
        AuthorizeResult::Exists => Ok(attach_key),
        AuthorizeResult::NeedsUpload {
            url,
            content_type: upload_ct,
            prefix,
            suffix,
            upload_key,
        } => {
            // Step 5.1c: PUT bytes to signed URL, then register upload.
            put_to_s3(api, &url, &upload_ct, &prefix, bytes, &suffix).await?;
            register_upload(api, &attach_key, &upload_key).await?;
            Ok(attach_key)
        }
    }
}

async fn create_imported_attachment_row(
    api: &LocalApi,
    parent_key: &str,
    filename: &str,
    content_type: &str,
    md5: &str,
    mtime: u64,
) -> Result<String> {
    let body = json!([{
        "itemType": "attachment",
        "parentItem": parent_key,
        "linkMode": "imported_file",
        "title": filename,
        "filename": filename,
        "contentType": content_type,
        "charset": "",
        "md5": md5,
        "mtime": mtime,
        "tags": [],
        "relations": {}
    }]);
    let resp = api
        .write_request(Method::POST, "/items")?
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    let v: Value = resp.json().await?;
    if !status.is_success() {
        return Err(Error::LocalApi {
            status: status.as_u16(),
            body: v.to_string(),
        });
    }
    v.get("successful")
        .and_then(|s| s.get("0"))
        .and_then(|i| i.get("key"))
        .and_then(|k| k.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::LocalApi {
            status: status.as_u16(),
            body: v.to_string(),
        })
}

enum AuthorizeResult {
    Exists,
    NeedsUpload {
        url: String,
        content_type: String,
        prefix: String,
        suffix: String,
        upload_key: String,
    },
}

async fn authorize_upload(
    api: &LocalApi,
    attach_key: &str,
    md5: &str,
    filename: &str,
    filesize: usize,
    mtime: u64,
) -> Result<AuthorizeResult> {
    let body = format!(
        "md5={md5}&filename={fn_enc}&filesize={size}&mtime={mtime}",
        fn_enc = urlencoding::encode(filename),
        size = filesize,
    );
    let resp = api
        .write_request(Method::POST, &format!("/items/{attach_key}/file"))?
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("If-None-Match", "*")
        .body(body)
        .send()
        .await
        .map_err(|e| Error::UploadFailed {
            stage: "authorize",
            detail: e.to_string(),
        })?;
    let status = resp.status();
    let v: Value = resp.json().await.map_err(|e| Error::UploadFailed {
        stage: "authorize",
        detail: format!("non-JSON response: {}", e),
    })?;
    if !status.is_success() {
        return Err(Error::UploadFailed {
            stage: "authorize",
            detail: format!("{}: {}", status, v),
        });
    }
    if v.get("exists").and_then(|x| x.as_i64()) == Some(1) {
        return Ok(AuthorizeResult::Exists);
    }
    let url = v
        .get("url")
        .and_then(|x| x.as_str())
        .ok_or_else(|| Error::UploadFailed {
            stage: "authorize",
            detail: format!("missing url in response: {}", v),
        })?
        .to_string();
    let content_type = v
        .get("contentType")
        .and_then(|x| x.as_str())
        .unwrap_or("application/octet-stream")
        .to_string();
    let prefix = v
        .get("prefix")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let suffix = v
        .get("suffix")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let upload_key = v
        .get("uploadKey")
        .and_then(|x| x.as_str())
        .ok_or_else(|| Error::UploadFailed {
            stage: "authorize",
            detail: format!("missing uploadKey in response: {}", v),
        })?
        .to_string();
    Ok(AuthorizeResult::NeedsUpload {
        url,
        content_type,
        prefix,
        suffix,
        upload_key,
    })
}

async fn put_to_s3(
    api: &LocalApi,
    url: &str,
    content_type: &str,
    prefix: &str,
    bytes: &[u8],
    suffix: &str,
) -> Result<()> {
    let mut body = Vec::with_capacity(prefix.len() + bytes.len() + suffix.len());
    body.extend_from_slice(prefix.as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(suffix.as_bytes());

    let resp = api
        .http
        .put(url)
        .header("Content-Type", content_type)
        .body(body)
        .send()
        .await
        .map_err(|e| Error::UploadFailed {
            stage: "s3_put",
            detail: e.to_string(),
        })?;
    let status = resp.status();
    if !status.is_success() {
        let detail = resp.text().await.unwrap_or_default();
        return Err(Error::UploadFailed {
            stage: "s3_put",
            detail: format!("{}: {}", status, detail),
        });
    }
    Ok(())
}

async fn register_upload(api: &LocalApi, attach_key: &str, upload_key: &str) -> Result<()> {
    let resp = api
        .write_request(
            Method::POST,
            &format!("/items/{attach_key}/file?upload={upload_key}"),
        )?
        .header("If-None-Match", "*")
        .send()
        .await
        .map_err(|e| Error::UploadFailed {
            stage: "register",
            detail: e.to_string(),
        })?;
    let status = resp.status();
    if !status.is_success() {
        let detail = resp.text().await.unwrap_or_default();
        return Err(Error::UploadFailed {
            stage: "register",
            detail: format!("{}: {}", status, detail),
        });
    }
    Ok(())
}
```

Add `urlencoding` to `Cargo.toml` `[dependencies]`:

```toml
urlencoding = "2"
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p zotero-mcp --test writer_attach_file 2>&1 | tail -20`

Expected: all 8 tests pass (5 from Task 8 + 3 new).

Run full suite: `cargo test -p zotero-mcp 2>&1 | grep "test result"`

Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/core/writer/attachments.rs \
        crates/zotero-mcp/tests/writer_attach_file.rs \
        crates/zotero-mcp/Cargo.toml Cargo.lock
git commit -m "feat(writer): attach_file imported_file mode (3-step upload)

Implements the documented Zotero file-upload protocol: create attachment
row, authorize upload (md5/filename/filesize/mtime), PUT prefix+bytes+suffix
to the returned S3 URL, then register the completed upload. Handles the
exists:1 short-circuit for byte-identical files. UploadFailed::stage labels
each step so callers can tell where a failure happened."
```

---

## Task 10: Wire `attach_file` into the MCP tool layer

**Files:**
- Modify: `crates/zotero-mcp/src/tools/attachments.rs`
- Modify: `crates/zotero-mcp/src/server.rs`

- [ ] **Step 1: Add tool wrapper**

In `crates/zotero-mcp/src/tools/attachments.rs`, add:

```rust
use crate::core::writer::attachments::{attach_file, AttachFileOptions, AttachmentMode};
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AttachFileArgs {
    pub parent_key: String,
    /// Absolute path to a local file.
    pub file_path: String,
    /// Override the config-default attachment mode. "imported_file" uploads
    /// bytes to Zotero cloud storage; "linked_file" stores a path reference
    /// (BYO storage). Omit to use cfg.zotero.attachment_mode.
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
}

pub async fn attach_file_t(s: &AppState, a: AttachFileArgs) -> Result<CallToolResult, Error> {
    let cfg = &s.cfg.zotero;
    let mode_str = a.mode.as_deref().unwrap_or(&cfg.attachment_mode);
    let mode = AttachmentMode::from_config(mode_str);
    let opts = AttachFileOptions {
        mode,
        linked_attachment_base_dir: cfg
            .linked_attachment_base_dir
            .as_deref()
            .map(crate::core::config::expand_tilde)
            .map(PathBuf::from),
        max_attachment_bytes: cfg.max_attachment_bytes,
        filename: a.filename,
        content_type: a.content_type,
    };
    let path = PathBuf::from(crate::core::config::expand_tilde(&a.file_path));
    let key = attach_file(&s.api, &a.parent_key, &path, &opts)
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::json!({
        "attachment_key": key,
    }))?]))
}
```

- [ ] **Step 2: Add `#[tool]` declaration**

In `crates/zotero-mcp/src/server.rs`, near the other attachment declarations:

```rust
    #[tool(description = "Attach a local file to a Zotero parent item. Two storage modes: \"imported_file\" (bytes uploaded to Zotero's cloud and downloaded locally on each device — Zotero's default) or \"linked_file\" (Zotero stores only a path reference; the file lives wherever you put it — useful for BYO-storage setups like Resilio/Syncthing). Default mode comes from cfg.zotero.attachment_mode; per-call override allowed. For linked_file, the file must be under cfg.zotero.linked_attachment_base_dir. Input: { parent_key, file_path (absolute), mode?, filename?, content_type? }. Returns { attachment_key }.")]
    pub async fn attach_file(
        &self,
        #[tool(aggr)] args: att::AttachFileArgs,
    ) -> Result<CallToolResult, McpError> {
        att::attach_file_t(&self.state, args).await
    }
```

- [ ] **Step 3: Build + test**

Run: `cargo build -p zotero-mcp 2>&1 | tail -5 && cargo test -p zotero-mcp 2>&1 | grep "test result"`

Expected: clean build, all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-mcp/src/tools/attachments.rs \
        crates/zotero-mcp/src/server.rs
git commit -m "feat(mcp): expose attach_file as an MCP tool

Reads mode/base-dir/size-cap from cfg.zotero, with per-call mode override.
Tool description documents both storage modes so AI agents inspecting the
tool list see the contract."
```

---

## Task 11: Write the live integration test (gated)

**Files:**
- Create: `crates/zotero-mcp/tests/writer_live_zotero.rs`

- [ ] **Step 1: Create the gated test file**

Create `crates/zotero-mcp/tests/writer_live_zotero.rs`:

```rust
//! End-to-end integration test against the real Zotero Web API.
//!
//! Gated by environment variables — does nothing on machines that don't have
//! them set. To run:
//!
//! ```bash
//! ZOTERO_MCP_LIVE_API_KEY=...   \
//! ZOTERO_MCP_LIVE_USER_ID=...   \
//! ZOTERO_MCP_TEST_COLLECTION_KEY=...   \
//! cargo test -p zotero-mcp --test writer_live_zotero -- --nocapture --ignored
//! ```
//!
//! The test creates a junk journalArticle in the named collection, attaches
//! a tiny PDF (imported_file mode), attaches a URL, verifies via list calls,
//! then deletes the parent (Zotero auto-trashes children).
//!
//! Marked `#[ignore]` so it doesn't run by default. The Definition of Done
//! requires this test to be run manually before merge — see plan Task 13.

use serde_json::json;
use std::env;
use std::path::PathBuf;
use zotero_mcp::core::writer::attachments::{
    attach_file, attach_link, AttachFileOptions, AttachmentMode,
};
use zotero_mcp::core::writer::client::LocalApi;
use zotero_mcp::core::writer::items::create_item;

fn live_env() -> Option<(String, i64, String)> {
    let key = env::var("ZOTERO_MCP_LIVE_API_KEY").ok()?;
    let user_id = env::var("ZOTERO_MCP_LIVE_USER_ID").ok()?.parse().ok()?;
    let collection = env::var("ZOTERO_MCP_TEST_COLLECTION_KEY").ok()?;
    Some((key, user_id, collection))
}

#[tokio::test]
#[ignore]
async fn live_create_item_attach_file_attach_link_roundtrip() {
    let Some((api_key, user_id, collection_key)) = live_env() else {
        eprintln!("LIVE env vars not set; skipping");
        return;
    };

    let api = LocalApi::new("http://localhost:23119", user_id)
        .unwrap()
        .with_api_key(api_key);

    // Step 1: create_item.
    let unique = format!(
        "10.99999/zotero-mcp-test.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    let item = json!({
        "itemType": "journalArticle",
        "title": "zotero-mcp integration test (DELETE ME)",
        "DOI": unique,
        "creators": [{ "creatorType": "author", "firstName": "Integration", "lastName": "Test" }],
        "date": "2026-01-01",
        "tags": [{ "tag": "_zotero-mcp-test" }]
    });
    let (parent_key, _version) =
        create_item(&api, &item, &[collection_key.clone()]).await.unwrap();
    println!("created parent: {parent_key}");

    // Step 2: attach_file (imported_file). Uses the committed hello.pdf fixture.
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.pdf");
    let opts = AttachFileOptions {
        mode: AttachmentMode::ImportedFile,
        linked_attachment_base_dir: None,
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let attach_key = attach_file(&api, &parent_key, &fixture, &opts).await.unwrap();
    println!("attached file: {attach_key}");

    // Step 3: attach_link.
    let link_key = attach_link(
        &api,
        &parent_key,
        "https://example.com/zotero-mcp-test",
        Some("Example test link"),
    )
    .await
    .unwrap();
    println!("attached link: {link_key}");

    // Step 4: Visual verification pause — Definition of Done requires the
    // human to confirm the item + attachment + link are visible in the
    // Zotero UI before teardown.
    if env::var("ZOTERO_MCP_TEST_PAUSE").is_ok() {
        println!("\n>>> Open Zotero, navigate to the test collection, and verify:");
        println!(">>>   - Item: 'zotero-mcp integration test (DELETE ME)'");
        println!(">>>   - Child PDF attachment: hello.pdf");
        println!(">>>   - Child link: Example test link");
        println!(">>> Press ENTER in this terminal to continue with teardown...");
        let mut s = String::new();
        std::io::stdin().read_line(&mut s).unwrap();
    }

    // Step 5: Teardown — delete the parent. Children auto-trash with it.
    use reqwest::Method;
    let resp = api
        .write_request(Method::DELETE, &format!("/items/{parent_key}"))
        .unwrap()
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success() || resp.status() == 412 || resp.status() == 404,
        "delete failed: {}",
        resp.status()
    );
    println!("teardown complete");
}

#[tokio::test]
#[ignore]
async fn live_attach_file_linked_file_roundtrip() {
    let Some((api_key, user_id, collection_key)) = live_env() else {
        eprintln!("LIVE env vars not set; skipping");
        return;
    };

    let api = LocalApi::new("http://localhost:23119", user_id)
        .unwrap()
        .with_api_key(api_key);

    // Use a temp dir as the base dir for this test scope. The path stored in
    // Zotero will be a path local to this machine and won't replicate, which
    // is fine for verifying the encoding mechanism.
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("papers");
    std::fs::create_dir_all(&sub).unwrap();
    let pdf_path = sub.join("linked-test.pdf");
    let hello = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.pdf"),
    )
    .unwrap();
    std::fs::write(&pdf_path, &hello).unwrap();

    // Step 1: parent.
    let unique = format!(
        "10.99999/zotero-mcp-test-linked.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    let item = json!({
        "itemType": "journalArticle",
        "title": "zotero-mcp linked-file test (DELETE ME)",
        "DOI": unique,
        "tags": [{ "tag": "_zotero-mcp-test" }]
    });
    let (parent_key, _) =
        create_item(&api, &item, &[collection_key.clone()]).await.unwrap();

    // Step 2: linked_file attach.
    let opts = AttachFileOptions {
        mode: AttachmentMode::LinkedFile,
        linked_attachment_base_dir: Some(dir.path().to_path_buf()),
        max_attachment_bytes: 50 * 1024 * 1024,
        filename: None,
        content_type: None,
    };
    let attach_key = attach_file(&api, &parent_key, &pdf_path, &opts).await.unwrap();
    println!("linked attachment: {attach_key}");

    // Roundtrip: read the attachment item back via the Web API and verify the
    // path field came back with the "attachments:" prefix.
    let item_json: serde_json::Value = api
        .http
        .get(format!(
            "https://api.zotero.org/users/{}/items/{}",
            user_id, attach_key
        ))
        .header("Zotero-API-Version", "3")
        .header(
            "Authorization",
            format!(
                "Bearer {}",
                env::var("ZOTERO_MCP_LIVE_API_KEY").unwrap()
            ),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let path_val = item_json
        .get("data")
        .and_then(|d| d.get("path"))
        .and_then(|p| p.as_str())
        .unwrap();
    assert!(
        path_val.starts_with("attachments:"),
        "expected attachments: prefix, got {path_val}"
    );
    println!("path roundtrip ok: {path_val}");

    // Teardown.
    use reqwest::Method;
    api.write_request(Method::DELETE, &format!("/items/{parent_key}"))
        .unwrap()
        .send()
        .await
        .unwrap();
}
```

- [ ] **Step 2: Build and verify the test compiles**

Run: `cargo test -p zotero-mcp --test writer_live_zotero --no-run 2>&1 | tail -10`

Expected: clean build of the test binary. The tests themselves don't run (they're `#[ignore]`'d).

- [ ] **Step 3: Commit**

```bash
git add crates/zotero-mcp/tests/writer_live_zotero.rs
git commit -m "test(live): add gated end-to-end integration tests against real Zotero

Two #[ignore]'d tests covering: (1) full create_item + attach_file
imported_file + attach_link roundtrip with optional human-confirmation
pause, and (2) attach_file linked_file mode with roundtrip verification
of the 'attachments:' path encoding. Run with ZOTERO_MCP_LIVE_* env vars
set. Definition of Done requires both to pass before merge."
```

---

## Task 12: README update

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add the new tools to the README**

Read the current README to find the right insertion points. Then add three things:

**(a) Tool list:** Find where existing tools are listed (the README mentions `delete_item` at the top — there should be a tools section). Add three lines:

```markdown
- `create_item` — Create a new Zotero item from a metadata JSON object.
- `attach_file` — Attach a local file to a parent item (supports `imported_file` and `linked_file` modes).
- `attach_link` — Attach a URL to a parent item as a `linked_url` attachment.
```

**(b) Configuration section:** Find the existing config example (the one mentioning `api_key`). Add the new knobs:

```markdown
For attachments:

```toml
[zotero]
# Storage model for files attached via attach_file. "imported_file" (default)
# uploads bytes to Zotero's cloud; "linked_file" stores only a path reference
# (BYO storage, e.g. Resilio Sync, Syncthing).
attachment_mode = "imported_file"

# Required when attachment_mode = "linked_file". Files attached via
# attach_file must live inside this directory.
linked_attachment_base_dir = "/Users/you/Resilio/Zotero-Attachments"

# Per-file size ceiling. Default: 50 MB.
max_attachment_bytes = 52428800
```
```

**(c) Integration-test section:** Add a new section near the bottom (before any "Development" or "Contributing" section, or right at the end):

```markdown
## Integration test against your real Zotero library

The unit tests use mocked HTTP servers and don't touch your library. A
separate gated test exercises the write tools against the real Zotero
Web API end-to-end. Useful when:

- You're about to depend on `create_item` / `attach_file` / `attach_link`
  in a workflow.
- You've upgraded `zotero-mcp` and want to verify writes still work.
- You're contributing changes that touch the write tools.

Setup (one-time):

1. Generate a Zotero Web API key with `library:write` permission at
   <https://www.zotero.org/settings/keys>.
2. In Zotero desktop, create a collection named `_zotero-mcp-test`. The
   test scopes everything to this collection so a failure can't pollute
   real data. Note its key (right-click → "Generate Report" or via the
   Zotero connector — any way to get the 8-char collection key).
3. Find your Zotero user ID at the same Settings page.

Run:

```bash
ZOTERO_MCP_LIVE_API_KEY=<key> \
ZOTERO_MCP_LIVE_USER_ID=<user-id> \
ZOTERO_MCP_TEST_COLLECTION_KEY=<collection-key> \
ZOTERO_MCP_TEST_PAUSE=1 \
cargo test -p zotero-mcp --test writer_live_zotero -- --nocapture --ignored
```

`ZOTERO_MCP_TEST_PAUSE` triggers a manual-verification pause before
teardown — open Zotero, navigate to the test collection, eyeball the
created item and its two children, then press ENTER to let the test
clean up.
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs(readme): document create_item, attach_file, attach_link + integration test setup"
```

---

## Task 13: Run the live integration test (non-optional pre-merge gate)

**Files:** None modified. This task is a gate.

**Per the spec's §10 Definition of Done, this task must complete successfully before the work is considered done.**

- [ ] **Step 1: Pre-flight checks**

```bash
echo "Checking ZOTERO_MCP_LIVE_API_KEY: ${ZOTERO_MCP_LIVE_API_KEY:+set}"
echo "Checking ZOTERO_MCP_LIVE_USER_ID: ${ZOTERO_MCP_LIVE_USER_ID:-unset}"
echo "Checking ZOTERO_MCP_TEST_COLLECTION_KEY: ${ZOTERO_MCP_TEST_COLLECTION_KEY:-unset}"
```

All three must be set. If any is missing, follow the setup in README section "Integration test against your real Zotero library" first, then re-run.

Also verify the `_zotero-mcp-test` collection exists in Zotero (open the app and look in the collections sidebar). If it doesn't, create it first.

- [ ] **Step 2: Run the live integration tests**

```bash
ZOTERO_MCP_TEST_PAUSE=1 cargo test -p zotero-mcp --test writer_live_zotero -- --nocapture --ignored 2>&1 | tail -40
```

Expected behaviour:
1. `live_create_item_attach_file_attach_link_roundtrip` runs:
   - Creates the parent item (you see `created parent: <KEY>`).
   - Attaches `hello.pdf` (you see `attached file: <KEY>`).
   - Attaches a URL (you see `attached link: <KEY>`).
   - **Pauses with the verification prompt.** Do not press ENTER yet.

2. **Visual verification (the part that catches bugs the unit tests miss):**
   - Open Zotero desktop.
   - Navigate to `_zotero-mcp-test` collection.
   - Confirm you see the new item titled "zotero-mcp integration test (DELETE ME)".
   - Expand the item — confirm both children are present: a PDF attachment showing as `hello.pdf` and a link attachment titled "Example test link".
   - Click the PDF — Zotero should be able to open it.
   - Click the link — Zotero should open `https://example.com/zotero-mcp-test`.

3. **Only after visual verification passes:** press ENTER in the terminal. The test tears down (deletes the parent; children cascade).

4. The second test (`live_attach_file_linked_file_roundtrip`) runs next, with no pause. It exercises linked_file mode and asserts the `attachments:` path encoding came back correctly. No visual step needed — the assertion catches encoding bugs.

- [ ] **Step 3: Confirm clean test outcome**

Both tests must show `ok` in the final summary:

```
test result: ok. 2 passed; 0 failed; 0 ignored; ...
```

If either fails, STOP. Diagnose and fix before declaring the work done.

- [ ] **Step 4: Confirm teardown completed**

In Zotero, refresh the `_zotero-mcp-test` collection. The two test items should be gone (or in Trash). Empty the trash if you want.

- [ ] **Step 5: Document the run**

Append a single line to the project's run log (or just announce in the next message):

```
2026-MM-DD: writer_live_zotero — 2/2 pass. Visual UI verification confirmed
            create_item + attach_file (imported_file + linked_file) + attach_link
            against KSALPBV7-test collection. Teardown clean.
```

This task is **complete only when** all of steps 1-4 have passed and the UI verification has been confirmed by a human eye. No "the test passed in CI" substitutes here.

---

## Task 14: Final verification and merge readiness

**Files:** None modified.

- [ ] **Step 1: Full unit-test suite**

```bash
cargo test -p zotero-mcp 2>&1 | grep "test result"
```

Expected: every test group passes. No failures, no skipped tests except the `#[ignore]`'d live tests.

- [ ] **Step 2: Build the release binary**

```bash
cargo build -p zotero-mcp --release 2>&1 | tail -5
```

Expected: clean release build.

- [ ] **Step 3: Confirm Task 13 was actually run**

If Task 13 is not yet checked, stop and do it. The Definition of Done is unambiguous here: live integration test must have run and passed before this task can complete.

- [ ] **Step 4: Final state check**

```bash
git status --short                                          # must be empty
git log --oneline ^main HEAD                                # all task commits, in order
```

Confirm:
- Working tree clean.
- Every task above produced exactly one commit (except Task 13 which produces none).
- Total commit count is 12 commits (Tasks 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12).

- [ ] **Step 5: Ready for merge**

Report:

- All 12 commits landed.
- Unit tests: pass count.
- Live integration tests: 2/2 pass with visual UI verification confirmed.
- Release binary builds clean.
- Ready to merge.

---

## Self-Review Notes (filled at plan-write time)

**Spec coverage:**

- §3 API surface (create_item, attach_file, attach_link signatures) → Tasks 4, 6, 8, 9 (impl); Tasks 5, 7, 10 (MCP wiring).
- §4 Item JSON shape → Task 4 (`create_item` impl + tests verify the shape).
- §5 File upload protocol → Tasks 8 (linked_file mode) and 9 (imported_file 3-step protocol).
- §6 Configuration → Task 1.
- §7 Error model → Task 2.
- §8 Telemetry → Tasks 8 (linked_file WARN log), 9 (DEBUG per step is implicit in the tracing-prone code paths; explicit per-step DEBUG was simplified to just the per-step error paths to avoid log spam). Per-success INFO is left to the tool wrapper (Tasks 5, 7, 10) — if you want it added explicitly, add a `tracing::info!` after the success branch in each `_t` function.
- §9.1 Unit tests → Tasks 4, 6, 8, 9.
- §9.2 Integration test → Tasks 11 (write) and 13 (run).
- §10 Definition of Done → enforced by Tasks 13, 14.
- §11 Files touched → matches the File Structure section at the top of this plan.

**Placeholder scan:** Clean. The §8 telemetry simplification is the only deviation from the spec; flagged inline above so the reviewer can request it back if wanted.

**Type consistency:**
- `AttachmentMode::ImportedFile` / `LinkedFile` consistent across Tasks 8, 9, 10, 11.
- `AttachFileOptions { mode, linked_attachment_base_dir, max_attachment_bytes, filename, content_type }` consistent.
- `Error::UploadFailed { stage: &'static str, detail: String }` consistent across Task 2 (definition) and Tasks 8, 9 (construction).
- `cfg.zotero.attachment_mode` / `cfg.zotero.linked_attachment_base_dir` / `cfg.zotero.max_attachment_bytes` consistent across Tasks 1, 10.
- `create_item(api, &Value, &[String]) -> Result<(String, i64)>` consistent across Tasks 4, 5, 11.
- `attach_link(api, parent_key, url, title?) -> Result<String>` consistent across Tasks 6, 7, 11.
- `attach_file(api, parent_key, file_path, &AttachFileOptions) -> Result<String>` consistent across Tasks 8, 9, 10, 11.

**Optional follow-on (not in this plan):** explicit per-success `tracing::info!` in each `_t` wrapper. Easy to add after merge if the operator wants more telemetry.
