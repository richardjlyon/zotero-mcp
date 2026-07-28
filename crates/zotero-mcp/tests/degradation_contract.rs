//! The degradation contract: a flat-text substitute is never delivered as an
//! ordinary success when a layout route was configured and expected.
//!
//! The incident this encodes: the same call minutes apart returned
//! `source: live_extract` / plain text and then `source: docling` / markdown
//! with tables — both successes, distinguished only by a metadata field the
//! caller had to know to inspect. Character volume was comparable (3,496 flat
//! vs 3,299 markdown on page 1), so no size check catches it, and it depends
//! on service warmth, so it does not reproduce reliably.
//!
//! Every test here runs on any host: the "configured but unavailable" posture
//! is simulated with a dead endpoint rather than by stopping a real service.

mod fixtures;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use zotero_mcp::core::config::ZoteroConfig;
use zotero_mcp::core::derivatives::{DerivativeMeta, DerivativeStore, EXTRACTION_PROFILE};
use zotero_mcp::core::error::Error;
use zotero_mcp::core::pdf::{
    extract, get_pdf_text_stored, Completeness, DoclingEngine, ExtractPolicy, PdfEngines,
    PdfFormat, PdfTextSource, ServedFrom,
};
use zotero_mcp::core::reader::pool::ReadOnlyPool;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// No layout route configured at all — CI, the Pi, any host without the GPU
/// box. Nothing about this posture changes.
fn no_route() -> PdfEngines {
    PdfEngines::build(&ZoteroConfig::default()).with_docling(None)
}

/// A layout route IS configured and is unreachable. Port 9 is the discard
/// service: nothing listens, so the health probe fails fast.
fn configured_but_dead() -> PdfEngines {
    let dead = Arc::new(DoclingEngine::new(
        "http://127.0.0.1:9".to_string(),
        Duration::from_secs(2),
        Duration::from_millis(200),
    ));
    PdfEngines::build(&ZoteroConfig::default()).with_docling(Some(dead))
}

#[tokio::test]
async fn no_layout_route_configured_still_succeeds_without_opt_in() {
    let dir = tempfile::tempdir().unwrap();
    let r = extract(
        &fixture("hello.pdf"),
        dir.path(),
        &no_route(),
        ExtractPolicy::default(),
    )
    .await
    .expect("a host with no layout route must keep working exactly as before");
    assert_eq!(r.format, PdfFormat::Plain);
    assert!(!r.completeness.complete, "still labelled incomplete");
    assert!(r.text.contains("Hello fallback world"));
}

#[tokio::test]
async fn configured_but_unavailable_refuses_without_opt_in() {
    let dir = tempfile::tempdir().unwrap();
    let err = extract(
        &fixture("tables.pdf"),
        dir.path(),
        &configured_but_dead(),
        ExtractPolicy::default(),
    )
    .await
    .expect_err("a cold layout route must not quietly downgrade the caller");

    match err {
        Error::LayoutRouteUnavailable { endpoint, .. } => {
            assert!(
                endpoint.contains("127.0.0.1:9"),
                "the error must name what is down, not merely that something is: {endpoint}"
            );
            let msg = Error::LayoutRouteUnavailable {
                path: "x".into(),
                endpoint,
            }
            .to_string();
            assert!(
                msg.contains("allow_degraded"),
                "the error must name the opt-in as the remedy"
            );
            assert!(
                msg.contains("table"),
                "the error must say what is actually lost"
            );
        }
        other => panic!("expected LayoutRouteUnavailable, got {other}"),
    }
}

#[tokio::test]
async fn configured_but_unavailable_succeeds_with_opt_in() {
    let dir = tempfile::tempdir().unwrap();
    let r = extract(
        &fixture("tables.pdf"),
        dir.path(),
        &configured_but_dead(),
        ExtractPolicy::allowing_degraded(),
    )
    .await
    .expect("an opted-in caller still gets the flat chain");
    assert_eq!(r.format, PdfFormat::Plain);
    assert!(
        !r.completeness.complete,
        "opting in does not make degraded output complete"
    );
    assert!(matches!(
        r.source,
        PdfTextSource::LiveExtract | PdfTextSource::PdftotextFallback
    ));
}

#[tokio::test]
async fn plain_is_never_gated() {
    // `plain` is a caller deliberately choosing flat output — a different
    // thing from tolerating a degraded substitute — and must keep working
    // with the layout route dead and no opt-in.
    let dir = tempfile::tempdir().unwrap();
    let r = extract(
        &fixture("hello.pdf"),
        dir.path(),
        &configured_but_dead(),
        ExtractPolicy::plain(),
    )
    .await
    .expect("plain output was asked for on purpose");
    assert_eq!(r.format, PdfFormat::Plain);
}

#[tokio::test]
async fn the_extracted_text_carries_no_server_added_marker() {
    // The text is the artefact downstream fact-checking treats as
    // authoritative. A warning banner inside it would put sentences into the
    // arbiter that are not in the document — manufacturing exactly the kind of
    // false content this work exists to prevent. Degradation is reported
    // alongside the text, never inside it.
    let dir = tempfile::tempdir().unwrap();
    for (engines, policy) in [
        (no_route(), ExtractPolicy::default()),
        (configured_but_dead(), ExtractPolicy::allowing_degraded()),
        (configured_but_dead(), ExtractPolicy::plain()),
    ] {
        let r = extract(&fixture("hello.pdf"), dir.path(), &engines, policy)
            .await
            .unwrap();
        for banned in [
            "degraded",
            "WARNING",
            "flat-text engine",
            "cannot detect",
            "incomplete",
        ] {
            assert!(
                !r.text.contains(banned),
                "server-added {banned:?} leaked into the document text: {:?}",
                r.text
            );
        }
        // ...while the report says so plainly.
        assert!(!r.completeness.complete);
    }
}

#[tokio::test]
async fn a_stored_derivative_is_served_even_when_the_route_is_dead() {
    // A store hit is layout-faithful by construction, so it is not subject to
    // the gate: a cold service must not make an already-extracted document
    // unreadable.
    let f = fixtures::build_fixture::build();
    std::fs::copy(
        fixture("tables.pdf"),
        f.storage_dir().join("AAAA0001").join("paper.pdf"),
    )
    .unwrap();
    let _ = std::fs::remove_file(f.storage_dir().join("AAAA0001").join(".zotero-ft-cache"));
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let store = DerivativeStore::new(tmp.path().join("derivatives"));
    let pdf = f.storage_dir().join("AAAA0001").join("paper.pdf");
    let hash = DerivativeStore::content_hash(&pdf).await.unwrap();
    let meta = DerivativeMeta {
        attachment_key: "AAAA0001".into(),
        source_hash: hash.clone(),
        profile: EXTRACTION_PROFILE.into(),
        engine: PdfTextSource::Docling,
        format: PdfFormat::Markdown,
        page_anchors: false,
        character_count: 21,
        completeness: Completeness::flat_text(PdfTextSource::Docling),
        windows: vec![(1, 1)],
        built_at: String::new(),
    };
    store
        .put("AAAA0001", &hash, "| a | b |\n| 1 | 2 |\n", &meta)
        .await
        .unwrap();

    let r = get_pdf_text_stored(
        &pool,
        "AAAA0001",
        1,
        &f.storage_dir(),
        &configured_but_dead(),
        &store,
        ExtractPolicy::default(),
        None,
        false,
    )
    .await
    .expect("a stored derivative must survive a cold layout route");
    assert_eq!(r.served_from, ServedFrom::Store);
    assert!(r.text.contains("| a | b |"));
}

#[tokio::test]
async fn without_a_stored_derivative_a_dead_route_refuses_through_the_tool_path() {
    let f = fixtures::build_fixture::build();
    std::fs::copy(
        fixture("tables.pdf"),
        f.storage_dir().join("AAAA0001").join("paper.pdf"),
    )
    .unwrap();
    let _ = std::fs::remove_file(f.storage_dir().join("AAAA0001").join(".zotero-ft-cache"));
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let store = DerivativeStore::new(tmp.path().join("derivatives"));

    let err = get_pdf_text_stored(
        &pool,
        "AAAA0001",
        1,
        &f.storage_dir(),
        &configured_but_dead(),
        &store,
        ExtractPolicy::default(),
        None,
        false,
    )
    .await
    .expect_err("nothing stored and the route down: refuse, do not substitute");
    assert!(
        matches!(err, Error::LayoutRouteUnavailable { .. }),
        "got {err}"
    );
}

#[tokio::test]
async fn a_window_walk_aborts_on_the_first_refusal() {
    // Building a derivative for a document over the page cap walks windows. If
    // the layout route is down, the FIRST window must abort the walk: a stream
    // with an unmarked hole — some windows layout-faithful, some flat — is
    // worse than no stream, and a partial assembly must never be stored as
    // though it were the document.
    use zotero_mcp::core::pdf::build_whole_document;
    let dir = tempfile::tempdir().unwrap();
    let err = build_whole_document(
        &fixture("large.pdf"),
        dir.path(),
        &configured_but_dead(),
        ExtractPolicy::default(),
        20,
    )
    .await
    .expect_err("a dead layout route must abort the build, not half-fill it");

    match err {
        Error::DerivativeIncomplete { from, to, total, detail, .. } => {
            assert_eq!((from, to), (1, 20), "must abort on the FIRST window");
            assert_eq!(total, 60);
            assert!(
                detail.contains("layout-aware extraction route"),
                "the underlying cause must survive into the message: {detail}"
            );
        }
        other => panic!("expected DerivativeIncomplete, got {other}"),
    }
}
