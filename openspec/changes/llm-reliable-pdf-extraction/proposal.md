## Why

`zotero-mcp`'s PDF text is the **arbiter** downstream LLM ingestion fact-checks notes
against (the Obsidian vault pipeline). Today that text comes from a flat-text chain
(`.zotero-ft-cache` → `pdf-extract` → `pdftotext`) with three failures that directly
produce fabrication and, worse, *false-deletion* risk:

1. **No layout** — tables collapse to interleaved number-soup, so a model reconstructs
   them from memory (this put co-firing biomass in the wrong ROC band in the vault).
2. **Content silently dropped** — equations and figure/chart contents never appear, and
   nothing says so. A real quote that lived in an equation reads as "absent," so an
   automated remediation pass **deleted real quotes** believing them fabricated. The
   root defect is that the extraction cannot distinguish *"not in the document"* from
   *"present but not extracted."*
3. **Scanned PDFs yield nothing** — image-only PDFs return empty, and the gap is
   invisible to callers.

The system already runs a Docling GPU service (`docling-serve`) with ML table structure,
OCR, and an available-but-unenabled formula-decoding model. This change makes the
server's own extraction **complete and self-describing** for LLM ingestion.

## What Changes

- A **layout-aware primary extraction route** via the Docling service: markdown output
  with real tables, reading order, and `--- p.N ---` page anchors; **formula enrichment
  enabled** so equations are decoded to LaTeX rather than dropped.
- An **OCR pre-step** (ocrmypdf/tesseract) for image-only / no-text-layer PDFs, so
  scanned documents extract instead of returning empty.
- **A machine-readable completeness report on every extraction** — the load-bearing new
  property. Each result carries: the route/engine used, page count, per-page character
  counts, and the count + page locations of every *undecoded* region (formula not
  decoded, figure/chart image not transcribed, OCR-applied pages, low-text pages), plus
  a boolean `complete`. Downstream may trust *presence* in the text, and MUST treat
  *absence where the report shows drops* as "unknown," never as "not in the document."
- **Preserved offline fallback** (`pdf-extract` → `pdftotext`) for when the service is
  unreachable — but it reports `complete: false` with reason `flat-text-engine`, so its
  absence is never mistaken for authoritative.
- **Route/engine always labelled; loud failure** when no route can extract text — never
  empty-as-success.
- **A golden-set of fixtures + integration tests** (equation-bearing PDF, table-heavy
  report, two-column paper, scanned/image-only PDF) asserting both the extracted content
  and the completeness report; `cargo test` is the gate.

## Capabilities

### New Capabilities

- `pdf-extraction`: layout-aware, page-anchored, formula-decoded, OCR-capable
  PDF-to-markdown extraction with a machine-readable completeness report and loud
  failure, shared by all PDF-reading tools of the server.

### Modified Capabilities

_None removed. `get_pdf_text` / `get_pdf_first_pages` keep their verbs and arguments; the
result gains `format`, `page_anchors`, and `completeness` fields, and `source` gains
Docling/OCR variants. A `plain` option preserves the old flat-text output._

## Impact

- **Affected code**: `crates/zotero-mcp/src/core/pdf.rs` (new `DoclingEngine`, OCR
  pre-step, markdown + page-anchor assembly, completeness report, orchestrator routing),
  `src/server.rs` (tool result shape + descriptions), new fixtures under
  `crates/zotero-mcp/tests/fixtures/`, new `tests/pdf_extraction.rs`.
- **New dependencies/config**: HTTP call to `docling-serve` (reqwest already present);
  `DOCLING_URL` + timeouts in config; `ocrmypdf` subprocess (like the existing
  `pdftotext` shell-out). Docling formula enrichment enabled in the convert request.
- **Runtime**: primary route needs the tailnet Docling endpoint; fully degrades to the
  local flat-text chain when it is unreachable (reported as incomplete).
- **Risk**: low-medium. `get_pdf_text` output changes from plain text to markdown on the
  primary route (consumers are LLMs, which benefit); `plain` preserves old behaviour; the
  completeness report is additive. Rollback = disable the Docling route (config), leaving
  today's chain intact.
