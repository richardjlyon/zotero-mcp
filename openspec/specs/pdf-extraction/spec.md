# pdf-extraction Specification

## Purpose
TBD - created by archiving change llm-reliable-pdf-extraction. Update Purpose after archive.
## Requirements
### Requirement: Layout-aware, page-anchored PDF-to-markdown extraction

The server SHALL extract PDFs to markdown that preserves detected tables and reading
order, with per-page markers (`--- p.N ---`), via a layout-aware primary route (the
Docling service). The result SHALL declare its `format` (markdown or plain) and whether
page anchors are present. A `plain` option SHALL preserve the previous flat-text output.

#### Scenario: Table-bearing report

- **WHEN** a PDF containing tabular data is extracted on the primary route
- **THEN** tables appear as markdown tables with rows and columns intact (not interleaved
  text), each page's content follows its `--- p.N ---` marker, and `format` is markdown

#### Scenario: Plain output preserved

- **WHEN** a caller requests `plain` extraction
- **THEN** the previous flat-text behaviour is used and `format` is plain

### Requirement: Equation decoding

The primary route SHALL enable Docling formula enrichment so mathematical formulas are
decoded to their LaTeX representation in the output rather than dropped.

#### Scenario: Equation-bearing document

- **WHEN** a PDF containing a display equation is extracted on the primary route
- **THEN** the equation's LaTeX appears in the output at its page location, and the
  completeness report lists zero undecoded formulas for that page

#### Scenario: Enrichment unavailable

- **WHEN** formula decoding cannot run (model/route unavailable) and a formula region is
  detected
- **THEN** the region is preserved as an explicit undecoded marker AND its page is
  recorded in the completeness report's `undecoded_formulas` — never silently omitted

### Requirement: OCR for image-only PDFs

The server SHALL detect PDFs with no usable text layer and OCR them (ocrmypdf/tesseract)
before extraction, without modifying the original file. OCR-recovered pages SHALL be
recorded in the completeness report.

#### Scenario: Scanned document

- **WHEN** an image-only PDF is extracted
- **THEN** text is recovered via an OCR pre-step, the engine label reflects OCR was used,
  the recovered pages appear in `ocr_pages`, and the original file is unchanged

#### Scenario: Nothing extractable

- **WHEN** no route (including OCR) can obtain text above the minimum floor
- **THEN** extraction returns a loud error naming the remedy and never returns empty text
  as success

### Requirement: Machine-readable completeness report

Every extraction result SHALL include a completeness report stating the engine used, page
count, per-page character counts, the page locations of undecoded formulas, untranscribed
figures/charts, OCR-applied pages, and low-text pages, and a boolean `complete` that is
true only when a layout route ran with formula enrichment and left zero unresolved drops.

#### Scenario: Complete extraction

- **WHEN** a prose-and-table PDF extracts cleanly on the primary route with no undecoded
  formulas and no untranscribed images
- **THEN** `complete` is true and the drop-location lists are empty

#### Scenario: Incomplete extraction is declared, not hidden

- **WHEN** an extraction drops a figure or an equation, or falls back to a flat-text
  engine
- **THEN** `complete` is false and the report identifies what was dropped and on which
  pages, so a consumer can treat those regions as unknown rather than absent

#### Scenario: Flat-text fallback is never authoritative

- **WHEN** extraction falls back to `pdf-extract` or `pdftotext` (Docling unreachable)
- **THEN** `complete` is false with a note that a flat-text engine cannot detect
  structure, so downstream never reads its absence as "not in the document"

### Requirement: Labelled route and preserved fallback

Extraction SHALL try the layout-aware primary route first and fall back to the local
flat-text chain when the service is unreachable, and the result SHALL always identify
which engine produced it. Degradation SHALL be observable in the result, never silent.

#### Scenario: Service unreachable

- **WHEN** the Docling endpoint fails its health check
- **THEN** extraction proceeds via the local fallback chain, the result's engine label
  reflects the fallback, and the completeness report marks it incomplete

### Requirement: Tested against a golden set

The change SHALL ship integration tests with fixtures covering an equation-bearing PDF, a
table-heavy report, a two-column paper, and a scanned/image-only PDF, asserting both the
extracted content and the completeness report. `cargo test` SHALL be the gate; tests that
require an unavailable external engine (Docling service, ocrmypdf, pdftotext) SHALL skip
loudly rather than fail.

#### Scenario: Golden set runs in CI-like conditions

- **WHEN** `cargo test` runs on a host without the Docling service
- **THEN** primary-route tests skip with a clear message, the flat-text and completeness
  tests still run and pass, and no test silently passes by extracting nothing

