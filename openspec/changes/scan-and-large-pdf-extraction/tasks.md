> **Design deviation (D2/D3):** page windows are sliced **locally** into a temp PDF
> and sent to the existing route stack (Docling's own OCR handles scans). This
> resolves the docling `page_range` Open Question entirely (no dependency on the
> service's field shape) and makes the OCR pre-step / flat-chain rescue
> window-scoped for free — the slice *is* the pre-split. Tasks 3.1/3.4/4.1/4.2 are
> satisfied by this single mechanism.
>
> **Slicer choice (perf, learned in validation):** `lopdf` slicing is
> catastrophically slow on large files — `load` 135s + `delete_pages` 388s on the
> 414-page fixture (an 11-minute window). Slicing therefore uses **Poppler
> `pdfseparate` + `pdfunite`** (~1.5s for the same window; `pdfinfo` for the page
> count), with `lopdf` kept only as a pure-Rust fallback for hosts without Poppler.
> A single large-scan window now takes ~7s; the full 414-page walk ~4 min.
>
> **Delivery (§9):** shipped a `zotero-mcp pdf-text` CLI subcommand (same engine as
> the MCP tool) so the Pi harness — which has no MCP client by design — can consume
> it. A CLI has no response-size ceiling, so it walks large scans internally and
> streams the whole document to stdout in one call. Paired with the Pi skill
> `zotero-pdf-scan`.

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

## 9. CLI + Pi delivery (added — Pi has no MCP by design)

- [x] 9.1 `zotero-mcp pdf-text <key> [--from --to] [--plain] [--window-size]` subcommand:
  reuses `get_pdf_text`; whole-doc auto-walks large scans to stdout (no response-size cap);
  clean text on stdout, route/pages/total/complete diagnostics on stderr.
- [x] 9.2 Poppler-primary slicer + `pdfinfo` page count (perf fix above). Live-validated on
  `6RT3NJQ6`: window 5..=7 in ~7s; auto-walk streams windows 1..=3, 4..=6, … with correct
  anchors and per-window route/completeness.
- [x] 9.3 Pi skill `~/.pi/agent/skills/zotero-pdf-scan/SKILL.md` (validates via pi's loader):
  documents the CLI + the library-wide image-only scan procedure; flags the `zref` arbiter
  rewire as a deliberate follow-up.
- [ ] 9.4 FOLLOW-UP (needs sign-off): install the rebuilt binary (`cargo install --path`) and
  point `zref cmd_pdf_text`'s scanned/large path at `zotero-mcp pdf-text`. Not done
  autonomously — it changes the fact-check arbiter and needs a reinstall.
