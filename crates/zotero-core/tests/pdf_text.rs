mod fixtures;
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::pdf::{get_pdf_text, PdfTextSource};

#[tokio::test]
async fn prefers_zotero_ft_cache_when_present() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let res = get_pdf_text(&pool, "AAAA0001", 1, &f.storage_dir()).await.unwrap();
    assert!(matches!(res.source, PdfTextSource::ZoteroCache));
    assert!(res.text.contains("zoteroconnectortest"));
}
