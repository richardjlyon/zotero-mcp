mod fixtures;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_mcp::core::bbt::BbtClient;
use zotero_mcp::core::reader::pool::ReadOnlyPool;
use zotero_mcp::core::reader::items::{get_item_by_key, hydrate_citation_key};

#[tokio::test]
async fn hydrates_citation_key_from_bbt() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let mut item = get_item_by_key(&pool, "JGF2UTMW", 1).await.unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/better-bibtex/json-rpc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc":"2.0","result":{"JGF2UTMW":"rabkin2016"},"id":1
        }))).mount(&server).await;
    let bbt = BbtClient::new(server.uri()).unwrap();
    hydrate_citation_key(&mut item, Some(&bbt)).await;
    assert_eq!(item.citation_key.as_deref(), Some("rabkin2016"));
}
