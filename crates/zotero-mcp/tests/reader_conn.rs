mod fixtures;
use zotero_mcp::core::reader::conn::{check_schema, open_read_only};

#[test]
fn opens_read_only_and_passes_schema_check() {
    let f = fixtures::build_fixture::build();
    let conn = open_read_only(&f.sqlite_path()).unwrap();
    let v = check_schema(&conn, 120, 135).unwrap();
    assert_eq!(v, 125);
}

#[test]
fn rejects_unknown_schema_version() {
    let f = fixtures::build_fixture::build();
    let conn = open_read_only(&f.sqlite_path()).unwrap();
    let err = check_schema(&conn, 200, 210).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("125"));
    assert!(msg.contains("200"));
}
