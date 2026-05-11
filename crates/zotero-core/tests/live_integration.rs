//! Live test against the user's real local Zotero. Gated by env var so CI
//! doesn't try to run it. Execute manually:
//!
//!     ZOTERO_MCP_LIVE_TEST=1 cargo test -p zotero-core --test live_integration -- --nocapture

use zotero_core::reader::conn::{check_schema, open_read_only};
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::reader::search::{search_metadata, SearchParams};

fn enabled() -> bool {
    std::env::var("ZOTERO_MCP_LIVE_TEST").is_ok()
}

#[tokio::test]
async fn live_schema_and_search() {
    if !enabled() {
        eprintln!("skipped (set ZOTERO_MCP_LIVE_TEST=1 to run)");
        return;
    }
    let path = directories::UserDirs::new()
        .expect("could not resolve home directory")
        .home_dir()
        .join("Zotero/zotero.sqlite");

    let conn = open_read_only(&path).expect("open zotero.sqlite read-only");
    let v = check_schema(&conn, 100, 150).expect("schema in tested range");
    eprintln!("userdata schema version: {}", v);

    let pool = ReadOnlyPool::new(path, 2).await.unwrap();
    let hits = search_metadata(
        &pool,
        1,
        SearchParams { query: "the".into(), limit: 3, ..Default::default() },
    )
    .await
    .unwrap();
    assert!(!hits.is_empty(), "expected at least one hit for 'the'");
    eprintln!("found {} hits", hits.len());
}
