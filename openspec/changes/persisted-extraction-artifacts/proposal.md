## Why

Extraction is complete and trustworthy (`llm-reliable-pdf-extraction`,
`scan-and-large-pdf-extraction`) but **nothing keeps the result**. `attach_file` stores a
PDF; `get_pdf_text` runs Docling and hands markdown to the caller; nothing links the two.
After a successful extraction the item holds the PDF and no text derivative
(`list_attachments` shows only the PDF), so the markdown exists solely in one model's
context and dies with the session.

Evidenced by an ingest of eleven sources into the ENERGY collection on 2026-07-27:

- **Every consumer re-extracts.** A 69-page report (`U8R34XW9`) is ~250,000 characters of
  context to produce a file that should have been written once. Cost and latency scale
  with readers, not documents. Today "make the markdown available" and "read the whole
  document into a model" are the same operation, because there is no addressable path to
  a Docling result — the only on-disk copies are the harness's tool-result spill files,
  a client-side accident of response size keyed to a single call, not to an item.
- **Results are not reproducible across sessions.** A figure quoted from an extraction in
  one session cannot be checked against a stored artefact later; it can only be
  re-derived, possibly by a different engine.

Two constraints shape the answer and were verified in the source before this proposal was
written:

- **The 69-page report cannot be extracted in one call.** Whole-document requests above
  `pdf_whole_document_max_pages` (default 50) are refused with `PdfDocumentTooLarge`
  (`core/pdf.rs:1278`, `core/config.rs:137`) — deliberately, per the large-document
  requirement. Any derivative must therefore be built by walking page windows, as the
  `pdf-text` CLI already does internally.
- **Writes and reads use different Zotero faces.** Writes go to `api.zotero.org`; reads
  come from the local SQLite database (`core/writer/client.rs:6-14`,
  `core/reader/conn.rs`). Anything written into the Zotero library is invisible to this
  server's own reads until Zotero desktop syncs it back down. Storage cannot be a Zotero
  child attachment without accepting an unbounded blind interval. See `design.md`.

**Search coverage, checked.** `search_items(include_fulltext)` queries Zotero's own
`fulltextWords`/`fulltextItemWords` tables (`core/reader/search.rs:64-73`), which only
Zotero's indexer populates — this server never writes them. So full-text search covers
Zotero's flat index and never the Docling markdown, on any route. Two further limits are
in the same code and are currently undocumented: the full-text clause is added **only when
the query contains no whitespace**, so a multi-word full-text query is dropped entirely
rather than degraded; and the join runs via `parentItemID`, so a top-level attachment
never matches.

## What Changes

- **At most one extraction per PDF.** The layout-faithful markdown for a given PDF is
  produced once and stored. Later reads serve the stored artefact.
- **`get_pdf_text` serves the derivative.** This is the point of the change: existing
  consumers — including the `pdf-text` CLI and the Pi arbiter — stop re-extracting without
  changing how they call anything. A result states whether it was served from storage or
  freshly extracted.
- **The derivative is a full-document window walk.** Building it walks page windows
  internally and assembles them in page order, so documents above the whole-document page
  cap (the 69-page report, the 414-page scan) get a complete derivative even though a
  single whole-document tool call over them is, correctly, still refused.
- **Addressable by item key, with a path.** A caller can obtain the filesystem path
  without the content passing through the response, so a whole document can be handed to
  a tool or a person at constant response cost. The path is server-local, and the contract
  says so plainly for remote (HTTP transport) callers.
- **Staleness is defined, not guessed.** The derivative is keyed to the PDF's content hash
  and a server-owned extraction-profile string. A changed PDF or a bumped profile
  invalidates it; nothing else does. Callers can force a refresh.
- **Only layout-route output is ever stored.** Flat-text output never becomes a
  derivative, never overwrites one, and never satisfies "a derivative exists" — whatever
  its character count.
- **Backfill in place.** Pre-existing items reach the same state without being deleted and
  recreated, with item metadata and hand-written `extra` fields untouched. Resumable, with
  a per-item outcome report.
- **Search coverage is documented at the point of use**, including the whitespace and
  parent-item limits above, so a caller knows what a negative result means.
- **Fixtures are committed to the repo**, so the acceptance test runs in `cargo test` on
  any host rather than only against one person's library.

Non-goals: no change to how extraction itself works (windowing, OCR, formula enrichment
all stand); no new search engine and no change to Zotero's indexing; no change to the
degradation contract — a cold Docling service still yields today's labelled flat-text
fallback. **That contract is the separate change `honest-extraction-degradation`, which
this change deliberately does not depend on**: this one only refuses to *store* flat text.

## Capabilities

### New Capabilities
- `extraction-artifacts`: durable, addressable, idempotent storage of a layout-faithful
  text derivative per PDF attachment — one-extraction-per-PDF, window-walk assembly,
  retrieval by item key and by path, a defined staleness key, layout-only storage gating,
  in-place backfill, a stated search-coverage contract, and committed fixtures.

### Modified Capabilities
<!-- none — the degradation contract lives in `honest-extraction-degradation` -->

## Impact

- **Code**: `crates/zotero-mcp/src/core/pdf.rs` (window-walk assembly, serve-from-store in
  the `get_pdf_text` path); a new derivative store module; `src/tools/attachments.rs`
  (`get_pdf_text`, `get_pdf_path`, `attach_file` at `:281`, plus a derivative-path /
  backfill verb); `src/main.rs` (the `pdf-text` CLI walks the store instead of
  re-extracting); `src/server.rs` registration; `src/core/config.rs` (store location,
  extraction-profile string).
- **APIs/contracts**: additive only. New fields on the extraction result (served-from vs
  fresh); at least one new tool verb. No existing verb or argument changes meaning.
- **Storage**: a server-owned store outside the Zotero library — see `design.md` for why
  not a Zotero attachment and not a sidecar in `~/Zotero/storage`. Order of ~250KB of
  markdown per large report.
- **Hosts**: must work on the Mac (read-write `~/Zotero`) and on the VM, which reads a
  **read-only** Resilio mirror of `~/Zotero`; the store therefore cannot live under the
  Zotero tree.
- **Dependencies**: none new expected; Docling and Poppler as today.
- **Tests / fixtures**: committed synthetic table-bearing PDFs (a small one and one above
  the whole-document page cap) with asserted table counts and a window-walk assembly test;
  Docling-dependent tests skip loudly when the service is absent, per existing convention.
  The real ENERGY items — `W6U9A838` (routed both ways), `X4C6F4EF` (42 tables),
  `2PGJT3N2` (24 tables), `U8R34XW9` (69 pp), `GQTHCXPR` (metadata, no PDF) — are recorded
  as a manual verification pass, not as the automated gate.
