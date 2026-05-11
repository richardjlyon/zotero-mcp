mod fixtures;
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::reader::{collections, recent, tags};

#[tokio::test]
async fn lists_collections_tags_and_recent() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();

    let cs = collections::list(&pool, 1, None).await.unwrap();
    assert_eq!(cs.len(), 1);
    assert_eq!(cs[0].key, "COL00001");
    assert_eq!(cs[0].name, "Reading List");

    let ts = tags::list(&pool, 1, None).await.unwrap();
    assert!(ts.iter().any(|t| t.name == "history" && t.item_count == 1));

    let rs = recent::list(&pool, 1, "dateModified", 5).await.unwrap();
    assert!(rs.len() >= 3);
}
