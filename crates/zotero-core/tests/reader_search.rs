mod fixtures;
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::reader::search::{search_metadata, SearchParams};

#[tokio::test]
async fn finds_items_by_title_substring() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let hits = search_metadata(&pool, 1, SearchParams { query: "Israel".into(), ..Default::default() }).await.unwrap();
    assert!(hits.iter().any(|h| h.key == "JGF2UTMW"));
}

#[tokio::test]
async fn finds_items_by_creator_lastname() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let hits = search_metadata(&pool, 1, SearchParams { query: "Rabkin".into(), ..Default::default() }).await.unwrap();
    assert!(hits.iter().any(|h| h.key == "JGF2UTMW"));
}

#[tokio::test]
async fn limit_and_offset_work() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let hits = search_metadata(&pool, 1, SearchParams {
        query: String::new(),
        limit: 1, offset: 0, ..Default::default()
    }).await.unwrap();
    assert_eq!(hits.len(), 1);
}
