use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use tempfile::tempdir;
use zotero_mcp::core::cache::DiskCache;
use zotero_mcp::core::enrichment::semantic_scholar::SemanticScholarClient;

#[tokio::test]
async fn search_normalizes_results() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/graph/v1/paper/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{
                "paperId": "abc",
                "title": "A Paper on Things",
                "year": 2024,
                "abstract": "Body.",
                "externalIds": {"DOI": "10.1234/abcd"},
                "authors": [{"name":"Alice Aardvark"}]
            }]
        }))).mount(&server).await;

    let dir = tempdir().unwrap();
    let c = SemanticScholarClient::new(server.uri(), DiskCache::new(dir.path().to_path_buf(), 60), "test/0.1", None);
    let v = c.search("paper on things", 1).await.unwrap();
    assert_eq!(v[0].fields["title"], "A Paper on Things");
    assert_eq!(v[0].fields["DOI"], "10.1234/abcd");
}
