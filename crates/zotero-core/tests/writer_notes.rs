use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_core::writer::client::LocalApi;
use zotero_core::writer::notes::add_note;

#[tokio::test]
async fn posts_a_child_note_against_parent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/users/93338/items"))
        .and(header("Zotero-API-Version", "3"))
        .and(body_partial_json(serde_json::json!([{
            "itemType": "note",
            "parentItem": "JGF2UTMW"
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "successful": { "0": { "key": "NEWN0001", "version": 12345 } }
        })))
        .mount(&server).await;

    let api = LocalApi::new(server.uri(), 93338).unwrap();
    let new_key = add_note(&api, "JGF2UTMW", "# Heading\n\nSome **markdown**.").await.unwrap();
    assert_eq!(new_key, "NEWN0001");
}
