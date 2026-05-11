mod fixtures;
use zotero_mcp::core::reader::pool::ReadOnlyPool;
use zotero_mcp::core::web::{get_webpage_content, WebMode, WebSource};

#[tokio::test]
async fn snapshot_mode_returns_readable_text() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let res = get_webpage_content(&pool, "WEB00001", 1, &f.storage_dir(), WebMode::Snapshot, "test/0.1").await.unwrap();
    assert!(matches!(res.source, WebSource::Snapshot));
    assert!(res.text.to_lowercase().contains("hello snapshot"));
}
