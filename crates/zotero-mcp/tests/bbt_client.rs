use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::bbt::BbtClient;

#[tokio::test]
async fn citationkey_lookup_works() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/better-bibtex/json-rpc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": { "JGF2UTMW": "rabkinWhatModernIsrael2016" },
            "id": 1
        })))
        .mount(&server)
        .await;

    let c = BbtClient::new(server.uri()).unwrap();
    let map = c.citationkeys(&["JGF2UTMW".into()]).await.unwrap();
    assert_eq!(
        map.get("JGF2UTMW").map(String::as_str),
        Some("rabkinWhatModernIsrael2016")
    );
}
