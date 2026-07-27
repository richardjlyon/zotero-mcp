//! Walks the whole tool surface and checks every schema.
//!
//! This is the test that was missing when a single malformed schema cost this
//! server its entire tool list: Claude Code's validator rejected the
//! `tools/list` response wholesale, so the server showed as connected with zero
//! usable tools — a failure that reads like a broken connection, not a bad
//! schema.
//!
//! Building the router is itself half the guard. rmcp evaluates every tool's
//! output schema at router-construction time and *panics* if one lacks a root
//! `"type": "object"` (`handler/server/common.rs` → `router/tool/tool_traits.rs`),
//! so a tool that would kill the server at startup cannot pass this test.

use zotero_mcp::server::ZoteroServer;

/// Update deliberately when adding or removing a tool — a change here should be
/// a decision, not a surprise.
const EXPECTED_TOOL_COUNT: usize = 37;

fn all_tools() -> Vec<rmcp::model::Tool> {
    // Constructing the router is the operation that validates output schemas.
    ZoteroServer::tool_router().list_all()
}

#[test]
fn every_tool_has_an_object_rooted_input_schema() {
    for t in all_tools() {
        let root = t.input_schema.get("type").and_then(|v| v.as_str());
        assert_eq!(
            root,
            Some("object"),
            "tool `{}` has input schema root type {:?}; MCP requires an object. Full schema: {}",
            t.name,
            root,
            serde_json::to_string_pretty(&*t.input_schema).unwrap()
        );
    }
}

#[test]
fn every_output_schema_is_object_rooted() {
    for t in all_tools() {
        let Some(schema) = t.output_schema.as_ref() else {
            continue; // Text-returning tools legitimately have none.
        };
        let root = schema.get("type").and_then(|v| v.as_str());
        assert_eq!(
            root,
            Some("object"),
            "tool `{}` has output schema root type {:?}; rmcp panics at startup on anything else. \
             Full schema: {}",
            t.name,
            root,
            serde_json::to_string_pretty(&**schema).unwrap()
        );
    }
}

#[test]
fn every_tool_is_described_and_annotated() {
    for t in all_tools() {
        let desc = t.description.as_deref().unwrap_or("");
        assert!(
            !desc.trim().is_empty(),
            "tool `{}` has no description — the model picks tools by these",
            t.name
        );
        let ann = t
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("tool `{}` carries no annotations", t.name));
        assert!(
            ann.read_only_hint.is_some(),
            "tool `{}` does not declare read_only_hint; clients rely on it for retry \
             and confirmation behaviour",
            t.name
        );
    }
}

/// The search tool must state what its full-text option actually covers.
/// A caller who reads a miss as "not in the document" will report absence for
/// content that is present — the failure this contract exists to prevent — so
/// the three real limits are asserted, not left to prose drift.
#[test]
fn search_tool_states_its_fulltext_coverage() {
    let tools = all_tools();
    let search = tools
        .iter()
        .find(|t| t.name == "search_items")
        .expect("search_items must exist");
    let d = search.description.as_deref().unwrap_or_default().to_lowercase();
    assert!(
        d.contains("zotero's own"),
        "must say whose index is searched"
    );
    assert!(
        d.contains("derivative"),
        "must say stored derivatives are not searched"
    );
    assert!(
        d.contains("single-word"),
        "must say multi-word queries drop full-text matching"
    );
    assert!(
        d.contains("parent item"),
        "must say matching resolves through parent items"
    );
}

#[test]
fn tool_count_and_key_names_are_stable() {
    let tools = all_tools();
    assert_eq!(
        tools.len(),
        EXPECTED_TOOL_COUNT,
        "tool count changed; update EXPECTED_TOOL_COUNT deliberately. Names: {:?}",
        tools.iter().map(|t| t.name.as_ref()).collect::<Vec<_>>()
    );
    // A rename should be visible rather than silent — these are the ones skills
    // and prose refer to by name.
    for name in [
        "search_items",
        "get_item",
        "find_duplicates",
        "attach_file",
        "lookup_isbn",
        "get_pdf_text",
        "get_derivative_path",
        "build_derivatives",
    ] {
        assert!(
            tools.iter().any(|t| t.name == name),
            "tool `{name}` is missing from the router"
        );
    }
}
