use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use tempfile::tempdir;
use zotero_mcp::core::cache::DiskCache;
use zotero_mcp::core::enrichment::crossref::CrossrefClient;

#[tokio::test]
async fn lookup_doi_normalizes_to_zotero_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/works/10.1234/abcd"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "ok",
            "message": {
                "DOI": "10.1234/abcd",
                "title": ["A Paper on Things"],
                "author": [{"given":"Alice","family":"Aardvark"}],
                "issued": {"date-parts": [[2024, 3]]},
                "container-title": ["Journal of Things"],
                "publisher": "ThingPress",
                "type": "journal-article",
                "URL": "https://doi.org/10.1234/abcd",
                "abstract": "Abstract content."
            }
        })))
        .mount(&server).await;

    let dir = tempdir().unwrap();
    let cache = DiskCache::new(dir.path().to_path_buf(), 60);
    let c = CrossrefClient::new(server.uri(), cache, "zotero-mcp/0.1");
    let norm = c.lookup_doi("10.1234/abcd").await.unwrap();
    assert_eq!(norm.fields["title"], "A Paper on Things");
    assert_eq!(norm.fields["DOI"], "10.1234/abcd");
    assert_eq!(norm.fields["date"], "2024-03");
    assert_eq!(norm.fields["itemType"], "journalArticle");
    assert_eq!(norm.creators[0].last_name.as_deref(), Some("Aardvark"));
}
