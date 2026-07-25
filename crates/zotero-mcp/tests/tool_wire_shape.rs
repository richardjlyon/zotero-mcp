//! Asserts what tools actually put on the wire.
//!
//! Nothing tested this before, which is why the May 2026 question "would
//! changing the response shape break a client?" was argued rather than
//! measured. These tests pin the shape of all three response families, so any
//! future change to one of them is a visible, deliberate edit to an expectation
//! here.
//!
//! No live Zotero needed: `AppState` builds against the test fixture, with
//! `user_id` pinned so no user-id detection call is attempted.

mod fixtures;

use rmcp::handler::server::tool::IntoCallToolResult;
use rmcp::model::CallToolResult;
use serde_json::Value;
use zotero_mcp::core::config::Config;
use zotero_mcp::state::AppState;
use zotero_mcp::tools::{attachments, search};

struct Harness {
    _f: fixtures::build_fixture::Fixture,
    state: AppState,
}

async fn harness() -> Harness {
    let f = fixtures::build_fixture::build();
    let mut cfg = Config::default();
    cfg.zotero.data_dir = f.dir.path().to_string_lossy().into_owned();
    // Pinned deliberately: user_id = 0 would trigger a local-API detection call
    // and make this test depend on Zotero running.
    cfg.zotero.user_id = 1;
    let state = AppState::build(cfg)
        .await
        .expect("AppState should build against the fixture library");
    Harness { _f: f, state }
}

/// Put a handler's return value through the same conversion the MCP server uses,
/// so these assertions are about the real wire payload rather than an internal
/// type.
fn wire<T: IntoCallToolResult>(v: T) -> CallToolResult {
    v.into_call_tool_result()
        .expect("conversion to a tool result")
}

fn text_of(r: &CallToolResult) -> String {
    r.content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("a text content block")
}

// ---------------------------------------------------------------------------
// Family 1: list-returning tools — the envelope
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_tool_returns_an_envelope_with_count() {
    let h = harness().await;
    let r = wire(
        search::search_items(
            &h.state,
            search::SearchArgs {
                query: "Israel".into(),
                item_type: None,
                tag: None,
                collection: None,
                include_fulltext: false,
                limit: 0,
                offset: 0,
            },
        )
        .await
        .unwrap(),
    );

    let body: Value = serde_json::from_str(&text_of(&r)).expect("content is JSON");
    assert!(
        body.is_object(),
        "list tools now return an object envelope, not a bare array: {body}"
    );
    let items = body["items"].as_array().expect("items is an array");
    assert!(
        items.iter().any(|h| h["key"] == "JGF2UTMW"),
        "expected the Rabkin record among items: {body}"
    );
    assert_eq!(
        body["count"].as_u64(),
        Some(items.len() as u64),
        "count must match the items actually returned"
    );
    assert_eq!(
        body["possibly_truncated"].as_bool(),
        Some(false),
        "two hits under a limit of 50 is not truncated: {body}"
    );

    // The typed field carries the same object, so a client reading either sees
    // the same thing.
    assert_eq!(
        r.structured_content.as_ref(),
        Some(&body),
        "structured_content must mirror the content block"
    );
}

#[tokio::test]
async fn list_tool_flags_possible_truncation_at_the_limit() {
    let h = harness().await;
    let r = wire(
        search::search_items(
            &h.state,
            search::SearchArgs {
                query: String::new(),
                item_type: None,
                tag: None,
                collection: None,
                include_fulltext: false,
                limit: 1,
                offset: 0,
            },
        )
        .await
        .unwrap(),
    );
    let body: Value = serde_json::from_str(&text_of(&r)).unwrap();
    assert_eq!(body["count"].as_u64(), Some(1));
    assert_eq!(
        body["possibly_truncated"].as_bool(),
        Some(true),
        "one row returned against a limit of one: the library holds more. {body}"
    );
}

// ---------------------------------------------------------------------------
// Family 2: object-returning tools — regression guard for the 10 already typed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn object_tool_shape_is_unchanged() {
    let h = harness().await;
    let r = wire(
        search::get_item(
            &h.state,
            search::GetItemArgs {
                item_key: Some("JGF2UTMW".into()),
                citation_key: None,
            },
        )
        .await
        .unwrap(),
    );
    let body: Value = serde_json::from_str(&text_of(&r)).unwrap();
    assert_eq!(body["key"], "JGF2UTMW");
    assert!(
        body["fields"].is_object(),
        "item fields should still be a nested object: {body}"
    );
    assert!(
        !body.as_object().unwrap().contains_key("items"),
        "an object tool must NOT gain a list envelope: {body}"
    );
    assert!(r.structured_content.is_some());
}

// ---------------------------------------------------------------------------
// Family 3: text-returning tools — must stay bare text
// ---------------------------------------------------------------------------

/// `get_pdf_path` stands in for the 13 text-returning tools. Chosen because it
/// reads only the local library — `format_citation` would need the Web API and
/// make this test depend on the network.
#[tokio::test]
async fn text_tool_stays_plain_text_with_no_structured_content() {
    let h = harness().await;
    let r = wire(
        attachments::get_pdf_path(
            &h.state,
            attachments::ItemKeyArgs {
                item_key: "AAAA0001".into(),
            },
        )
        .await
        .unwrap(),
    );
    let body = text_of(&r);
    assert!(
        body.ends_with("paper.pdf"),
        "expected a bare filesystem path, got {body:?}"
    );
    assert!(
        serde_json::from_str::<Value>(&body).is_err(),
        "a path is not JSON, and this tool must not start wrapping it: {body:?}"
    );
    assert!(
        r.structured_content.is_none(),
        "text tools deliberately carry no structured content"
    );
}
