mod fixtures;
use rusqlite::Connection;

#[test]
fn fixture_has_expected_items() {
    let f = fixtures::build_fixture::build();
    let conn = Connection::open(f.sqlite_path()).unwrap();
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM items WHERE itemTypeID IN (2,4,14)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // 3 original regular items + the two dedup fixtures (GAZA0001, TRSH0001).
    assert_eq!(n, 5);
    // One of those five is in the trash and must be filterable.
    let trashed: i64 = conn
        .query_row("SELECT COUNT(*) FROM deletedItems", [], |r| r.get(0))
        .unwrap();
    assert_eq!(trashed, 1);
    let title: String = conn.query_row(
        "SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID WHERE d.itemID=1 AND d.fieldID=1",
        [], |r| r.get(0)).unwrap();
    assert_eq!(title, "What is Modern Israel?");
}
