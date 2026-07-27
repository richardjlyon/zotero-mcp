## Context

The capability needs one load-bearing decision — **where the derivative lives** — and
three that follow from it: how a whole-document derivative is built for documents above
the single-call page cap, what makes a derivative stale, and how backfill runs. Four
facts in the current code constrain all of them.

1. **Reads and writes use different Zotero faces.** Reads come from the local SQLite
   database opened read-only (`core/reader/conn.rs`); writes go to `api.zotero.org`
   because Zotero's local HTTP server returns `501` for `POST`/`PATCH`/`DELETE`
   (`core/writer/client.rs:6-14`). Anything this server writes into the library is
   invisible to its own reads until Zotero desktop syncs it down — an interval this
   server neither controls nor observes.
2. **The Zotero data directory is not always writable.** On the VM it is a read-only
   Resilio mirror of the Mac's `~/Zotero`. Anything written under `~/Zotero/storage`
   works on one host and fails on the other.
3. **Whole-document extraction above 50 pages is refused by design.**
   `extract_windowed` returns `PdfDocumentTooLarge` when no window is requested and the
   page count exceeds `pdf_whole_document_max_pages` (`core/pdf.rs:1278`,
   `core/config.rs:137`). This is the large-document requirement working correctly, and
   it applies to the 69-page report that motivates the change.
4. **The window walk already exists.** The `pdf-text` CLI walks windows internally and
   streams a whole document to stdout (shipped with `scan-and-large-pdf-extraction`);
   Poppler slicing makes a window ~7s and a 414-page walk ~4 min.

## Goals / Non-Goals

**Goals:**

- One layout extraction per (PDF content, extraction profile), served to every later
  reader through the surfaces callers already use.
- A derivative that covers the whole document regardless of page count.
- A filesystem path to it, obtainable at constant response cost.
- Identical behaviour on the Mac and on the read-only-mirror VM.
- Backfill that touches nothing in the Zotero library.

**Non-Goals:**

- Changing what a caller is served when the layout route is unavailable — that is
  `honest-extraction-degradation`.
- Making derivatives visible in the Zotero UI, or syncing them between machines.
- Making derivatives searchable. Search coverage is documented, not extended.
- Detecting that the remote Docling model changed.

## Decisions

### D1 — Storage: a server-owned store outside the Zotero library

The derivative is written to a server-owned directory (default under the platform state
directory, e.g. `~/.local/state/zotero-mcp/derivatives/`, overridable in config), keyed by
attachment key plus a content hash of the source PDF.

Rejected alternatives, and why:

- **A Zotero child attachment via `api.zotero.org`.** The natural answer, and the one the
  first draft of this change implicitly assumed. It fails on fact 1: after the write, this
  server cannot see the derivative until Zotero desktop syncs it down, so
  "a later session finds it by item key" is unsatisfiable for an unbounded interval. It
  also mutates the library (bumping item versions), doubles every item's child count in
  `list_attachments`, consumes Zotero storage quota, and — because Zotero would index the
  markdown — quietly changes search coverage, an explicit non-goal.
- **A sidecar file in `~/Zotero/storage/<attachmentKey>/`.** Invisible to Zotero and free
  of quota, but fails on fact 2: unwritable on the VM. Zotero also treats those directories
  as its own and may prune unrecognised files on sync.

The cost of D1 is real and accepted: derivatives are not visible in the Zotero UI and do
not travel between machines — each host builds its own. That is the price of working on
both hosts without touching the library. If Zotero-visible derivatives are wanted later,
an explicit "publish derivative as an attachment" verb can be added on top; it is out of
scope here.

### D2 — The derivative is built by an internal window walk

Building a derivative walks page windows using the existing slicing and assembly path and
concatenates them in page order, so it is not subject to the single-call whole-document
cap (fact 3). The cap continues to govern *tool calls*, unchanged — a caller asking for a
414-page document whole still gets the loud `PdfDocumentTooLarge` with the windowed
remedy; the store is what makes the whole document available anyway.

A walk is atomic: windows are assembled to a temporary file and moved into place only when
every page is covered, so an interrupted walk leaves no short derivative to be mistaken
for a whole one. Failed walks record which pages failed, so a retry is possible and a
caller is never told a document is complete when it is not.

### D3 — Staleness key: PDF content hash + extraction-profile string

The store key is the hash of the source PDF's bytes plus a server-owned profile string
covering engine identity and the settings that materially affect output (layout route,
OCR policy, formula enrichment, page-anchor placeholder). Content-hashing the PDF is
cheap next to a Docling call and is the only signal that is reliable across hosts —
mtime is not, given Resilio.

The profile string is hand-bumped in the source when extraction behaviour changes. The
remote Docling model version is deliberately not part of the key: `DoclingEngine` exposes
only a liveness probe, so it cannot be observed, and pretending otherwise would make the
requirement untestable.

### D4 — Serving: the store sits in front of `get_pdf_text`, not beside it

The lookup happens inside the existing text path, so `get_pdf_text`, `get_pdf_first_pages`
and the `pdf-text` CLI all benefit with no change to their arguments. A stored
whole-document derivative also satisfies page-window requests by slicing on page anchors —
no extraction, no Docling call. Results gain a field distinguishing served-from-store from
freshly-extracted; nothing existing changes meaning.

Only reads populate the store. `attach_file` does **not** block on extraction: it is
currently a local write plus one API POST, and making it wait minutes on a GPU host over
the tailnet would be a bad trade for a verb used in bulk ingest. The consequence — a
derivative exists after the first read rather than at the instant of attach — is a
deliberate deviation from the literal wording of the originating request's outcome 1, and
it preserves the substance of outcomes 1–3: extraction happens once, durably, and every
later reader is served from storage. Eager population belongs in the backfill verb, which
is the right place for "extract these eleven items now".

### D5 — Backfill is a server-side verb over a set of items

Backfill takes a collection or a list of item keys, walks each item's PDF, and writes
derivatives. It issues no Zotero writes at all, so the "metadata untouched" requirement is
satisfied structurally rather than by care. It is resumable because the store key already
tells it what is done, and it reports per-item outcomes rather than a single aggregate, so
a partial run is legible. Its response must stay well inside the MCP response ceiling: it
returns counts and per-item statuses, never extracted text.

### D6 — Fixtures are committed; the real library is a manual pass

Two synthetic PDFs are committed: a small table-bearing document and one above the
whole-document page cap. Assertions are on table count, page coverage and anchors — never
character count, since the observed failure mode is comparable volume with the tables gone
(3,496 flat vs 3,299 markdown on page 1 of `W6U9A838`). The eleven ENERGY items stay in
the proposal as a recorded manual verification pass; they cannot gate `cargo test`
anywhere but one machine.

## Risks / Trade-offs

- **Derivatives are host-local.** The Mac and the VM each build and hold their own. Two
  Docling runs per document across the estate instead of one. Accepted: the alternative
  is writing into a tree that is read-only on one of them.
- **Not visible in Zotero.** A user looking at the item in Zotero sees only the PDF. The
  derivative is a server-side asset addressed through the server, which is the honest
  description of what it is.
- **Store growth is unbounded.** ~250KB per large report, no eviction specified. Deliberate
  for now: the store is cheap relative to the extractions it replaces, and eviction can be
  added when a real size problem exists rather than a hypothetical one.
- **Content hashing large PDFs costs I/O on every read.** Negligible against a Docling
  call, but it is on the path of an otherwise pure cache hit; if it shows up in practice,
  hash size+mtime first and fall back to a full hash on mismatch.
- **A stale profile string is a silent correctness risk.** If extraction behaviour changes
  and the string is not bumped, stale derivatives are served indefinitely. Mitigation: the
  profile string lives next to the extraction settings it describes, and the served result
  reports which profile produced it, so a wrong one is visible rather than invisible.
- **Serving from store hides route provenance.** A derivative served today was produced by
  a route that ran days ago. The result must therefore carry the engine and profile that
  produced it, not the engine that would run now — otherwise the labelling that
  `honest-extraction-degradation` depends on becomes a lie about the past.
