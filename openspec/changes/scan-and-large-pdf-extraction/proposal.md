## Why

The reference-integration pipeline grounds every claim against a reference's source
text read through this MCP (`get_pdf_text`). That contract only holds if *any* item in
the library returns faithful, complete text. Two classes fail today: image-only/scanned
PDFs come back with no usable text, and large scans cannot be read at all — a 414-page
scan (`6RT3NJQ6`) is currently unreadable, and a 15-page scan (`5X3ZBGQS`) yields
nothing. When the arbiter text is empty or absent, the pipeline reads "not in the
document", turning a retrieval gap into a fabrication / false-deletion risk.

The previous change (`llm-reliable-pdf-extraction`) specified an OCR route and a
completeness report, but extraction is still whole-document: OCR and the Docling convert
run over the entire file under a single fixed timeout, and `get_pdf_first_pages` extracts
the *whole* document before truncating. A large scan therefore exhausts the OCR/convert
timeout and returns nothing — no page of it is reachable. Length, not just scan-ness, is
the barrier.

## What Changes

- **Page-windowed extraction.** `get_pdf_text` and `get_pdf_first_pages` gain an optional
  page range so a caller can extract a bounded window (e.g. pages 1–20) instead of the
  whole document. The Docling route converts only that window (via `page_range`), and OCR
  is applied only to that window — so per-call work is bounded by window size, not
  document size. A 414-page scan becomes readable in full by walking windows.
- **Total page count in every result.** Each result reports the document's true total page
  count independent of the returned window, so a caller can deterministically iterate
  windows until the whole document is covered. Concatenating consecutive non-overlapping
  windows reproduces the full faithful text.
- **Scan OCR that actually fires and is observable.** The image-only detection + OCR path
  is made reliable for the windowed route, and every result states plainly whether OCR ran,
  on which pages, and — when a scan could not be OCR'd — fails loud with the remedy rather
  than returning empty text as success.
- **No silent length failures.** A whole-document request that cannot complete within its
  time budget surfaces a loud, actionable error naming windowed extraction as the remedy —
  never a silent timeout that reads as "nothing extractable".
- **Golden set extended** with the two evidenced items' shapes: a small image-only scan and
  a large (>100-page) scan, asserting both recovered text and a coherent completeness /
  page-count report, walked across windows.

Non-goals: no change to the layout/equation/table behaviour already specified; no change to
the flat-text fallback semantics beyond page-count reporting; tool verbs stay stable
(new args are optional and default to today's whole-document behaviour for small files).

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `pdf-extraction`: adds page-windowed extraction and total-page-count reporting as
  first-class requirements, tightens the OCR-for-scans requirement so it holds for large
  scans, and adds a "large document is never silently unreadable" requirement.

## Impact

- **Code**: `crates/zotero-mcp/src/core/pdf.rs` (orchestrator `extract`, `get_pdf_text`,
  `get_pdf_first_pages`, `DoclingEngine::convert`/`extract_markdown`, the OCR pre-step,
  `Completeness`, `truncate_to_first_pages`); `crates/zotero-mcp/src/tools/attachments.rs`
  (`PdfTextArgs`, `FirstPagesArgs`, tool schemas). No new tool verbs.
- **APIs/contracts**: `PdfTextResult` gains a total-page-count field; extraction tools gain
  optional page-range args. Additive; existing callers unaffected.
- **Dependencies**: Docling `page_range` convert option (service already supports it);
  ocrmypdf/tesseract unchanged. Possibly `qpdf`/`pikepdf` for pre-split when OCR must run on
  a window and Docling's own OCR is not relied upon — decided in design.
- **Tests**: new fixtures for a small and a large image-only scan; window-walk integration
  tests. External-engine tests skip loudly when Docling/ocrmypdf/pdftotext are absent.
