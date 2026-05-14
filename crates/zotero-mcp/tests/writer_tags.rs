use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::writer::client::LocalApi;
use zotero_mcp::core::writer::tags::{
    add_tags, add_to_collection, remove_from_collection, remove_tags,
};

#[tokio::test]
async fn add_tags_round_trips() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/93338/items/JGF2UTMW"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "tags": [{"tag": "existing"}], "version": 10005 }
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/users/93338/items/JGF2UTMW"))
        .and(header("If-Unmodified-Since-Version", "10005"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let api = LocalApi::new("http://unused", 93338)
        .unwrap()
        .with_web_base(server.uri())
        .with_api_key("test-key");
    add_tags(&api, "JGF2UTMW", &["new1".into(), "new2".into()])
        .await
        .unwrap();
}

#[tokio::test]
async fn remove_tags_filters_out_specified() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/93338/items/JGF2UTMW"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "tags": [{"tag": "keep"}, {"tag": "remove-me"}], "version": 10006 }
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/users/93338/items/JGF2UTMW"))
        .and(header("If-Unmodified-Since-Version", "10006"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let api = LocalApi::new("http://unused", 93338)
        .unwrap()
        .with_web_base(server.uri())
        .with_api_key("test-key");
    remove_tags(&api, "JGF2UTMW", &["remove-me".into()])
        .await
        .unwrap();
}

#[tokio::test]
async fn add_to_collection_round_trips() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/93338/items/JGF2UTMW"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "collections": ["COLL1"], "version": 10007 }
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/users/93338/items/JGF2UTMW"))
        .and(header("If-Unmodified-Since-Version", "10007"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let api = LocalApi::new("http://unused", 93338)
        .unwrap()
        .with_web_base(server.uri())
        .with_api_key("test-key");
    add_to_collection(&api, "JGF2UTMW", "COLL2").await.unwrap();
}

#[tokio::test]
async fn remove_from_collection_round_trips() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/93338/items/JGF2UTMW"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "collections": ["COLL1", "COLL2"], "version": 10008 }
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/users/93338/items/JGF2UTMW"))
        .and(header("If-Unmodified-Since-Version", "10008"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let api = LocalApi::new("http://unused", 93338)
        .unwrap()
        .with_web_base(server.uri())
        .with_api_key("test-key");
    remove_from_collection(&api, "JGF2UTMW", "COLL1")
        .await
        .unwrap();
}
