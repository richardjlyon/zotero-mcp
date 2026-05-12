use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::error::Error;
use zotero_mcp::core::writer::client::LocalApi;
use zotero_mcp::core::writer::items::create_item;

fn api(server_uri: &str) -> LocalApi {
    LocalApi::new("http://unused", 93338)
        .unwrap()
        .with_web_base(server_uri)
        .with_api_key("test-key")
}

#[tokio::test]
async fn creates_item_and_returns_key_and_version() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(header("Zotero-API-Version", "3"))
        .and(body_partial_json(json!([{
            "itemType": "journalArticle",
            "title": "Test paper"
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "NEWK0001", "version": 42 } }
        })))
        .mount(&server)
        .await;

    let item = json!({
        "itemType": "journalArticle",
        "title": "Test paper"
    });
    let (key, version) = create_item(&api(&server.uri()), &item, &[]).await.unwrap();
    assert_eq!(key, "NEWK0001");
    assert_eq!(version, 42);
}

#[tokio::test]
async fn merges_collection_keys_into_item() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(body_partial_json(json!([{
            "itemType": "book",
            "collections": ["COLL0001", "COLL0002"]
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "BOOKK001", "version": 1 } }
        })))
        .mount(&server)
        .await;

    let item = json!({ "itemType": "book", "title": "x" });
    let _ = create_item(
        &api(&server.uri()),
        &item,
        &["COLL0001".into(), "COLL0002".into()],
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn surfaces_zotero_400_as_localapi_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad itemType"))
        .mount(&server)
        .await;

    let item = json!({ "itemType": "nonsense" });
    let err = create_item(&api(&server.uri()), &item, &[]).await.unwrap_err();
    match err {
        Error::LocalApi { status, body } => {
            assert_eq!(status, 400);
            assert!(body.contains("bad itemType"));
        }
        other => panic!("expected LocalApi(400), got {:?}", other),
    }
}

#[tokio::test]
async fn missing_api_key_returns_write_api_key_missing() {
    // No web base / api key configured — write_request errors before any send.
    let api = LocalApi::new("http://unused", 93338).unwrap();
    let item = json!({ "itemType": "journalArticle" });
    let err = create_item(&api, &item, &[]).await.unwrap_err();
    assert!(matches!(err, Error::WriteApiKeyMissing));
}
