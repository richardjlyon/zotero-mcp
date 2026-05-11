use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use tempfile::tempdir;
use zotero_core::cache::DiskCache;
use zotero_core::enrichment::arxiv::ArxivClient;

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
    Mock::given(method("GET")).and(path("/api/query"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE_ATOM))
        .mount(&server).await;
    let dir = tempdir().unwrap();
    let c = ArxivClient::new(server.uri(), DiskCache::new(dir.path().to_path_buf(), 60), "test/0.1");
    let r = c.lookup_arxiv("2401.00001").await.unwrap();
    assert_eq!(r.fields["title"], "A Cool Preprint");
    assert_eq!(r.fields["itemType"], "preprint");
    assert_eq!(r.creators.len(), 2);
}
