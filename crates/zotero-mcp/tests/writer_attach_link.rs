use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::writer::attachments::attach_link;
use zotero_mcp::core::writer::client::LocalApi;

fn api(server_uri: &str) -> LocalApi {
    LocalApi::new("http://unused", 93338)
        .unwrap()
        .with_web_base(server_uri)
        .with_api_key("test-key")
}

#[tokio::test]
async fn attach_link_posts_linked_url_attachment_and_returns_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(header("Zotero-API-Version", "3"))
        .and(body_partial_json(json!([{
            "itemType": "attachment",
            "parentItem": "PARENT01",
            "linkMode": "linked_url",
            "url": "https://example.com/test",
            "title": "Example page"
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "LINKK001", "version": 7 } }
        })))
        .mount(&server)
        .await;

    let key = attach_link(
        &api(&server.uri()),
        "PARENT01",
        "https://example.com/test",
        Some("Example page"),
    )
    .await
    .unwrap();
    assert_eq!(key, "LINKK001");
}

#[tokio::test]
async fn attach_link_uses_url_as_title_when_omitted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .and(body_partial_json(json!([{
            "linkMode": "linked_url",
            "url": "https://example.com/page",
            "title": "https://example.com/page"
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "successful": { "0": { "key": "K", "version": 1 } }
        })))
        .mount(&server)
        .await;

    let _ = attach_link(
        &api(&server.uri()),
        "PARENT01",
        "https://example.com/page",
        None,
    )
    .await
    .unwrap();
}
