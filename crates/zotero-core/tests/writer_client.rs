use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_core::writer::client::LocalApi;

#[tokio::test]
async fn sends_api_version_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/users/93338/items"))
        .and(header("Zotero-API-Version", "3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let api = LocalApi::new(server.uri(), 93338).unwrap();
    let v = api.list_items_raw("", 0, 1).await.unwrap();
    assert!(v.is_array());
}
