# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-07-25

> **Response shapes changed in this release.** Three groups: the nine
> list-returning tools now answer in an envelope rather than a bare array;
> `get_pdf_text` / `get_pdf_first_pages` gained fields and default to markdown
> on the primary route; and `find_weak_metadata_items` items are now named
> objects. Details below. Two config keys are deprecated but still parse, so no
> config file needs editing to upgrade.

### Added

- **`find_duplicates` — a real dedup gate.** One read-only tool that answers
  "is this already in the library?" before an item is created, replacing a
  multi-step procedure that previously lived as prose in a skill and was
  therefore skippable. Three passes — individual title words, author surname,
  and identifier in every plausible form (both ISBN forms, URL-wrapped DOIs) —
  unioned by item key, trashed items excluded, each candidate triaged with a
  recommended action (`abort` / `attach_to_existing` / `ask` / `create_new`),
  a title-similarity score, and a note of which fields the existing record
  lacks. `queries_run` reports every query and its row count, so the caller can
  show its working. Closes two observed failures: a duplicate missed because the
  stored author's first name was misspelt, and a title never matched because the
  stored title carried a colon the query did not — the latter needing token
  matching, since library search is a single SQL `LIKE` and normalising only the
  query side cannot help.

- **Page-windowed PDF extraction.** `get_pdf_text` accepts optional
  `from_page` / `to_page` (1-indexed, inclusive) to extract a bounded page
  *window* instead of the whole document. The requested pages are sliced
  locally (`lopdf`) and only that slice is OCR'd and converted, so a large
  or scanned PDF stays tractable — per-call work is bounded by the window,
  not the document. Page anchors carry the document's true page numbers.
  This makes any reference in the library — scanned or not, of any length —
  readable in full through the MCP by walking windows.
- **Total page count.** `completeness` gains `total_pages`: the document's
  true page count, independent of the returned window, so a caller knows how
  many windows remain. `get_pdf_first_pages` now extracts only its `[1, n]`
  window rather than processing the whole document and truncating — so the
  opening pages of a large scan are reachable.
- **Large-document guard.** A *whole-document* request on a PDF with more
  than `pdf_whole_document_max_pages` pages (config, default 50) is refused
  with a loud `PdfDocumentTooLarge` error naming windowed extraction and the
  page count — never a silent timeout or empty success.

### Changed

- **Response-shape change: the nine list-returning tools now answer in an
  envelope.** `search_items`, `list_recent_items`, `list_collections`,
  `list_tags`, `list_attachments`, `list_annotations`, `search_crossref`,
  `search_semantic_scholar` and `find_weak_metadata_items` return
  `{"items": [...], "count": n, "possibly_truncated": bool}` where they
  previously returned a bare JSON array. This completes Slice G: all nine now
  declare an `outputSchema` on `tools/list` and populate `structured_content`,
  which a bare array cannot do — MCP requires an object at the root of a tool's
  output schema, and rmcp enforces it by refusing to start. Having been obliged
  to wrap, the envelope earns its place: `possibly_truncated` tells a caller
  that the response filled its limit and the library may hold more, which a bare
  array left indistinguishable from a complete result.

  `find_weak_metadata_items` additionally changes its element shape from a
  positional pair (`["ABC123", ["missing DOI"]]`) to a named object
  (`{"item_key": "ABC123", "weak_fields": ["missing DOI"]}`).

  The 10 object-returning tools and the 13 text-returning tools are unchanged.
  The three `lookup_*` tools stay untyped by decision, not deferral: the schema
  they would gain says only "an object", while migrating would push their
  structured `lookup_failed` body out of tool content. Reasoning in
  `docs/superpowers/specs/2026-07-25-slice-g-wire-format-decision.md`.

- **Two new guards on the tool surface.** `tests/tool_surface.rs` walks *every*
  registered tool and asserts an object-rooted input schema, an object-rooted
  output schema wherever one exists, a non-empty description and present
  annotations — the test that was missing when one malformed schema caused a
  client to reject the entire `tools/list` response, leaving the server
  connected with zero usable tools. `tests/tool_wire_shape.rs` asserts the
  actual `content` and `structured_content` of all three response families
  against the test fixture, so any future change to a response shape is a
  visible edit to an expectation rather than a discovery in production.

- **`lookup_doi` / `lookup_isbn` / `lookup_arxiv` survive a bad moment
  upstream.** Each retries once on a transient fault (5xx, 429 — honouring
  `Retry-After` up to a 5-second ceiling — connection errors, timeouts) and
  deliberately does not retry a genuine "not found". Identifiers are normalised
  first, so a DOI pasted as `https://doi.org/…` or an id as `arXiv:2401.12345`
  works. `lookup_isbn` tries the alternate ISBN form automatically when the
  given one is not indexed, which is the common case for an older paperback.
  When every attempt fails the tool returns an error result carrying a
  structured `lookup_failed` body — the attempt trail plus a `suggestion` of
  `fall_back_to_hand_built` or `stop_and_ask` — so the caller branches on data
  instead of parsing an HTTP error string. The observed failure this closes:
  `lookup_isbn` on a valid paperback ISBN returning a bare
  `http error: error sending request` with no retry and no alternate form.

- **`attach_file` no longer decides how the bytes are stored.** From an
  agent's point of view there is only "attach this file to this item"; where
  the bytes live is Zotero's own file-sync preference (cloud, WebDAV, or
  none), and the server had no business duplicating that decision at its own
  layer. `mode` now defaults to `null`, meaning "store it the way Zotero's
  own UI would" — bytes at `<data_dir>/storage/<key>/<filename>`, sync left
  to Zotero desktop. Nothing in the call path reads config any more. `mode`
  survives as an advanced escape hatch: pass `"linked_file"` if you
  specifically want Zotero to hold a path reference instead of the file
  (Calibre mirror, shared NAS), in which case the file's absolute path is
  stored. The tool description and README were rewritten to match; the
  `AttachmentOutsideBaseDir` error message no longer names a config key.

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

### Deprecated

- **`[zotero] attachment_mode` and `[zotero] linked_attachment_base_dir`.**
  Both still parse — an existing `config.toml` keeps working — but neither is
  read by any call path, and a `WARN` naming the key is logged at startup when
  either is present. Removal at v0.5.x. There is no migration to do: delete
  the keys and set your file-sync preference in Zotero itself.

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
