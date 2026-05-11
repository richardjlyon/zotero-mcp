mod fixtures;
use zotero_mcp::core::reader::pool::ReadOnlyPool;
use zotero_mcp::core::reader::items::get_item_by_key;

#[tokio::test]
async fn fetches_item_with_fields_and_creators() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();

    let item = get_item_by_key(&pool, "JGF2UTMW", 1).await.unwrap();
    assert_eq!(item.item_type, "book");
    assert_eq!(item.fields["title"], "What is Modern Israel?");
    assert_eq!(item.fields["date"], "2016");
    assert_eq!(item.fields["publisher"], "Pluto Press");
    assert_eq!(item.creators.len(), 1);
    assert_eq!(item.creators[0].last_name.as_deref(), Some("Rabkin"));
    assert_eq!(item.creators[0].creator_type, "author");
    assert_eq!(item.collection_keys, vec!["COL00001"]);
    assert!(item.recommended_content_tool.is_none()); // no PDF attached directly
}

#[tokio::test]
async fn missing_item_returns_error() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let err = get_item_by_key(&pool, "DOESNOTEXIST", 1).await.unwrap_err();
    assert!(matches!(err, zotero_mcp::core::Error::ItemNotFound(_)));
}
