# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

> A version bump is due at the next release: the `get_pdf_text` /
> `get_pdf_first_pages` result shape changed (additive fields, but the
> default output format on the primary route is now markdown).

### Changed

- **`get_pdf_text` / `get_pdf_first_pages` result shape.** `PdfTextResult`
  gains three fields: `format` (`markdown` | `plain`), `page_anchors`
  (whether `--- p.N ---` page markers are present), and `completeness` — a
  machine-readable report carrying the engine used, page count, per-page
  character counts, page locations of undecoded formulas / untranscribed
  images / OCR-recovered pages / low-text pages, notes, and a boolean
  `complete`. `source` gains two variants: `docling` and `ocr_then_docling`.
  On the primary route the text is now layout-aware **markdown** (tables,
  reading order, LaTeX equations) rather than flat text; a new
  `plain: true` argument on both tools preserves the previous flat-text
  output. Contract for consumers: presence in the text is trustworthy;
  where the report declares drops, absence means *unknown*, never "not in
  the document".
- Extraction route order is now Docling (with OCR pre-step) →
  `.zotero-ft-cache` → `pdf-extract` → `pdftotext`; the cache is demoted
  below Docling because it is itself a flat extraction. Flat-text results
  always report `complete: false` with an explicit note.

### Added

- **Docling primary extraction route**: HTTP convert via a
  [docling-serve](https://github.com/docling-project/docling-serve)
  instance with `do_formula_enrichment=true`, page anchors assembled from
  the page-break sentinel. Configured via the `DOCLING_URL` environment
  variable (takes precedence) or `docling_url` in `config.toml`, with
  `docling_convert_timeout_secs` (default 300) and
  `docling_health_timeout_secs` (default 5). Unset = route disabled,
  flat-text chain only.
- **OCR pre-step for scanned PDFs**: image-only PDFs (no usable text
  layer) are run through `ocrmypdf --skip-text` on a temp copy — the
  original is never modified — then extracted via Docling; the source is
  labelled `ocr_then_docling` and recovered pages populate
  `completeness.ocr_pages`. New config `ocrmypdf_path`; a missing
  `ocrmypdf` degrades gracefully and is declared in the report.
- Golden-set fixtures (equation, tables, two-column, scanned) and
  `tests/pdf_extraction.rs` integration tests; Docling/OCR-dependent tests
  skip loudly on hosts without the service or binaries.

## [0.3.2]

### Fixed

- **MCP clients silently dropped all tools.** Three struct fields typed as
  `serde_json::Value` (`Item.fields`, `FieldChange.current`, `FieldChange.proposed`)
  derived a boolean JSON Schema (`true`) under schemars. Claude Code's tool-schema
  validator rejects a boolean where a property schema is expected and, on that
  rejection, discards the *entire* `tools/list` response — so the server showed as
  "Connected" with zero usable tools. These fields now emit object-form schemas via
  `#[schemars(schema_with = ...)]` (`{}` for free-form values, `{"type": "object"}`
  for `Item.fields`), so the full tool surface registers again.

### Added

- OAuth: defensive alias `/.well-known/openid-configuration` → OAuth authorization
  server metadata, for clients that probe the OIDC discovery path.

## [0.3.1]

### Fixed

- `attach_file`: `imported_file` attachments now write bytes to local Zotero
  storage and omit `md5`/`mtime` from the row body, repairing attachment creation
  for WebDAV users.
