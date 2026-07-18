## Context

`get_pdf_text`/`get_pdf_first_pages` run the orchestrator in `core/pdf.rs`: Docling
(layout-aware, with an ocrmypdf pre-step for scans) → `.zotero-ft-cache` → `pdf-extract`
→ `pdftotext` → flat-chain OCR rescue. Every stage today is **whole-document**:

- Docling `convert` POSTs the entire file; the ocrmypdf pre-step OCRs the entire file
  under a fixed 300 s timeout.
- `get_pdf_first_pages` calls `get_pdf_text` (whole document) and only *then*
  `truncate_to_first_pages`. So even "first 2 pages" of a 414-page scan pays the full OCR
  + convert cost — which exceeds the timeout and returns nothing.

Result: small scans depend entirely on the whole-file OCR/convert succeeding, and large
scans are unreachable at any page. The fix is to make the requested page window the unit
of work everywhere, and to give callers the total page count so they can walk a large
document window by window. Response size (a 414-page markdown far exceeds a usable tool
response) means windowing must be **caller-driven**, not internal auto-pagination.

## Goals / Non-Goals

**Goals:**
- Extraction work (OCR + convert) is bounded by the requested window, not document size.
- Every result reports the document's true total page count, independent of the window.
- Image-only PDFs of any length are recoverable a window at a time; no scanned page is
  silently returned as empty text.
- A whole-document request that cannot complete fails loud with the windowing remedy.
- Concatenated non-overlapping windows reproduce the full faithful text.

**Non-Goals:**
- No change to layout/table/equation-enrichment behaviour or the completeness-report
  semantics beyond adding total page count and per-window scoping.
- No internal auto-pagination that streams a whole 400-page doc into one response.
- No new tool verbs; no change to flat-text fallback semantics beyond page-count reporting.

## Decisions

### D1 — Windowing is a first-class arg threaded to the engines, not post-hoc truncation
Add an optional inclusive 1-indexed page window to `PdfTextArgs`
(`from_page`/`to_page`, both optional; absent = whole document). `get_pdf_first_pages`'s
`n` maps to the window `[1, n]` and is passed to the extractor **before** conversion, not
truncated after. `extract()` gains a `window: Option<(u32, u32)>` parameter carried to the
Docling convert and the OCR pre-step. `truncate_to_first_pages` is retained only as the
flat-text path's cap (the flat engines cannot page-slice), and for defensive trimming.

*Alternative rejected:* keep extract-then-truncate and just raise timeouts — does nothing
for response size and still pays whole-document OCR cost.

### D2 — Docling `page_range` is the primary windowing mechanism; its built-in OCR handles scans
`DoclingEngine::convert` adds `page_range = [from, to]` to the multipart form. The
docling-serve build already OCRs by default, so a scanned window sent with `page_range`
is OCR'd internally by Docling — **no ocrmypdf pass on the primary route**, eliminating
the whole-file OCR bottleneck. Page anchors must carry the document's *true* page numbers:
`assemble_page_anchors` numbers pages from the window start offset (`from`), not always
from 1. A live test asserts a window's anchors equal the requested page numbers.

*Alternative rejected:* always pre-split + ocrmypdf before Docling — redundant with
Docling's own OCR and reintroduces a per-file OCR cost.

### D3 — ocrmypdf pre-step and flat-chain OCR rescue become window-scoped
When OCR must run outside Docling (Docling unreachable → flat-chain rescue), OCR only the
window: pre-split the requested pages into a temp PDF (Poppler `pdfseparate`/`qpdf
--pages`, whichever resolves) then ocrmypdf that slice. If no splitter is available, the
windowed OCR rescue is unavailable and the result says so (loud, not silent). The probe
`probe_text_layer` runs against the windowed slice so detection matches what is extracted.

### D4 — Total page count from a cheap, engine-independent source
Report `total_pages` on the result. Obtain it once, up front, independent of Docling and
of the flat engines, via the `lopdf` crate (pure Rust, already a transitive dep through
`pdf-extract`; count `/Type /Page` objects). Poppler `pdfinfo` is the fallback if `lopdf`
fails to parse. Total page count is also the gate for D5.

*Alternative rejected:* derive count from the extracted anchors — unavailable on the
flat-text route and only ever reflects the window, not the document.

### D5 — Whole-document requests above a page threshold are refused with the remedy
Because we know the page count up front (D4), a whole-document request (no window) on a
document exceeding `pdf_whole_document_max_pages` (new config, default 50) returns a loud
`PdfDocumentTooLarge`-style error naming windowed extraction and the total page count —
instead of attempting a doomed multi-minute OCR/convert that yields a bare timeout. Small
documents keep exactly today's whole-document behaviour. Windowed requests are never
refused on size.

*Alternative rejected:* attempt then time out — wastes minutes, yields no path forward,
and violates "never silently unreadable".

### D6 — Result shape is additive
`PdfTextResult`/`Completeness` gain `total_pages: u32`. `Completeness.pages` and
`per_page_chars` continue to describe the returned window; a new note distinguishes "a
complete window" from "a complete document" when a window is requested. Tool arg additions
are optional with today's defaults, so existing callers are unaffected.

## Risks / Trade-offs

- **Docling `page_range` page numbering / off-by-one** → assemble anchors with the window
  offset and add a live test asserting `--- p.N ---` equals the requested page numbers;
  fail the test loudly if Docling numbers windows from 1.
- **`page_range` unsupported by the deployed docling-serve build** → detect a rejected
  `page_range` (HTTP 4xx / errors array) and fall back to whole-document convert for that
  call, recording it in notes; large scans then hit the D5 threshold and get the remedy.
- **Total-count source disagreement (encrypted / malformed PDFs)** → `lopdf` then
  `pdfinfo`; if both fail, `total_pages = 0` with a note, and D5 falls back to attempting
  (small) or a generic large-doc error rather than crashing.
- **Splitter absent for flat-chain windowed OCR rescue** → primary route (Docling) is the
  main path; the rescue degrades to "unavailable, install qpdf/poppler", stated in notes,
  never a silent empty success.
- **Window-boundary faithfulness** → integration test walks the large-scan fixture across
  adjacent windows and asserts full page coverage with no gaps/overlaps and a stable
  `total_pages`.

## Migration Plan

Additive and backward-compatible: no verb or arg removals, new args default to current
behaviour. Deploy by rebuilding the server; existing stdio/HTTP callers keep working. New
config key `pdf_whole_document_max_pages` defaults on. Rollback is reverting the crate; no
data migration. The sister-repo mirror tax does not apply — these are `core/pdf.rs` /
tools changes, not the Plan-8 transport stack.

## Open Questions

- Exact `page_range` field name/shape the deployed docling-serve expects (`page_range`
  vs `page_start`/`page_end`) — confirm against the live service during implementation and
  pin in a live test.
- Default window size guidance for callers (the tool description) — likely ~20 pages;
  confirm against typical response-size limits.
