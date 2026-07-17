//! Live integration tests for the layout-aware (Docling) extraction route.
//!
//! These hit a real docling-serve instance. The endpoint comes from the
//! `DOCLING_URL` environment variable; the tests skip loudly ONLY when it
//! is unset or the service is unreachable (portability guard for hosts/CI
//! without the service). With `DOCLING_URL` exported and the service up,
//! they run for real and must pass.

use std::path::PathBuf;
use std::time::Duration;
use zotero_mcp::core::config::ZoteroConfig;
use zotero_mcp::core::pdf::{
    extract, truncate_to_first_pages, Completeness, DoclingEngine, PdfEngines, PdfFormat,
    PdfTextSource,
};

/// Endpoint from `DOCLING_URL`, or `None` to signal "skip on this host".
fn docling_url() -> Option<String> {
    std::env::var("DOCLING_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

fn hello_pdf() -> PathBuf {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.pdf");
    assert!(
        fixture.exists(),
        "hello.pdf fixture missing — run tests/fixtures/gen_pdfs.py"
    );
    fixture
}

fn scanned_pdf() -> PathBuf {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scanned.pdf");
    assert!(
        fixture.exists(),
        "scanned.pdf fixture missing — run tests/fixtures/gen_pdfs.py"
    );
    fixture
}

/// Full orchestrator run over the live stack: real `ocrmypdf` OCRs the
/// image-only fixture into a temp copy, real docling-serve converts that
/// copy, and the result is labelled `OcrThenDocling` with `ocr_pages`
/// populated. The original fixture must be byte-identical afterward.
#[tokio::test]
async fn ocr_prestep_recovers_scanned_pdf_live() {
    let Some(url) = docling_url() else {
        eprintln!("DOCLING_URL not set; skipping live OCR pre-step integration test");
        return;
    };
    let probe = DoclingEngine::new(url, Duration::from_secs(300), Duration::from_secs(5));
    if !probe.healthy().await {
        eprintln!("Docling service at DOCLING_URL unreachable; skipping live OCR pre-step test");
        return;
    }
    let Ok(ocrmypdf) = which::which("ocrmypdf") else {
        eprintln!("ocrmypdf not on PATH; skipping live OCR pre-step test");
        return;
    };

    let scanned = scanned_pdf();
    let original_bytes = std::fs::read(&scanned).unwrap();

    // PdfEngines::build picks the endpoint up from DOCLING_URL; pin the
    // discovered ocrmypdf explicitly so the test asserts the config path.
    let cfg = ZoteroConfig {
        ocrmypdf_path: Some(ocrmypdf.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let engines = PdfEngines::build(&cfg);
    let storage_item_dir = tempfile::TempDir::new().unwrap();

    let r = extract(&scanned, storage_item_dir.path(), &engines, false)
        .await
        .expect("live OCR + Docling extraction succeeded");

    assert_eq!(r.source, PdfTextSource::OcrThenDocling);
    assert!(r.page_anchors);
    assert!(
        r.text.contains("Scanned quarterly report"),
        "OCR-recovered text missing, got: {:?}",
        r.text
    );
    assert_eq!(r.completeness.engine, PdfTextSource::OcrThenDocling);
    assert_eq!(r.completeness.ocr_pages, vec![1]);
    // The original scan is never mutated.
    assert_eq!(
        std::fs::read(&scanned).unwrap(),
        original_bytes,
        "scanned.pdf must be byte-identical after extraction"
    );
}

/// `plain: true` must force the old flat-text path even when a live,
/// healthy Docling endpoint is configured: the result comes from the
/// flat chain with `format: Plain`, no page anchors, and an incomplete
/// (flat-text) completeness report.
#[tokio::test]
async fn plain_option_forces_flat_path_despite_live_docling() {
    let Some(url) = docling_url() else {
        eprintln!("DOCLING_URL not set; skipping live plain-option integration test");
        return;
    };
    let probe = DoclingEngine::new(url, Duration::from_secs(120), Duration::from_secs(5));
    if !probe.healthy().await {
        eprintln!("Docling service at DOCLING_URL unreachable; skipping live plain-option test");
        return;
    }

    // PdfEngines::build picks the live endpoint up from DOCLING_URL, so
    // the Docling route is configured and healthy — plain must skip it.
    let engines = PdfEngines::build(&ZoteroConfig::default());
    let storage_item_dir = tempfile::TempDir::new().unwrap();

    let r = extract(&hello_pdf(), storage_item_dir.path(), &engines, true)
        .await
        .expect("plain flat-path extraction succeeded");

    assert_eq!(r.format, PdfFormat::Plain);
    assert_eq!(r.source, PdfTextSource::LiveExtract);
    assert!(!r.page_anchors);
    assert!(r.text.contains("Hello fallback world"), "got: {:?}", r.text);
    assert!(!r.completeness.complete);
    assert!(r
        .completeness
        .notes
        .iter()
        .any(|n| n.contains("flat-text engine")));
}

#[tokio::test]
async fn docling_engine_extracts_hello_pdf_live() {
    let Some(url) = docling_url() else {
        eprintln!("DOCLING_URL not set; skipping live Docling integration test");
        return;
    };
    let eng = DoclingEngine::new(url, Duration::from_secs(120), Duration::from_secs(5));
    if !eng.healthy().await {
        eprintln!("Docling service at DOCLING_URL unreachable; skipping live integration test");
        return;
    }

    let ext = eng
        .extract_markdown(&hello_pdf())
        .await
        .expect("live docling extraction succeeded");
    let md = &ext.markdown;

    assert!(
        md.starts_with("--- p.1 ---"),
        "expected page-anchored markdown, got: {:?}",
        &md[..md.len().min(120)]
    );
    assert!(md.contains("Hello fallback world"), "got: {:?}", md);

    let c = Completeness::from_page_anchored_markdown(
        md,
        PdfTextSource::Docling,
        ext.formula_enrichment,
        Vec::new(),
    );
    assert_eq!(c.engine, PdfTextSource::Docling);
    assert_eq!(c.pages, 1);
    assert!(c.undecoded_formulas.is_empty());
    assert!(c.untranscribed_images.is_empty());
    if ext.formula_enrichment {
        assert!(c.complete);
    } else {
        // The deployment could not run formula enrichment: the report must
        // declare the gap rather than claim completeness.
        assert!(!c.complete);
        assert!(c.notes.iter().any(|n| n.contains("formula enrichment")));
    }
}

fn fixture(name: &str) -> PathBuf {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    assert!(
        fixture.exists(),
        "{name} fixture missing — run tests/fixtures/gen_pdfs.py"
    );
    fixture
}

/// Build the engine bundle from `DOCLING_URL` and skip loudly when the
/// service is not reachable (portability guard for hosts/CI without it).
async fn live_engines(test_name: &str) -> Option<PdfEngines> {
    let Some(url) = docling_url() else {
        eprintln!("DOCLING_URL not set; skipping {test_name}");
        return None;
    };
    let probe = DoclingEngine::new(url, Duration::from_secs(300), Duration::from_secs(5));
    if !probe.healthy().await {
        eprintln!("Docling service at DOCLING_URL unreachable; skipping {test_name}");
        return None;
    }
    Some(PdfEngines::build(&ZoteroConfig::default()))
}

/// Golden set: the table-heavy report must come back as real markdown
/// tables — header row, separator row, and data rows with the cell values
/// on their own rows — not interleaved number-soup.
#[tokio::test]
async fn tables_extract_as_markdown_tables_live() {
    let Some(engines) = live_engines("live markdown-tables test").await else {
        return;
    };
    let storage_item_dir = tempfile::TempDir::new().unwrap();

    let r = extract(
        &fixture("tables.pdf"),
        storage_item_dir.path(),
        &engines,
        false,
    )
    .await
    .expect("live Docling extraction of tables.pdf succeeded");

    assert_eq!(r.source, PdfTextSource::Docling);
    assert_eq!(r.format, PdfFormat::Markdown);
    assert!(r.page_anchors);

    let lines: Vec<&str> = r.text.lines().map(str::trim).collect();
    let table_row = |needles: &[&str]| {
        lines.iter().any(|l| {
            l.starts_with('|') && l.ends_with('|') && needles.iter().all(|n| l.contains(n))
        })
    };
    // First table: header + separator + intact data rows.
    assert!(
        table_row(&["Region", "Q1", "Q4"]),
        "missing region header row"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with('|') && l.contains("---")),
        "missing markdown table separator row"
    );
    assert!(
        table_row(&["North", "1214", "1180", "1105", "1298"]),
        "North row not intact, text:\n{}",
        r.text
    );
    assert!(
        table_row(&["West", "874", "861", "902", "890"]),
        "West row not intact"
    );
    // Second table survives as its own table.
    assert!(
        table_row(&["Technology", "Load factor"]),
        "missing technology header row"
    );
    assert!(
        table_row(&["Biomass", "640", "4483", "0.80"]),
        "Biomass row not intact"
    );

    // Clean prose-and-tables page: nothing undecoded, report complete.
    assert_eq!(r.completeness.engine, PdfTextSource::Docling);
    assert!(r.completeness.undecoded_formulas.is_empty());
    assert!(r.completeness.untranscribed_images.is_empty());
    assert!(
        r.completeness.complete,
        "expected a complete report, got: {:?}",
        r.completeness
    );
}

/// Golden set: the display equation decodes to LaTeX (formula enrichment
/// on) and the completeness report shows zero undecoded formulas for the
/// page.
#[tokio::test]
async fn equation_decodes_to_latex_live() {
    let Some(engines) = live_engines("live equation-LaTeX test").await else {
        return;
    };
    let storage_item_dir = tempfile::TempDir::new().unwrap();

    let r = extract(
        &fixture("equation.pdf"),
        storage_item_dir.path(),
        &engines,
        false,
    )
    .await
    .expect("live Docling extraction of equation.pdf succeeded");

    assert_eq!(r.source, PdfTextSource::Docling);
    assert_eq!(r.format, PdfFormat::Markdown);
    assert!(
        r.text.contains("$$"),
        "no LaTeX display-math block in output:\n{}",
        r.text
    );
    assert!(
        r.text.contains("\\frac") || r.text.contains("\\sqrt"),
        "decoded LaTeX missing the fraction/radical:\n{}",
        r.text
    );
    assert!(
        !r.text.contains("<!-- formula-not-decoded -->"),
        "formula left undecoded:\n{}",
        r.text
    );

    assert_eq!(r.completeness.pages, 1);
    assert!(
        r.completeness.undecoded_formulas.is_empty(),
        "expected zero undecoded formulas, got: {:?}",
        r.completeness.undecoded_formulas
    );
    assert!(
        r.completeness.complete,
        "expected a complete report, got: {:?}",
        r.completeness
    );
}

/// Golden set: the two-column paper keeps reading order — each column's
/// distinctive sentence survives contiguously instead of being interleaved
/// line-by-line across the column gap.
#[tokio::test]
async fn two_column_reading_order_live() {
    let Some(engines) = live_engines("live two-column reading-order test").await else {
        return;
    };
    let storage_item_dir = tempfile::TempDir::new().unwrap();

    let r = extract(
        &fixture("twocolumn.pdf"),
        storage_item_dir.path(),
        &engines,
        false,
    )
    .await
    .expect("live Docling extraction of twocolumn.pdf succeeded");

    assert_eq!(r.source, PdfTextSource::Docling);
    assert_eq!(r.format, PdfFormat::Markdown);
    let left_sentence = "The aardvark population of the western valley increased steadily \
                         throughout the survey period, defying every published model.";
    let right_sentence = "Meanwhile the barnacle colonies of the eastern shoreline declined \
                          sharply, a collapse the tidal records had clearly foreshadowed.";
    assert!(
        r.text.contains(left_sentence),
        "left-column sentence not contiguous (columns interleaved?):\n{}",
        r.text
    );
    assert!(
        r.text.contains(right_sentence),
        "right-column sentence not contiguous (columns interleaved?):\n{}",
        r.text
    );
}

/// Golden set: a genuine three-page document assembles one `--- p.N ---`
/// anchor per page in order, and page-boundary truncation keeps the first
/// N pages whole while the completeness report describes only the
/// retained pages.
#[tokio::test]
async fn multipage_assembles_anchors_and_truncates_on_page_boundaries_live() {
    let Some(engines) = live_engines("live multi-page anchors/truncation test").await else {
        return;
    };
    let storage_item_dir = tempfile::TempDir::new().unwrap();

    let full = extract(
        &fixture("multipage.pdf"),
        storage_item_dir.path(),
        &engines,
        false,
    )
    .await
    .expect("live Docling extraction of multipage.pdf succeeded");

    assert_eq!(full.source, PdfTextSource::Docling);
    assert!(full.page_anchors);
    assert_eq!(full.completeness.pages, 3, "fixture has exactly 3 pages");
    assert_eq!(full.completeness.per_page_chars.len(), 3);
    // One anchor per page, in order, each page's sentence under its anchor.
    let p1 = full.text.find("--- p.1 ---").expect("p.1 anchor");
    let p2 = full.text.find("--- p.2 ---").expect("p.2 anchor");
    let p3 = full.text.find("--- p.3 ---").expect("p.3 anchor");
    assert!(p1 < p2 && p2 < p3, "anchors out of order");
    let albatross = full.text.find("albatross").expect("page-one sentence");
    let badger = full.text.find("badger").expect("page-two sentence");
    let capybara = full.text.find("capybara").expect("page-three sentence");
    assert!(
        p1 < albatross && albatross < p2,
        "page-one content misplaced"
    );
    assert!(p2 < badger && badger < p3, "page-two content misplaced");
    assert!(p3 < capybara, "page-three content misplaced");

    // Truncate to the first 2 pages: whole pages retained, p.3 gone,
    // and the report adjusted to the retained pages.
    let full_per_page = full.completeness.per_page_chars.clone();
    let complete_before = full.completeness.complete;
    let r = truncate_to_first_pages(full, 2);

    assert!(r.text.contains("--- p.1 ---"));
    assert!(r.text.contains("albatross"));
    assert!(r.text.contains("--- p.2 ---"));
    assert!(r.text.contains("badger"), "page 2 must be kept whole");
    assert!(
        !r.text.contains("--- p.3 ---"),
        "page 3 anchor must be dropped"
    );
    assert!(
        !r.text.contains("capybara"),
        "page 3 content must be dropped"
    );
    assert!(r.text.contains("[... truncated: first 2 of 3 pages ...]"));

    assert_eq!(r.completeness.pages, 2);
    assert_eq!(r.completeness.per_page_chars, full_per_page[..2].to_vec());
    assert!(
        r.completeness.low_text_pages.iter().all(|p| *p <= 2),
        "report must not reference dropped pages"
    );
    assert_eq!(r.completeness.complete, complete_before);
    assert!(r
        .completeness
        .notes
        .iter()
        .any(|n| n.contains("truncated to the first 2 of 3 pages")));
    assert_eq!(r.character_count, r.text.chars().count());
}

/// Spec scenario "Enrichment unavailable", live: when the deployed
/// docling-serve build rejects `do_formula_enrichment=true`, the retry
/// without enrichment must preserve the formula region as an explicit
/// undecoded marker and record its page in `undecoded_formulas`. When the
/// deployment *can* run enrichment, that path cannot be forced live and
/// this test asserts the decoded outcome instead (and says so).
#[tokio::test]
async fn enrichment_unavailable_live_declares_undecoded_formula() {
    let Some(engines) = live_engines("live enrichment-unavailable test").await else {
        return;
    };
    let storage_item_dir = tempfile::TempDir::new().unwrap();

    let r = extract(
        &fixture("equation.pdf"),
        storage_item_dir.path(),
        &engines,
        false,
    )
    .await
    .expect("live Docling extraction of equation.pdf succeeded");

    let enrichment_unavailable = r
        .completeness
        .notes
        .iter()
        .any(|n| n.contains("formula enrichment"));
    if enrichment_unavailable {
        // The live build rejected enrichment: the formula region must be
        // an explicit undecoded marker with its page recorded — never
        // silently omitted.
        assert!(
            r.text.contains("<!-- formula-not-decoded -->"),
            "undecoded formula marker missing:\n{}",
            r.text
        );
        assert_eq!(r.completeness.undecoded_formulas, vec![1]);
        assert!(!r.completeness.complete);
    } else {
        eprintln!(
            "live docling accepted formula enrichment; unavailable path \
             covered by the stubbed end-to-end test instead"
        );
        assert!(
            r.text.contains("$$"),
            "enrichment on but no decoded LaTeX:\n{}",
            r.text
        );
        assert!(r.completeness.undecoded_formulas.is_empty());
    }
}

/// Image-only PDF with the Docling route unreachable: the
/// nothing-extractable invariant must hold regardless of Docling health.
/// With `ocrmypdf` on the host, the OCR rescue runs the flat-text chain on
/// an OCR'd temp copy; the result is EITHER recovered text (above the
/// minimum floor) OR the loud nothing-extractable error naming the OCR
/// remedy — never empty/near-empty text as success. The original scan is
/// byte-identical afterward. Needs no live Docling; skips loudly only
/// without ocrmypdf.
#[tokio::test]
async fn image_only_pdf_with_docling_unreachable_never_returns_empty_success() {
    if which::which("ocrmypdf").is_err() {
        eprintln!("ocrmypdf not on PATH; skipping Docling-down OCR rescue test");
        return;
    }
    let scanned = fixture("scanned.pdf");
    let original_bytes = std::fs::read(&scanned).unwrap();

    let dead = DoclingEngine::new(
        "http://127.0.0.1:9".into(),
        Duration::from_secs(5),
        Duration::from_secs(2),
    );
    let engines =
        PdfEngines::build(&ZoteroConfig::default()).with_docling(Some(std::sync::Arc::new(dead)));
    let storage_item_dir = tempfile::TempDir::new().unwrap();

    match extract(&scanned, storage_item_dir.path(), &engines, false).await {
        Ok(r) => {
            let non_ws = r.text.chars().filter(|c| !c.is_whitespace()).count();
            assert!(
                non_ws >= 10,
                "success with near-empty text ({non_ws} chars) violates the floor: {:?}",
                r.text
            );
            assert!(
                r.completeness.notes.iter().any(|n| n.contains("OCR")),
                "OCR-rescued result must declare the OCR pre-step, got: {:?}",
                r.completeness.notes
            );
            assert!(!r.completeness.complete);
        }
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("ocrmypdf") || msg.contains("OCR"),
                "loud error must name the OCR remedy, got: {msg}"
            );
        }
    }
    assert_eq!(
        std::fs::read(&scanned).unwrap(),
        original_bytes,
        "scanned.pdf must be byte-identical after extraction"
    );
}

/// `plain=true` is the fast flat path and must NEVER run the OCR rescue,
/// even on an image-only PDF with ocrmypdf installed. The scan therefore
/// fails loudly (flat engines read nothing); if it somehow carried a text
/// layer it would return a flat source — but never `OcrThenDocling`.
#[tokio::test]
async fn plain_true_never_runs_ocr_rescue_on_scan() {
    if which::which("ocrmypdf").is_err() {
        eprintln!("ocrmypdf not on PATH; skipping plain-no-OCR test");
        return;
    }
    let scanned = fixture("scanned.pdf");
    let dead = DoclingEngine::new(
        "http://127.0.0.1:9".into(),
        Duration::from_secs(5),
        Duration::from_secs(2),
    );
    let engines =
        PdfEngines::build(&ZoteroConfig::default()).with_docling(Some(std::sync::Arc::new(dead)));
    let storage_item_dir = tempfile::TempDir::new().unwrap();

    match extract(&scanned, storage_item_dir.path(), &engines, true).await {
        Ok(r) => assert_ne!(
            r.source,
            PdfTextSource::OcrThenDocling,
            "plain=true must never OCR, but produced an OCR-sourced result"
        ),
        // A loud failure is the expected outcome for a scan on the flat path.
        Err(_) => {}
    }
}

/// When the Docling endpoint is dead (connection refused), the
/// orchestrator falls through to the flat-text chain and the result is
/// declared incomplete with the flat-text note — degraded, never silent.
/// Needs no live service; runs everywhere.
#[tokio::test]
async fn dead_docling_url_falls_back_flat_and_reports_incomplete() {
    // Port 9 (discard) refuses immediately on this host; the override
    // wins over any DOCLING_URL in the environment.
    let dead = DoclingEngine::new(
        "http://127.0.0.1:9".into(),
        Duration::from_secs(5),
        Duration::from_secs(2),
    );
    let engines =
        PdfEngines::build(&ZoteroConfig::default()).with_docling(Some(std::sync::Arc::new(dead)));
    let storage_item_dir = tempfile::TempDir::new().unwrap();

    let r = extract(
        &fixture("hello.pdf"),
        storage_item_dir.path(),
        &engines,
        false,
    )
    .await
    .expect("flat-text fallback extraction succeeded");

    assert_eq!(r.source, PdfTextSource::LiveExtract);
    assert_eq!(r.format, PdfFormat::Plain);
    assert!(!r.page_anchors);
    assert!(r.text.contains("Hello fallback world"), "got: {:?}", r.text);
    assert!(
        !r.completeness.complete,
        "flat-text result must never be complete"
    );
    assert_eq!(r.completeness.engine, PdfTextSource::LiveExtract);
    assert!(
        r.completeness
            .notes
            .iter()
            .any(|n| n.contains("flat-text engine")),
        "missing flat-text note, got: {:?}",
        r.completeness.notes
    );
}
