use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::cache::DiskCache;
use zotero_mcp::core::enrichment::crossref::CrossrefClient;
use zotero_mcp::core::enrichment::normalized_to_item;

#[tokio::test]
async fn lookup_doi_normalizes_to_zotero_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/works/10.1234/abcd"))
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
        .mount(&server)
        .await;

    let dir = tempdir().unwrap();
    let cache = DiskCache::new(dir.path().to_path_buf(), 60);
    let c = CrossrefClient::new(server.uri(), cache, "zotero-mcp/0.1");
    let r = c.lookup_doi("10.1234/abcd").await.unwrap();

    // Envelope assertions.
    assert_eq!(r.fields["title"], "A Paper on Things");
    assert_eq!(r.fields["DOI"], "10.1234/abcd");
    assert_eq!(r.fields["date"], "2024-03");
    assert_eq!(r.fields["itemType"], "journalArticle");
    assert_eq!(r.creators[0].last_name.as_deref(), Some("Aardvark"));

    // Flat-shape assertions.
    let v = normalized_to_item(&r);
    assert_eq!(v["itemType"], "journalArticle");
    assert_eq!(v["DOI"], "10.1234/abcd");
    assert_eq!(v["date"], "2024-03");
    assert_eq!(v["creators"][0]["creatorType"], "author");
    assert_eq!(v["creators"][0]["firstName"], "Alice");
    assert_eq!(v["creators"][0]["lastName"], "Aardvark");

    let extra = v["extra"].as_str().unwrap();
    assert!(extra.contains("source: crossref"));
    assert!(extra.contains("sourceURL: https://doi.org/10.1234/abcd"));
}
