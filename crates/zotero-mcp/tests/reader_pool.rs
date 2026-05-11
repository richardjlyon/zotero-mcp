mod fixtures;
use zotero_mcp::core::reader::pool::ReadOnlyPool;

#[tokio::test(flavor = "multi_thread")]
async fn pool_runs_concurrent_queries() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 4).await.unwrap();
    let mut handles = vec![];
    for _ in 0..8 {
        let p = pool.clone();
        handles.push(tokio::spawn(async move {
            p.with_conn(|c| {
                let n: i64 = c.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))?;
                Ok(n)
            }).await.unwrap()
        }));
    }
    for h in handles {
        let n = h.await.unwrap();
        assert!(n > 0);
    }
}
