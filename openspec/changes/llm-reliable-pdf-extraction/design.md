## Context

`core::pdf` already has the right shape: a `PdfEngine` trait (`extract(&Path) -> String`),
an orchestrator (`get_pdf_text` / `PdfEngines::build`) that tries engines in order, and a
typed `PdfTextResult { text, source, character_count }`. This change extends that seam
rather than replacing it. The consuming system treats the returned text as the frozen
*arbiter* a note is fact-checked against, so the design's overriding goal is: **presence
in the output is trustworthy, and every gap is explicitly declared** so absence is never
silently authoritative.

## The completeness report (the core design element)

Extraction returns, alongside the text, a `Completeness` value:

```
struct Completeness {
    complete: bool,             // true only if no unresolved drops
    engine: Engine,             // Docling | OcrThenDocling | PdfExtract | Pdftotext | ZoteroCache
    pages: u32,
    per_page_chars: Vec<usize>,
    undecoded_formulas: Vec<u32>,   // page numbers with a formula-not-decoded marker
    untranscribed_images: Vec<u32>, // page numbers with an image/chart not transcribed
    ocr_pages: Vec<u32>,            // pages recovered by OCR
    low_text_pages: Vec<u32>,       // pages under a char-density floor (possible drop)
    notes: Vec<String>,
}
```

Rules:
- The Docling markdown emits `<!-- formula-not-decoded -->` and `<!-- image -->`
  placeholders; the assembler counts them per page from the page-anchored output.
- Flat-text engines (`pdf-extract`, `pdftotext`) cannot detect structure, so they return
  `complete: false` with `notes: ["flat-text engine cannot detect tables/formulas/images"]`.
  This is deliberate: their absence must never read as authoritative.
- `complete == true` requires: a layout route ran, formula enrichment was on, and there
  are zero undecoded formulas and zero untranscribed images (or those images were
  transcribed/described). Otherwise `complete == false` and the drop locations say where.

## Decisions

- **Docling as the primary route, called over HTTP** (not embedded — Docling is
  Python/ML). `POST {DOCLING_URL}/v1/convert/file` with `to_formats=md`,
  `md_page_break_placeholder=<sentinel>`, and **`do_formula_enrichment=true`** (this
  change's key config: equations decode to LaTeX instead of dropping). Page anchors are
  derived by splitting on the sentinel and numbering `--- p.N ---`, matching the existing
  wider-system convention. Health-checked with a short timeout; on failure, fall through.
- **OCR as a pre-step, not a route.** When the source has no usable text layer (detected:
  primary route returns near-zero text, or a cheap text-layer probe fails), run
  `ocrmypdf --skip-text` to a temp text-layered copy and extract *that* through Docling.
  The original file is never mutated. Engine label becomes `OcrThenDocling` and the OCR'd
  pages populate `ocr_pages`.
- **Fallback chain preserved and reordered under the primary.** Order:
  `.zotero-ft-cache` (only if it is itself non-empty and not a known-bad cache) → Docling
  (± OCR) → `pdf-extract` → `pdftotext`. Cache is demoted below Docling for correctness
  because ft-cache is Zotero's own flat extraction; keep it as a fast path only when the
  Docling route is unavailable. (Open for the implementer: make cache-vs-Docling ordering
  configurable; default to Docling-first for arbiter quality.)
- **Output format flagged.** `PdfTextResult` gains `format: Markdown | Plain` and
  `page_anchors: bool`. A `plain: true` argument forces the old flat path for any caller
  that needs it. Markdown is the default because consumers are LLMs.
- **Loud failure preserved.** If every route fails or all yield sub-floor text, return the
  existing error naming the OCR remedy — never empty text with `source: success`.

## Risks / Trade-offs

- **LaTeX ≠ prose rendering.** A decoded equation is LaTeX; a note may quote the equation
  in prose. This change makes the content *present and declared* (so absence isn't
  assumed); exact string-matching of equations remains a downstream/human concern, not a
  goal here.
- **Chart/figure numbers.** Enabling picture-description / chart-data extraction is
  heavier (VLM) and lower-precision; this change requires only that untranscribed images
  are *counted and located* in the report (so downstream quarantines them). Actually
  transcribing chart data is a NON-GOAL here, listed as a follow-on.
- **Service dependency.** Primary route needs the tailnet endpoint; the completeness
  report makes degraded (fallback) extraction a declared, queryable state, not a silent
  one.
- **ft-cache demotion** could slow some reads; mitigated by keeping it as the fast path
  whenever the Docling route is not configured/reachable.

## Non-Goals

- Transcribing numeric data out of chart/figure images (report-and-quarantine only).
- Changing the MCP tool verbs or the enrichment/citation subsystems.
- The downstream vault-remediation logic itself (separate work; this change only gives it
  a complete, self-describing arbiter).
