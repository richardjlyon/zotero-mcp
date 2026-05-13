use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use tempfile::tempdir;
use zotero_mcp::core::cache::DiskCache;
use zotero_mcp::core::enrichment::openlibrary::OpenLibraryClient;

#[tokio::test]
async fn lookup_isbn_normalizes() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/isbn/9780000000000.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "title": "Some Book",
            "publish_date": "March 5, 2020",
            "publishers": ["BookCo"],
            "authors": [{"key":"/authors/OL1A"}]
        }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/authors/OL1A.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "Jane Doe"
        }))).mount(&server).await;

    let dir = tempdir().unwrap();
    let c = OpenLibraryClient::new(server.uri(), DiskCache::new(dir.path().to_path_buf(), 60), "test/0.1");
    let r = c.lookup_isbn("9780000000000").await.unwrap();

    // Envelope assertions (unchanged).
    assert_eq!(r.fields["title"], "Some Book");
    assert_eq!(r.fields["itemType"], "book");
    assert_eq!(r.creators[0].last_name.as_deref(), Some("Doe"));

    // New: date now normalised to ISO 8601.
    assert_eq!(r.fields["date"], "2020-03-05");
    // New: source_url points at the actual record, not just the base URL.
    let expected_url = format!("{}/isbn/9780000000000", server.uri());
    assert_eq!(r.source_url.as_deref(), Some(expected_url.as_str()));
}
