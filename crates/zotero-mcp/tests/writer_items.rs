use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::writer::client::LocalApi;
use zotero_mcp::core::writer::items::{delete_item, update_item_fields};

/// Test helper: build a LocalApi whose web base points at the wiremock
/// server, with a fixed API key the mocks can assert against.
fn test_api(server: &MockServer) -> LocalApi {
    LocalApi::new("http://unused-local-base", 93338)
        .unwrap()
        .with_web_base(server.uri())
        .with_api_key("test-key")
}

#[tokio::test]
async fn patches_item_with_version_header() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/users/93338/items/JGF2UTMW"))
        .and(header("Zotero-API-Version", "3"))
        .and(header("If-Unmodified-Since-Version", "10005"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let api = test_api(&server);
    let fields = serde_json::json!({ "abstractNote": "New abstract." });
    update_item_fields(&api, "JGF2UTMW", 10005, fields)
        .await
        .unwrap();
}

#[tokio::test]
async fn version_conflict_returns_typed_error() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(412).set_body_string("Precondition Failed"))
        .mount(&server)
        .await;
    let api = test_api(&server);
    let err = update_item_fields(&api, "JGF2UTMW", 10005, serde_json::json!({}))
        .await
        .unwrap_err();
    assert!(matches!(err, zotero_mcp::core::Error::VersionConflict(_)));
}

#[tokio::test]
async fn write_without_api_key_returns_write_api_key_missing() {
    let server = MockServer::start().await;
    // No mock — the request should not even be sent.
    let api = LocalApi::new("http://unused-local-base", 93338)
        .unwrap()
        .with_web_base(server.uri());
    let err = update_item_fields(&api, "JGF2UTMW", 10005, serde_json::json!({}))
        .await
        .unwrap_err();
    assert!(matches!(err, zotero_mcp::core::Error::WriteApiKeyMissing));
}

#[tokio::test]
async fn delete_item_issues_versioned_delete() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/users/93338/items/JGF2UTMW"))
        .and(header("Zotero-API-Version", "3"))
        .and(header("If-Unmodified-Since-Version", "10010"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let api = test_api(&server);
    delete_item(&api, "JGF2UTMW", 10010).await.unwrap();
}

#[tokio::test]
async fn delete_item_propagates_version_conflict() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(412).set_body_string("Precondition Failed"))
        .mount(&server)
        .await;
    let api = test_api(&server);
    let err = delete_item(&api, "JGF2UTMW", 9999).await.unwrap_err();
    assert!(matches!(err, zotero_mcp::core::Error::VersionConflict(_)));
}
