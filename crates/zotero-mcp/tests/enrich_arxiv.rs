use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::cache::DiskCache;
use zotero_mcp::core::enrichment::arxiv::ArxivClient;
use zotero_mcp::core::enrichment::normalized_to_item;

const SAMPLE_ATOM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<entry>
  <id>http://arxiv.org/abs/2401.00001v1</id>
  <title>A Cool Preprint</title>
  <summary>Abstract here.</summary>
  <published>2024-01-01T00:00:00Z</published>
  <author><name>Alice Aardvark</name></author>
  <author><name>Bob Baboon</name></author>
</entry>
</feed>"#;

#[tokio::test]
async fn lookup_arxiv_parses_atom() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/query"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE_ATOM))
        .mount(&server)
        .await;
    let dir = tempdir().unwrap();
    let c = ArxivClient::new(
        server.uri(),
        DiskCache::new(dir.path().to_path_buf(), 60),
        "test/0.1",
    );
    let r = c.lookup_arxiv("2401.00001").await.unwrap();

    // Envelope assertions.
    assert_eq!(r.fields["title"], "A Cool Preprint");
    assert_eq!(r.fields["itemType"], "preprint");
    assert_eq!(r.creators.len(), 2);

    // Flat-shape assertions.
    let v = normalized_to_item(&r);
    assert_eq!(v["itemType"], "preprint");
    assert_eq!(v["title"], "A Cool Preprint");
    assert_eq!(v["date"], "2024-01-01");
    assert_eq!(v["creators"].as_array().unwrap().len(), 2);

    let extra = v["extra"].as_str().unwrap();
    assert!(extra.contains("source: arxiv"));
    // arXiv parser does not populate source_url today.
    assert!(!extra.contains("sourceURL"), "got: {extra:?}");
}
