use serde_json::json;
use zotero_core::enrichment::propose::compute_diff;

#[test]
fn diff_includes_only_changes() {
    let current = json!({ "title": "Old", "DOI": "10.1/a" });
    let proposed = json!({ "title": "Old", "DOI": "10.1/b", "abstractNote": "added" });
    let d = compute_diff(&current, &proposed);
    let changed: Vec<&str> = d.changes.iter().map(|c| c.field.as_str()).collect();
    assert!(changed.contains(&"DOI"));
    assert!(changed.contains(&"abstractNote"));
    assert!(!changed.contains(&"title"));
}

#[test]
fn diff_empty_when_identical() {
    let v = json!({ "title": "Same", "date": "2024" });
    let d = compute_diff(&v, &v);
    assert!(d.changes.is_empty());
}

#[test]
fn diff_treats_null_proposed_as_no_change_when_field_absent() {
    let current = json!({ "title": "X" });
    let proposed = json!({ "title": "X", "DOI": null });
    let d = compute_diff(&current, &proposed);
    // null proposed for a missing field should not count as a change
    assert!(d.changes.is_empty());
}

#[test]
fn diff_reports_new_non_null_field() {
    let current = json!({ "title": "X" });
    let proposed = json!({ "title": "X", "abstractNote": "new content" });
    let d = compute_diff(&current, &proposed);
    assert_eq!(d.changes.len(), 1);
    assert_eq!(d.changes[0].field, "abstractNote");
    assert!(d.changes[0].current.is_none());
    assert_eq!(d.changes[0].proposed.as_str(), Some("new content"));
}
