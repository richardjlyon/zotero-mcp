use schemars::schema_for;
use zotero_mcp::core::dedup::FindDuplicatesResult;
use zotero_mcp::tools::attachments::CreateItemArgs;
use zotero_mcp::tools::dedup::FindDuplicatesArgs;
use zotero_mcp::tools::enrichment::{EnrichArgs, ProposeArgs};

fn property_type(schema_json: &serde_json::Value, name: &str) -> String {
    schema_json["properties"][name]["type"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| {
            panic!(
                "property `{}` has no `type`; full schema: {}",
                name,
                serde_json::to_string_pretty(schema_json).unwrap()
            )
        })
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

// A malformed schema is not a small problem: Claude Code's validator rejects the
// whole `tools/list` response, so the server shows as connected with zero usable
// tools. Both sides of find_duplicates get an explicit object-root check.

#[test]
fn find_duplicates_args_root_is_object() {
    let json = serde_json::to_value(schema_for!(FindDuplicatesArgs)).unwrap();
    assert_eq!(json["type"].as_str(), Some("object"));
    assert!(json["properties"]["input_kind"].is_object());
}

#[test]
fn find_duplicates_result_root_is_object() {
    let json = serde_json::to_value(schema_for!(FindDuplicatesResult)).unwrap();
    assert_eq!(
        json["type"].as_str(),
        Some("object"),
        "rmcp requires a type:object root on tool output; got {}",
        serde_json::to_string_pretty(&json).unwrap()
    );
    assert_eq!(property_type(&json, "candidates"), "array");
    assert_eq!(property_type(&json, "queries_run"), "array");
}
