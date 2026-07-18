## ADDED Requirements

### Requirement: Page-windowed extraction

The server SHALL support extracting a bounded, inclusive page window `[from, to]` (1-indexed)
so that the OCR and layout-conversion work for one call is bounded by the window size rather
than the document size. Both the layout-aware route (Docling `page_range`) and any OCR
pre-step SHALL be confined to the requested window. Consecutive, non-overlapping windows in
page order SHALL together reproduce the full faithful text of the document — a window carries
no truncation or summarisation of the pages it covers. When no window is requested, behaviour
is whole-document, subject to the large-document requirement.

#### Scenario: Window returns exactly its pages

- **WHEN** a caller requests pages `5..=8` of a document
- **THEN** the result contains only those four pages, each under its correct `--- p.N ---`
  anchor with the document's own page numbers (5, 6, 7, 8), and no content from pages outside
  the window

#### Scenario: Walking windows reconstructs the whole document

- **WHEN** a caller extracts a document as consecutive non-overlapping windows covering every
  page
- **THEN** concatenating the windows in page order yields text equivalent to a single
  whole-document extraction of the same route — no page is dropped, duplicated, or reordered

#### Scenario: Windowed extraction of a scan OCRs only the window

- **WHEN** a caller requests a page window of an image-only PDF
- **THEN** OCR/text recovery is applied only to the pages in that window, the recovered pages
  appear in `ocr_pages`, and per-call time is bounded by the window size — a large scan is
  readable one window at a time

### Requirement: Total page count reporting

Every extraction result SHALL report the document's true total page count, independent of the
window returned, so a caller can deterministically iterate windows until the whole document is
covered without guessing where it ends.

#### Scenario: Windowed result reports the whole-document page count

- **WHEN** a caller extracts pages `1..=20` of a 414-page document
- **THEN** the result returns those 20 pages AND reports a total page count of 414, so the
  caller knows 394 pages remain and can request the next window

#### Scenario: Page count is present even on the flat-text route

- **WHEN** extraction falls back to a flat-text engine that cannot page-anchor its output
- **THEN** the result still reports the document's total page count (obtained independently of
  the flat engine) so callers can bound their iteration, while `pages`/`per_page_chars` may be
  empty as today

### Requirement: Large documents are never silently unreadable

A request that cannot be satisfied within its time budget — most acutely a whole-document
request over a large scan — SHALL fail loudly with an actionable error that names windowed
extraction (a page range) as the remedy. It SHALL NOT return empty or silently-partial text as
success, and SHALL NOT surface only a bare timeout with no path forward. The same document
SHALL be fully readable via windowed requests.

#### Scenario: Whole-document request on a large scan directs to windows

- **WHEN** a whole-document extraction of a 414-page image-only PDF cannot complete within its
  time budget
- **THEN** extraction returns a loud error stating the document is too large to extract whole
  and instructing the caller to request page windows — never empty text as success

#### Scenario: The large scan is readable in full via windows

- **WHEN** the same 414-page scan is extracted as a sequence of page windows
- **THEN** every window returns faithful OCR-recovered text for its pages, and the windows
  together cover all 414 pages

## MODIFIED Requirements

### Requirement: OCR for image-only PDFs

The server SHALL detect PDFs with no usable text layer and OCR them (ocrmypdf/tesseract or the
layout route's own OCR) before extraction, without modifying the original file. Detection and
OCR SHALL operate over the pages actually being extracted — the whole document when no window
is requested, or the requested window otherwise — so that image-only PDFs of any length are
recoverable a window at a time rather than only as an all-or-nothing whole-document OCR.
OCR-recovered pages SHALL be recorded in the completeness report.

#### Scenario: Scanned document

- **WHEN** an image-only PDF is extracted
- **THEN** text is recovered via an OCR pre-step, the engine label reflects OCR was used,
  the recovered pages appear in `ocr_pages`, and the original file is unchanged

#### Scenario: Large scanned document, window at a time

- **WHEN** an image-only PDF larger than a single OCR pass can handle whole is extracted one
  page window at a time
- **THEN** each window recovers faithful text for its pages via OCR, records those pages in
  `ocr_pages`, completes within the per-window time budget, and never returns empty text as
  success for a window that contains scanned content

#### Scenario: Nothing extractable

- **WHEN** no route (including OCR) can obtain text above the minimum floor for the requested
  pages
- **THEN** extraction returns a loud error naming the remedy and never returns empty text
  as success

### Requirement: Machine-readable completeness report

Every extraction result SHALL include a completeness report stating the engine used, the
document's total page count, the page count and per-page character counts of the returned
window, the page locations of undecoded formulas, untranscribed figures/charts, OCR-applied
pages, and low-text pages, and a boolean `complete` that is true only when a layout route ran
with formula enrichment and left zero unresolved drops over the pages it returned.

#### Scenario: Complete extraction

- **WHEN** a prose-and-table PDF extracts cleanly on the primary route with no undecoded
  formulas and no untranscribed images
- **THEN** `complete` is true and the drop-location lists are empty

#### Scenario: Windowed report describes the window against the whole

- **WHEN** a page window is extracted
- **THEN** the report's total page count reflects the whole document while the per-page and
  drop vectors describe only the returned window, and `complete` reflects only the pages
  actually returned — a complete window of an otherwise-unread document is not claimed to be a
  complete document

#### Scenario: Incomplete extraction is declared, not hidden

- **WHEN** an extraction drops a figure or an equation, or falls back to a flat-text
  engine
- **THEN** `complete` is false and the report identifies what was dropped and on which
  pages, so a consumer can treat those regions as unknown rather than absent

#### Scenario: Flat-text fallback is never authoritative

- **WHEN** extraction falls back to `pdf-extract` or `pdftotext` (Docling unreachable)
- **THEN** `complete` is false with a note that a flat-text engine cannot detect
  structure, so downstream never reads its absence as "not in the document"

### Requirement: Tested against a golden set

The change SHALL ship integration tests with fixtures covering an equation-bearing PDF, a
table-heavy report, a two-column paper, a small scanned/image-only PDF, and a large
(multi-window) scanned/image-only PDF, asserting both the extracted content and the
completeness report. The large-scan fixture SHALL be exercised across page windows, asserting
that the windows together cover every page and report a consistent total page count. `cargo
test` SHALL be the gate; tests that require an unavailable external engine (Docling service,
ocrmypdf, pdftotext) SHALL skip loudly rather than fail.

#### Scenario: Golden set runs in CI-like conditions

- **WHEN** `cargo test` runs on a host without the Docling service
- **THEN** primary-route tests skip with a clear message, the flat-text and completeness
  tests still run and pass, and no test silently passes by extracting nothing

#### Scenario: Large scan walked across windows

- **WHEN** the large-scan fixture is extracted as a sequence of page windows on a host with
  the OCR/layout engines available
- **THEN** each window returns faithful text for its pages within the per-window budget, the
  windows together cover all pages, and every result reports the same total page count
