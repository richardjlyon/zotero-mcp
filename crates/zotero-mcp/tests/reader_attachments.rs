mod fixtures;
use zotero_mcp::core::reader::attachments::{list_attachments, resolve_path};
use zotero_mcp::core::reader::pool::ReadOnlyPool;

#[tokio::test]
async fn lists_pdf_attachment_and_resolves_path() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();

    let atts = list_attachments(&pool, "AAAA0001", 1, &f.storage_dir())
        .await
        .unwrap();
    assert_eq!(atts.len(), 1);
    let a = &atts[0];
    assert_eq!(a.content_type.as_deref(), Some("application/pdf"));
    assert!(a
        .absolute_path
        .as_ref()
        .unwrap()
        .ends_with("AAAA0001/paper.pdf"));

    let p = resolve_path(&pool, "AAAA0001", 1, &f.storage_dir())
        .await
        .unwrap();
    assert!(p.ends_with("AAAA0001/paper.pdf"));
}
