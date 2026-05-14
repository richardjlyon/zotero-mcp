mod fixtures;
use zotero_mcp::core::reader::fulltext::fulltext_match_items;
use zotero_mcp::core::reader::pool::ReadOnlyPool;

#[tokio::test]
async fn matches_items_by_indexed_word() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    // The fixture's PDF attachment (key AAAA0001) has fulltextWord "zoteroconnectortest".
    // That attachment's parent item is key AAAA0001 (item ID 2).
    let parents = fulltext_match_items(&pool, 1, "zoteroconnectortest")
        .await
        .unwrap();
    assert!(parents.contains(&"AAAA0001".to_string()));
}

#[tokio::test]
async fn unknown_word_returns_empty() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let parents = fulltext_match_items(&pool, 1, "nonexistentwordxyz")
        .await
        .unwrap();
    assert!(parents.is_empty());
}
