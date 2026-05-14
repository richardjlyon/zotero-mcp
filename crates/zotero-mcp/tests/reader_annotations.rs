mod fixtures;
use zotero_mcp::core::reader::annotations::list_annotations;
use zotero_mcp::core::reader::pool::ReadOnlyPool;

#[tokio::test]
async fn lists_annotations_for_item() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let anns = list_annotations(&pool, "AAAA0001", 1).await.unwrap();
    assert_eq!(anns.len(), 1);
    let a = &anns[0];
    assert_eq!(a.text.as_deref(), Some("A highlighted passage."));
    assert_eq!(a.comment.as_deref(), Some("My note on it."));
    assert_eq!(a.color.as_deref(), Some("#ffff00"));
    assert_eq!(a.kind, "highlight");
}
