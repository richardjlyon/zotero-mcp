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

    // `ZOTERO_MCP_TEST_KEEP` lets the operator verify the item out-of-band
    // (e.g. via a separate Zotero desktop session) and run teardown manually
    // afterwards. Useful when stdin isn't a TTY and the pause prompt above
    // can't actually wait.
    if env::var("ZOTERO_MCP_TEST_KEEP").is_ok() {
        println!(
            "ZOTERO_MCP_TEST_KEEP set; skipping teardown. To delete manually:\n  \
            curl -X DELETE -H 'Authorization: Bearer $ZOTERO_MCP_LIVE_API_KEY' \
            -H 'Zotero-API-Version: 3' -H 'If-Unmodified-Since-Version: 99999999' \
            https://api.zotero.org/users/{user_id}/items/{parent_key}"
        );
        return;
    }

    // Step 5: Teardown — delete the parent. Children auto-trash with it.
    // Zotero requires an `If-Unmodified-Since-Version` header on DELETE
    // (otherwise 428 Precondition Required). The parent's version has been
    // bumped by child creation, so use a sentinel that always passes — this
    // is teardown for a known-disposable item, not real concurrency control.
    use reqwest::Method;
    let resp = api
        .write_request(Method::DELETE, &format!("/items/{parent_key}"))
        .unwrap()
        .header("If-Unmodified-Since-Version", "99999999")
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success() || resp.status() == 404,
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

    // Teardown. See the create_item_attach_file test for why we use a sentinel
    // If-Unmodified-Since-Version on DELETE.
    use reqwest::Method;
    let del = api
        .write_request(Method::DELETE, &format!("/items/{parent_key}"))
        .unwrap()
        .header("If-Unmodified-Since-Version", "99999999")
        .send()
        .await
        .unwrap();
    assert!(
        del.status().is_success() || del.status() == 404,
        "linked-file teardown delete failed: {}",
        del.status()
    );
}
