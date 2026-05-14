mod fixtures;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::reader::pool::ReadOnlyPool;
use zotero_mcp::core::web::refetch_url;
use zotero_mcp::core::writer::client::LocalApi;

#[tokio::test]
async fn refetches_and_saves_snapshot() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();

    let live = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/article"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<html><body><article><h1>Live</h1><p>Body.</p></article></body></html>",
        ))
        .mount(&live)
        .await;

    // Patch fixture: set the WEB00001 item's URL (valueID = 21) to the mock server.
    {
        let conn = rusqlite::Connection::open(f.sqlite_path()).unwrap();
        let url = format!("{}/article", live.uri());
        conn.execute(
            "UPDATE itemDataValues SET value = ?1 WHERE valueID = 21",
            [url],
        )
        .unwrap();
    }

    let api_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/93338/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "successful": {"0": {"key": "SNAP0001", "version": 7}}
        })))
        .mount(&api_server)
        .await;

    let api = LocalApi::new("http://unused", 93338)
        .unwrap()
        .with_web_base(api_server.uri())
        .with_api_key("test-key");
    let r = refetch_url(&pool, Some(&api), "WEB00001", 1, true, "test/0.1")
        .await
        .unwrap();
    assert_eq!(r.saved_attachment_key.as_deref(), Some("SNAP0001"));
    assert!(r.text.to_lowercase().contains("body"));
}
