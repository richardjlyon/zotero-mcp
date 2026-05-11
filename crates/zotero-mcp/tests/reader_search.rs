mod fixtures;
use zotero_mcp::core::reader::pool::ReadOnlyPool;
use zotero_mcp::core::reader::search::{search_metadata, SearchParams};

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

#[tokio::test]
async fn fulltext_finds_pdf_word() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let hits = search_metadata(&pool, 1, SearchParams {
        query: "zoteroconnectortest".into(),
        include_fulltext: true,
        ..Default::default()
    }).await.unwrap();
    assert!(hits.iter().any(|h| h.key == "AAAA0001"));
}
