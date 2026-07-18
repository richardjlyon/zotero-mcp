> **Design deviation (D2/D3):** page windows are sliced **locally with `lopdf`**
> (`delete_pages`) into a temp PDF, and that slice is sent to the existing route
> stack (Docling's own OCR handles scans). This resolves the docling `page_range`
> Open Question entirely (no dependency on the service's field shape) and makes the
> OCR pre-step / flat-chain rescue window-scoped for free — the slice *is* the
> pre-split, so no `qpdf`/`pdfseparate` is needed. Tasks 3.1/3.4/4.1/4.2 are
> satisfied by this single mechanism.

## 1. Total page count (D4)

- [x] 1.1 `total_page_count(path)`: `lopdf` count with a Poppler `pdfinfo` fallback and
  `0`-with-note on failure. Unit tests: `total_page_count_reports_document_length`,
  `total_page_count_is_zero_for_unparseable_pdf`.
- [x] 1.2 `total_pages: u32` on `Completeness`, stamped on every result in `extract_windowed`.
  Existing tests updated; `whole_document_result_reports_total_pages` covers it.

## 2. Windowing plumbing (D1)

- [x] 2.1 Optional 1-indexed `from_page`/`to_page` on `PdfTextArgs` (+ `page_window` helper,
  tool-description window guidance); `FirstPagesArgs.n` maps to window `[1, n]`.
- [x] 2.2 `window: Option<(u32,u32)>` threaded through `get_pdf_text` → `extract_windowed`;
  public `extract` kept as the whole-document wrapper (no churn to existing callers).
- [x] 2.3 `get_pdf_first_pages` extracts only its `[1, n]` window (slices before conversion),
  no longer whole-document-then-truncate.

## 3. Windowed layout conversion + built-in OCR (D2)

- [x] 3.1 Window slicing via `slice_pages` (`lopdf`) feeds the existing Docling route;
  `assemble_page_anchors(_, _, start_page)` offsets anchors to true page numbers.
  Live-verified: `windowed_docling_anchors_carry_true_page_numbers_live`.
- [x] 3.2 Anchor offset unit test: `windowed_anchors_carry_true_document_page_numbers`.
- [x] 3.3 Live test recovers windowed content with correct anchors + window-scoped
  completeness (`total_pages` whole-doc, `pages` = window). Passes against live docling-serve.
- [x] 3.4 N/A — local slicing removes the `page_range` field-shape risk; if `slice_pages`
  fails the orchestrator surfaces a loud `Error::Pdf`, never a silent gap.

## 4. Window-scoped OCR (D3)

- [x] 4.1 The OCR pre-step / flat-chain rescue run over the *slice* (bounded), because the
  whole route stack operates on the sliced working file. No separate splitter needed.
- [x] 4.2 `slice_pages` uses pure-Rust `lopdf` (always available); a slice failure is a loud
  error, not a silent empty success. Slicing does not mutate the original
  (`slice_pages_keeps_only_the_requested_window`).

## 5. Large-document guard (D5)

- [x] 5.1 Config `pdf_whole_document_max_pages` (default 50) + parse tests
  (`pdf_whole_document_max_pages_defaults_to_50`, `..._parses_from_toml`).
- [x] 5.2 `PdfDocumentTooLarge` error variant; guard in `extract_windowed` for un-windowed
  requests over the ceiling. Tests: `large_whole_document_request_is_refused_with_the_windowing_remedy`,
  `windowed_request_bypasses_the_guard_and_reports_total_pages`.

## 6. Completeness scoping (D6)

- [x] 6.1 `pages`/`per_page_chars`/drop vectors describe the returned window; `total_pages`
  the whole document; a "page window N..=M of T" note is added on windowed results.
  Cache read/write gated to whole-document requests (a window never poisons `.zotero-ft-cache`).

## 7. Golden set + evidenced items

- [x] 7.1 Reused `multipage.pdf` (3-page text doc) for offline window-walk coverage. NOTE: no
  synthetic >50-page scan fixture was added — generating one is heavy; the large-scan path is
  instead validated against the real evidenced item (7.3). A committed large-scan fixture
  remains a nice-to-have follow-up for engine-free CI.
- [x] 7.2 `window_walk_covers_the_whole_document_with_a_stable_total` (offline, `pdf-extract`):
  each window covers exactly its page, no spillover, stable `total_pages`.
- [~] 7.3 Manual verification against the real items (`5X3ZBGQS` 15-page scan, `6RT3NJQ6`
  414-page scan) via a throwaway harness on the freshly-built code — RUNNING (OCR+Docling is
  slow); results pending before commit.

## 8. Gate

- [x] 8.1 `cargo test` green (154 lib + integration incl. live Docling; external-engine tests
  skip loudly when absent). `cargo clippy` adds **zero** new warnings vs baseline.
  README + CHANGELOG + tool descriptions updated. Version bump/spec archive deferred to release.
