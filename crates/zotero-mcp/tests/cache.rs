use tempfile::tempdir;
use zotero_mcp::core::cache::DiskCache;

#[tokio::test]
async fn round_trips_json_with_ttl() {
    let dir = tempdir().unwrap();
    let cache = DiskCache::new(dir.path().to_path_buf(), 60 * 60);
    let key = "crossref:10.1/abc";
    cache
        .put(key, &serde_json::json!({"title":"hello"}))
        .await
        .unwrap();
    let v: serde_json::Value = cache.get(key).await.unwrap().expect("hit");
    assert_eq!(v["title"], "hello");
}

#[tokio::test]
async fn expired_returns_none() {
    let dir = tempdir().unwrap();
    let cache = DiskCache::new(dir.path().to_path_buf(), 0); // expires immediately
    cache.put("k", &serde_json::json!(1)).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let v: Option<serde_json::Value> = cache.get("k").await.unwrap();
    assert!(v.is_none());
}
