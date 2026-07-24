## 1. Completeness report (build this first — it is the contract)

- [x] 1.1 Add a `Completeness` type (`complete`, `engine`, `pages`, `per_page_chars`,
      `undecoded_formulas`, `untranscribed_images`, `ocr_pages`, `low_text_pages`,
      `notes`) and thread it onto `PdfTextResult` alongside `format` and `page_anchors`.
- [x] 1.2 Unit-test the report derivation from a page-anchored markdown string containing
      `<!-- formula-not-decoded -->` and `<!-- image -->` markers (no network): correct
      per-page counts and locations, `complete` true/false logic.

## 2. Docling primary route

- [x] 2.1 `DoclingEngine`: POST to `{DOCLING_URL}/v1/convert/file` with `to_formats=md`,
      `md_page_break_placeholder=<sentinel>`, `do_formula_enrichment=true`; parse
      `.document.md_content`; short health-check timeout; config `DOCLING_URL` +
      convert/health timeouts.
- [x] 2.2 Assemble page anchors from the sentinel (`--- p.N ---`) and build the
      completeness report from the returned markdown. Engine label `Docling`.
- [x] 2.3 Wire into the orchestrator as the primary route ahead of the flat-text chain;
      on health-check/convert failure, fall through. Fallback engines return
      `complete: false` with the flat-text note.

## 3. OCR pre-step

- [x] 3.1 Detect no-usable-text-layer (cheap probe or near-empty primary result); run
      `ocrmypdf --skip-text` to a temp copy (original untouched); extract that via Docling.
- [x] 3.2 Engine label `OcrThenDocling`; populate `ocr_pages`. Missing `ocrmypdf` degrades
      gracefully (skip OCR, report incompleteness), never panics.

## 4. Tool surface

- [x] 4.1 Extend `get_pdf_text` / `get_pdf_first_pages` results with `format`,
      `page_anchors`, `completeness`; add the `plain` option preserving flat output.
      Update tool descriptions in `server.rs` to state the new contract (markdown default,
      completeness report, loud failure).
- [x] 4.2 Preserve the loud-fail floor: all-routes-fail or all-sub-floor returns the
      existing error naming the OCR remedy; never empty-as-success.

## 5. Golden set + tests

- [x] 5.1 Fixtures: equation-bearing PDF, table-heavy report, two-column paper,
      scanned/image-only PDF (extend `tests/fixtures/gen_pdfs.py`; commit generated PDFs).
- [x] 5.2 `tests/pdf_extraction.rs`: assert markdown tables intact; equation LaTeX present
      + zero undecoded formulas; OCR path recovers scanned text + populates `ocr_pages`;
      flat-text fallback reports `complete: false`. External-engine-dependent tests skip
      loudly when the engine is absent (pattern already used for `pdftotext`).
- [x] 5.3 `cargo test` green on a host without the Docling service (primary-route tests
      skip; structure/completeness/fallback tests pass).

## 6. Docs

- [x] 6.1 README: the extraction routes, the completeness contract (presence trustworthy;
      declared drops = unknown, not absent), and the `DOCLING_URL`/ocrmypdf config.
- [x] 6.2 Note the version bump and the `get_pdf_text` result-shape change in the changelog.
