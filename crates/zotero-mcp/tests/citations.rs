use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::citations::{format_citation, format_bibliography};
use zotero_mcp::core::writer::client::LocalApi;

#[tokio::test]
async fn formats_single_citation_as_bib() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/users/93338/items/JGF2UTMW"))
        .and(query_param("format", "bib"))
        .and(query_param("style", "apa"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<div>Rabkin, Y. (2016). ...</div>"))
        .mount(&server).await;

    let api = LocalApi::new(server.uri(), 93338).unwrap();
    let s = format_citation(&api, "JGF2UTMW", "apa", "bib").await.unwrap();
    assert!(s.contains("Rabkin"));
}

#[tokio::test]
async fn formats_bibliography_for_multiple_keys() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/users/93338/items"))
        .and(query_param("format", "bib"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<div>combined bib</div>"))
        .mount(&server).await;

    let api = LocalApi::new(server.uri(), 93338).unwrap();
    let s = format_bibliography(&api, &["A".into(), "B".into()], "chicago-author-date", "bib").await.unwrap();
    assert!(s.contains("combined"));
}
