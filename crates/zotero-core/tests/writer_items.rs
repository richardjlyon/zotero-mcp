use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_core::writer::client::LocalApi;
use zotero_core::writer::items::update_item_fields;

#[tokio::test]
async fn patches_item_with_version_header() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/api/users/93338/items/JGF2UTMW"))
        .and(header("Zotero-API-Version", "3"))
        .and(header("If-Unmodified-Since-Version", "10005"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server).await;

    let api = LocalApi::new(server.uri(), 93338).unwrap();
    let fields = serde_json::json!({ "abstractNote": "New abstract." });
    update_item_fields(&api, "JGF2UTMW", 10005, fields).await.unwrap();
}

#[tokio::test]
async fn version_conflict_returns_typed_error() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(412).set_body_string("Precondition Failed"))
        .mount(&server).await;
    let api = LocalApi::new(server.uri(), 93338).unwrap();
    let err = update_item_fields(&api, "JGF2UTMW", 10005, serde_json::json!({})).await.unwrap_err();
    assert!(matches!(err, zotero_core::Error::VersionConflict(_)));
}
