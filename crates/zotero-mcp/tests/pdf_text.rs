mod fixtures;
use zotero_mcp::core::pdf::{get_pdf_text, PdfEngines, PdfTextSource};
use zotero_mcp::core::reader::pool::ReadOnlyPool;

#[tokio::test]
async fn prefers_zotero_ft_cache_when_present() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let engines = PdfEngines::build(&zotero_mcp::core::config::Config::default().zotero);
    let res = get_pdf_text(&pool, "AAAA0001", 1, &f.storage_dir(), &engines, false, None)
        .await
        .unwrap();
    assert!(matches!(res.source, PdfTextSource::ZoteroCache));
    assert!(res.text.contains("zoteroconnectortest"));
}

use std::path::PathBuf;
use std::time::Duration;
use zotero_mcp::core::pdf::{EngineError, PdfEngine, PdftotextEngine};

/// Locate `pdftotext` on PATH; return None to signal "skip this test on this host".
fn locate_pdftotext() -> Option<PathBuf> {
    which::which("pdftotext").ok()
}

#[tokio::test]
async fn pdftotext_engine_extracts_text_from_hello_pdf() {
    let Some(bin) = locate_pdftotext() else {
        eprintln!("pdftotext not on PATH; skipping integration test");
        return;
    };
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.pdf");
    assert!(
        fixture.exists(),
        "hello.pdf fixture missing — run tests/fixtures/gen_pdfs.py"
    );

    let eng = PdftotextEngine::new(bin);
    let text = eng.extract(&fixture).await.expect("extraction succeeded");
    assert!(text.contains("Hello fallback world"), "got: {:?}", text);
}

#[tokio::test]
async fn pdftotext_engine_returns_timeout_when_deadline_exceeded() {
    let Some(bin) = locate_pdftotext() else {
        eprintln!("pdftotext not on PATH; skipping integration test");
        return;
    };
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.pdf");

    // 1 ns is unreachable; the timer fires before pdftotext can even spawn-and-exit.
    let eng = PdftotextEngine::with_timeout(bin, Duration::from_nanos(1));
    let err: EngineError = eng.extract(&fixture).await.expect_err("should time out");
    assert!(matches!(err, EngineError::Timeout(0)), "got: {:?}", err);
}
