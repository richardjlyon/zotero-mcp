//! Durable derivative store: window-walk assembly, storage gating, and
//! serving without re-extraction.
//!
//! Everything here runs against committed fixtures on any host. The tests
//! that need the layout service skip loudly; the rest — assembly, storage
//! gating, staleness, serving — run on the flat-text chain and still assert
//! real behaviour, because those are the parts that must not depend on a GPU
//! being awake.

mod fixtures;

use std::path::PathBuf;
use std::time::Duration;
use zotero_mcp::core::pdf::get_pdf_text_stored;
use zotero_mcp::core::reader::pool::ReadOnlyPool;
use zotero_mcp::core::config::{Config, ZoteroConfig};
use zotero_mcp::core::derivatives::{DerivativeStatus, DerivativeStore, EXTRACTION_PROFILE};
use zotero_mcp::core::error::Error;
use zotero_mcp::core::pdf::{
    build_whole_document, extract_windowed, is_layout_faithful, DoclingEngine, PdfEngines,
    PdfFormat, PdfTextSource, ServedFrom, DERIVATIVE_WINDOW_PAGES,
};

/// Page count of `large.pdf`, mirrored from `gen_pdfs.py`. Above the default
/// 50-page whole-document cap on purpose: a derivative for it can only be
/// built by walking windows.
const LARGE_PDF_PAGES: u32 = 60;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    assert!(
        p.exists(),
        "{name} fixture missing — run tests/fixtures/gen_pdfs.py"
    );
    p
}

/// Engines with no layout route: the flat-text chain only. This is the CI
/// posture and the posture of any host without the GPU box.
fn flat_engines() -> PdfEngines {
    // `DOCLING_URL` in the environment overrides config, so a default build
    // on a developer machine quietly HAS the layout route. Strip it
    // explicitly: these tests exist to prove the CI posture works.
    PdfEngines::build(&ZoteroConfig::default()).with_docling(None)
}

/// Engines with the layout route, or `None` to skip on this host.
async fn layout_engines() -> Option<PdfEngines> {
    let url = std::env::var("DOCLING_URL").ok().filter(|s| !s.is_empty())?;
    let probe = DoclingEngine::new(
        url.clone(),
        Duration::from_secs(300),
        Duration::from_secs(5),
    );
    if !probe.healthy().await {
        return None;
    }
    let cfg = ZoteroConfig {
        docling_url: Some(url),
        ..Default::default()
    };
    Some(PdfEngines::build(&cfg))
}

#[tokio::test]
async fn whole_document_tool_call_over_the_cap_is_still_refused() {
    // The store exists *because* this refusal is correct and must stay.
    let tmp = tempfile::tempdir().unwrap();
    let err = extract_windowed(&fixture("large.pdf"), tmp.path(), &flat_engines(), false, None)
        .await
        .expect_err("a 60-page whole-document request must be refused");
    match err {
        Error::PdfDocumentTooLarge { pages, .. } => assert_eq!(pages, LARGE_PDF_PAGES),
        other => panic!("expected PdfDocumentTooLarge, got {other}"),
    }
}

#[tokio::test]
async fn window_walk_covers_every_page_of_a_document_over_the_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let (result, windows) = build_whole_document(
        &fixture("large.pdf"),
        tmp.path(),
        &flat_engines(),
        DERIVATIVE_WINDOW_PAGES,
    )
    .await
    .expect("a document over the cap must still build whole, by walking windows");

    assert_eq!(windows, vec![(1, 20), (21, 40), (41, 60)]);
    assert_eq!(result.completeness.total_pages, LARGE_PDF_PAGES);

    // Every page contributed exactly once, in order: the marker is unique per
    // page in the fixture, so this catches dropped, duplicated and reordered
    // windows in one assertion.
    let mut last_at = 0usize;
    for n in 1..=LARGE_PDF_PAGES {
        let marker = format!("Pagemarker {n} of {LARGE_PDF_PAGES}");
        let hits: Vec<usize> = result.text.match_indices(&marker).map(|(i, _)| i).collect();
        assert_eq!(hits.len(), 1, "page {n} must appear exactly once: {hits:?}");
        assert!(hits[0] > last_at, "page {n} out of order in the assembly");
        last_at = hits[0];
    }
}

#[tokio::test]
async fn assembly_matches_the_windows_it_was_built_from() {
    let tmp = tempfile::tempdir().unwrap();
    let engines = flat_engines();
    let (assembled, windows) =
        build_whole_document(&fixture("large.pdf"), tmp.path(), &engines, 20)
            .await
            .unwrap();

    let mut manual = String::new();
    for (from, to) in &windows {
        let w = extract_windowed(
            &fixture("large.pdf"),
            tmp.path(),
            &engines,
            false,
            Some((*from, *to)),
        )
        .await
        .unwrap();
        if !manual.is_empty() {
            manual.push_str("\n\n");
        }
        manual.push_str(w.text.trim_end());
    }
    manual.push('\n');
    assert_eq!(
        assembled.text, manual,
        "the assembly must be exactly the concatenation of its windows"
    );
}

#[tokio::test]
async fn a_small_document_builds_in_one_window() {
    let tmp = tempfile::tempdir().unwrap();
    let (result, windows) =
        build_whole_document(&fixture("multipage.pdf"), tmp.path(), &flat_engines(), 20)
            .await
            .unwrap();
    assert_eq!(windows, vec![(1, 3)], "under the cap: no walk needed");
    assert_eq!(result.completeness.total_pages, 3);
    assert!(result.text.contains("albatross"));
    assert!(result.text.contains("capybara"));
}

#[tokio::test]
async fn flat_text_output_is_never_layout_faithful() {
    let tmp = tempfile::tempdir().unwrap();
    let r = extract_windowed(
        &fixture("tables.pdf"),
        tmp.path(),
        &flat_engines(),
        false,
        None,
    )
    .await
    .unwrap();
    assert!(
        !is_layout_faithful(&r),
        "flat-text output must never qualify for storage, whatever its length"
    );
    assert_eq!(r.format, PdfFormat::Plain);
    assert!(!r.completeness.complete);
}

#[tokio::test]
async fn character_count_does_not_qualify_output_for_storage() {
    // The trap this guards: on the incident that motivated the store, the
    // flat run produced MORE characters than the layout run on the same page
    // (3,496 vs 3,299) with every table gone. Length is never the criterion.
    let Some(layout) = layout_engines().await else {
        eprintln!("no layout route on this host; skipping the fidelity comparison");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let flat = extract_windowed(
        &fixture("tables.pdf"),
        tmp.path(),
        &flat_engines(),
        false,
        None,
    )
    .await
    .unwrap();
    let md = extract_windowed(&fixture("tables.pdf"), tmp.path(), &layout, false, None)
        .await
        .unwrap();

    assert!(is_layout_faithful(&md), "layout route output must qualify");
    assert!(!is_layout_faithful(&flat), "flat output must not");

    // The distinguishing property is table structure, not size.
    let md_rows = md.text.matches('|').count();
    let flat_rows = flat.text.matches('|').count();
    assert!(
        md_rows > flat_rows,
        "layout output must carry table pipes the flat output lacks ({md_rows} vs {flat_rows})"
    );
}

#[tokio::test]
async fn store_serves_the_second_read_without_re_extracting() {
    let Some(layout) = layout_engines().await else {
        eprintln!("no layout route on this host; skipping store-serving test");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let store = DerivativeStore::new(tmp.path().join("derivatives"));
    let pdf = fixture("tables.pdf");
    let hash = DerivativeStore::content_hash(&pdf).await.unwrap();

    assert_eq!(
        store.status("AAAA0001", &hash).await,
        DerivativeStatus::Absent
    );

    let (built, windows) = build_whole_document(&pdf, tmp.path(), &layout, 20)
        .await
        .unwrap();
    assert!(is_layout_faithful(&built));
    let meta = zotero_mcp::core::derivatives::DerivativeMeta {
        attachment_key: "AAAA0001".into(),
        source_hash: hash.clone(),
        profile: EXTRACTION_PROFILE.into(),
        engine: built.source,
        format: built.format,
        page_anchors: built.page_anchors,
        character_count: built.character_count,
        completeness: built.completeness.clone(),
        windows,
        built_at: String::new(),
    };
    store
        .put("AAAA0001", &hash, &built.text, &meta)
        .await
        .unwrap();

    // Now break the layout route entirely. A store hit must still serve.
    let hit = store.get("AAAA0001", &hash).await.expect("stored");
    assert_eq!(hit.text, built.text);
    assert_eq!(hit.meta.engine, built.source);
    assert_eq!(
        store.status("AAAA0001", &hash).await,
        DerivativeStatus::Present
    );
}

#[tokio::test]
async fn a_replaced_pdf_is_never_served_from_the_old_derivative() {
    let tmp = tempfile::tempdir().unwrap();
    let store = DerivativeStore::new(tmp.path().join("derivatives"));
    let a = DerivativeStore::content_hash(&fixture("hello.pdf")).await.unwrap();
    let b = DerivativeStore::content_hash(&fixture("tables.pdf"))
        .await
        .unwrap();
    assert_ne!(a, b);

    let meta = zotero_mcp::core::derivatives::DerivativeMeta {
        attachment_key: "AAAA0001".into(),
        source_hash: a.clone(),
        profile: EXTRACTION_PROFILE.into(),
        engine: PdfTextSource::Docling,
        format: PdfFormat::Markdown,
        page_anchors: true,
        character_count: 5,
        completeness: zotero_mcp::core::pdf::Completeness::flat_text(PdfTextSource::Docling),
        windows: vec![(1, 1)],
        built_at: String::new(),
    };
    store.put("AAAA0001", &a, "hello", &meta).await.unwrap();

    assert!(
        store.get("AAAA0001", &b).await.is_none(),
        "different PDF content must not resolve to the old derivative"
    );
}

#[tokio::test]
async fn store_root_is_outside_the_zotero_tree() {
    // Guards the decision, not just the implementation: if someone points the
    // store back inside the Zotero data directory, the VM (read-only mirror)
    // stops working and derivatives take an unbounded sync interval to appear.
    let cfg = Config::default();
    let derivatives = cfg.resolved_derivatives_dir();
    let zotero = cfg.storage_dir();
    assert!(
        !derivatives.starts_with(&zotero),
        "derivatives dir {derivatives:?} must not live under the Zotero storage dir {zotero:?}"
    );
}

#[tokio::test]
async fn served_from_defaults_to_fresh_on_a_real_extraction() {
    let tmp = tempfile::tempdir().unwrap();
    let r = extract_windowed(
        &fixture("hello.pdf"),
        tmp.path(),
        &flat_engines(),
        false,
        None,
    )
    .await
    .unwrap();
    assert_eq!(r.served_from, ServedFrom::Fresh);
    assert!(r.profile.is_none(), "a fresh run is on the current profile");
}

/// Stand up the SQLite fixture with a *real* PDF behind item AAAA0001, so the
/// full path — resolve attachment, hash it, consult the store, extract, store,
/// serve — runs end to end.
async fn fixture_with_pdf(name: &str) -> (fixtures::build_fixture::Fixture, ReadOnlyPool) {
    let f = fixtures::build_fixture::build();
    let dest = f.storage_dir().join("AAAA0001").join("paper.pdf");
    std::fs::copy(fixture(name), &dest).unwrap();
    // The Zotero flat cache would short-circuit the flat chain and confuse the
    // "what actually ran" assertions.
    let _ = std::fs::remove_file(f.storage_dir().join("AAAA0001").join(".zotero-ft-cache"));
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    (f, pool)
}

#[tokio::test]
async fn second_read_is_served_from_the_store_without_re_extracting() {
    let Some(layout) = layout_engines().await else {
        eprintln!("no layout route on this host; skipping end-to-end store-serving test");
        return;
    };
    let (f, pool) = fixture_with_pdf("tables.pdf").await;
    let tmp = tempfile::tempdir().unwrap();
    let store = DerivativeStore::new(tmp.path().join("derivatives"));

    let first = get_pdf_text_stored(
        &pool, "AAAA0001", 1, &f.storage_dir(), &layout, &store, false, None, false,
    )
    .await
    .unwrap();
    assert_eq!(first.served_from, ServedFrom::Fresh);
    assert!(is_layout_faithful(&first));

    // Break the layout route completely. A store hit must not need it.
    let dead = PdfEngines::build(&ZoteroConfig::default()).with_docling(None);
    let second = get_pdf_text_stored(
        &pool, "AAAA0001", 1, &f.storage_dir(), &dead, &store, false, None, false,
    )
    .await
    .unwrap();
    assert_eq!(
        second.served_from,
        ServedFrom::Store,
        "the second read must come from the store, not the engines"
    );
    assert_eq!(second.text, first.text, "stored bytes must round-trip");
    assert_eq!(second.source, first.source, "provenance is of the stored bytes");
    assert_eq!(second.profile.as_deref(), Some(EXTRACTION_PROFILE));
}

#[tokio::test]
async fn a_window_is_served_from_a_stored_whole_document() {
    let Some(layout) = layout_engines().await else {
        eprintln!("no layout route on this host; skipping windowed store-serving test");
        return;
    };
    let (f, pool) = fixture_with_pdf("multipage.pdf").await;
    let tmp = tempfile::tempdir().unwrap();
    let store = DerivativeStore::new(tmp.path().join("derivatives"));

    let whole = get_pdf_text_stored(
        &pool, "AAAA0001", 1, &f.storage_dir(), &layout, &store, false, None, false,
    )
    .await
    .unwrap();
    assert!(whole.page_anchors, "layout route anchors its pages");

    let dead = PdfEngines::build(&ZoteroConfig::default()).with_docling(None);
    let win = get_pdf_text_stored(
        &pool,
        "AAAA0001",
        1,
        &f.storage_dir(),
        &dead,
        &store,
        false,
        Some((2, 2)),
        false,
    )
    .await
    .unwrap();
    assert_eq!(win.served_from, ServedFrom::Store, "no extraction for a window we hold");
    assert!(win.text.contains("--- p.2 ---"));
    assert!(win.text.contains("badger"), "page 2's sentence");
    assert!(!win.text.contains("albatross"), "page 1 must not leak in");
    assert!(!win.text.contains("capybara"), "page 3 must not leak in");
    assert_eq!(win.completeness.pages, 1);
    assert_eq!(
        win.completeness.total_pages, 3,
        "the window reports the whole document's page count"
    );
}

#[tokio::test]
async fn refresh_forces_a_fresh_extraction() {
    let Some(layout) = layout_engines().await else {
        eprintln!("no layout route on this host; skipping refresh test");
        return;
    };
    let (f, pool) = fixture_with_pdf("tables.pdf").await;
    let tmp = tempfile::tempdir().unwrap();
    let store = DerivativeStore::new(tmp.path().join("derivatives"));

    for _ in 0..2 {
        let _ = get_pdf_text_stored(
            &pool, "AAAA0001", 1, &f.storage_dir(), &layout, &store, false, None, false,
        )
        .await
        .unwrap();
    }
    let refreshed = get_pdf_text_stored(
        &pool, "AAAA0001", 1, &f.storage_dir(), &layout, &store, false, None, true,
    )
    .await
    .unwrap();
    assert_eq!(refreshed.served_from, ServedFrom::Fresh);
}

#[tokio::test]
async fn a_flat_text_run_stores_nothing_and_keeps_re_extracting() {
    // No layout route: the CI posture. Extraction still works, but nothing is
    // stored, because a flat artefact must never become the permanent one.
    let (f, pool) = fixture_with_pdf("tables.pdf").await;
    let tmp = tempfile::tempdir().unwrap();
    let store = DerivativeStore::new(tmp.path().join("derivatives"));
    let flat = flat_engines();

    let first = get_pdf_text_stored(
        &pool, "AAAA0001", 1, &f.storage_dir(), &flat, &store, false, None, false,
    )
    .await
    .unwrap();
    assert_eq!(first.served_from, ServedFrom::Fresh);
    assert!(!is_layout_faithful(&first));

    let pdf = f.storage_dir().join("AAAA0001").join("paper.pdf");
    let hash = DerivativeStore::content_hash(&pdf).await.unwrap();
    assert_eq!(
        store.status("AAAA0001", &hash).await,
        DerivativeStatus::Absent,
        "flat-text output must not be stored"
    );

    let second = get_pdf_text_stored(
        &pool, "AAAA0001", 1, &f.storage_dir(), &flat, &store, false, None, false,
    )
    .await
    .unwrap();
    assert_eq!(
        second.served_from,
        ServedFrom::Fresh,
        "with nothing stored, the second read extracts again — as it must"
    );
}

#[tokio::test]
async fn plain_bypasses_the_store_in_both_directions() {
    let (f, pool) = fixture_with_pdf("tables.pdf").await;
    let tmp = tempfile::tempdir().unwrap();
    let store = DerivativeStore::new(tmp.path().join("derivatives"));
    let engines = layout_engines().await.unwrap_or_else(flat_engines);

    let r = get_pdf_text_stored(
        &pool, "AAAA0001", 1, &f.storage_dir(), &engines, &store, true, None, false,
    )
    .await
    .unwrap();
    assert_eq!(r.format, PdfFormat::Plain, "plain means plain");
    let pdf = f.storage_dir().join("AAAA0001").join("paper.pdf");
    let hash = DerivativeStore::content_hash(&pdf).await.unwrap();
    assert_eq!(
        store.status("AAAA0001", &hash).await,
        DerivativeStatus::Absent,
        "a deliberate flat-output request must never populate the store"
    );
}
