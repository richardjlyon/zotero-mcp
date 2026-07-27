## ADDED Requirements

### Requirement: At most one extraction per PDF

For a given PDF and extraction profile, layout-aware extraction SHALL run at most once.
Its markdown output SHALL be stored durably, surviving the session, process and client
that produced it, and SHALL be served to every later reader of that PDF. Storage SHALL NOT
live inside the Zotero library tree, so that it works on hosts where the Zotero data
directory is read-only.

#### Scenario: Second reader does not re-extract

- **WHEN** two separate sessions read the text of the same item, the second after the
  first has completed
- **THEN** the layout engine runs once in total, the second read is served from the store,
  and the result identifies itself as served from the store rather than freshly extracted

#### Scenario: Derivative outlives the creating process

- **WHEN** the process that produced a derivative has exited and the server has restarted
- **THEN** the derivative is still associated with the item and still served unchanged

#### Scenario: Read-only Zotero data directory

- **WHEN** the server runs on a host whose Zotero data directory is a read-only mirror
- **THEN** derivatives are still produced, stored and served — the store is outside the
  Zotero tree and no write is attempted inside it

### Requirement: Derivative is a full-document window walk

The derivative SHALL cover the whole document. Where a document exceeds the
whole-document page cap that governs a single extraction call, the derivative SHALL be
built by walking consecutive non-overlapping page windows and assembling them in page
order. The assembled derivative SHALL be equivalent to concatenating those windows: no
page dropped, duplicated, reordered or truncated. A partially built derivative SHALL NOT
be served as complete.

#### Scenario: Document above the whole-document cap

- **WHEN** a derivative is built for a document with more pages than the whole-document
  cap allows in one call
- **THEN** the derivative covers every page of the document, with page anchors carrying
  true document page numbers, even though a single whole-document extraction call over
  that document is still refused

#### Scenario: A window fails mid-walk

- **WHEN** one window of a walk fails to extract
- **THEN** no complete derivative is recorded for that document, the failure names the
  pages that could not be extracted, and a later attempt can resume — a document is never
  left with a silently short derivative presented as whole

#### Scenario: Assembly matches the windows

- **WHEN** a derivative built by walking windows is compared with the concatenation of the
  same windows extracted individually
- **THEN** the text is equivalent, and every page appears exactly once in page order

### Requirement: Existing readers are served from the store

The existing text-reading surface — the `get_pdf_text` tool and the `pdf-text` CLI —
SHALL serve a current stored derivative instead of re-extracting, without any change to
how callers invoke them. Results SHALL state whether they came from the store or from a
fresh extraction. A page-window request SHALL be satisfiable from a stored whole-document
derivative without re-running extraction.

#### Scenario: Unchanged call, no re-extraction

- **WHEN** an existing caller invokes the text tool exactly as it does today, for an item
  with a current derivative
- **THEN** the stored text is returned, no layout-engine call and no OCR is made, and the
  result declares it was served from the store

#### Scenario: Window served from a whole-document derivative

- **WHEN** a caller requests a page window of an item whose whole-document derivative is
  stored and current
- **THEN** the requested pages are returned from the store, with the same page anchors as
  a fresh windowed extraction, and no extraction runs

#### Scenario: No derivative yet

- **WHEN** the text of an item with no current derivative is requested
- **THEN** extraction runs as it does today, the result declares it was freshly extracted,
  and (where the run was layout-faithful) a derivative is stored for later readers

### Requirement: Addressable by item key, with a filesystem path

The derivative SHALL be retrievable by item key, and a filesystem path to it SHALL be
obtainable without the content passing through the response — the cost of asking for the
path SHALL be independent of document length. The path SHALL be documented as server-local:
where the server is reached over a remote transport, the contract SHALL say plainly that
the path refers to the server's filesystem, and any path rewriting applied SHALL be stated.

#### Scenario: Path retrieval is cheap for a large document

- **WHEN** a caller asks for the derivative path of a 69-page report
- **THEN** the response carries the path and not the document text, and its size is
  independent of the document's length

#### Scenario: The path resolves to the served content

- **WHEN** the returned path is read from the server's filesystem
- **THEN** its bytes are the same content the server serves for that item, in full,
  untruncated

#### Scenario: Remote caller is not misled about the path

- **WHEN** a caller connected over the HTTP transport asks for a derivative path
- **THEN** the response makes clear the path is on the server's filesystem — the caller is
  never handed a path that silently does not exist on its own machine

#### Scenario: Absent versus empty versus no PDF

- **WHEN** a derivative is requested for an item that has no stored derivative
- **THEN** the response distinguishes "not yet extracted", "extraction failed" and "item
  has no PDF attachment" from one another, and none of them is reported as an empty
  derivative

### Requirement: Defined staleness key and explicit refresh

A stored derivative SHALL be keyed to the content of the source PDF and to a server-owned
extraction-profile identifier covering the engine and settings that produced it. A
derivative SHALL be treated as stale when, and only when, the PDF's content differs from
the one that produced it or the profile identifier differs from the current one. Staleness
SHALL NOT depend on data the server cannot observe, such as a remote model version. A
caller SHALL be able to force re-extraction.

#### Scenario: Replaced PDF invalidates the derivative

- **WHEN** the attached PDF is replaced with different content and the text is requested
- **THEN** the stored derivative is not served as current — extraction re-runs and
  replaces it, so a derivative never silently describes a superseded file

#### Scenario: Untouched PDF is not re-extracted

- **WHEN** the text of an item is requested repeatedly and neither the PDF's content nor
  the extraction profile has changed
- **THEN** extraction never re-runs, however many times the text is requested

#### Scenario: Bumping the profile invalidates derivatives

- **WHEN** the extraction-profile identifier is changed
- **THEN** derivatives produced under the previous profile are treated as stale and
  rebuilt on next use, and the served result identifies the profile it was produced under

#### Scenario: Forced refresh

- **WHEN** a caller explicitly requests re-extraction of an item that has a current
  derivative
- **THEN** extraction runs again, the store is updated, and the result declares itself
  freshly extracted

### Requirement: Only layout-route output may be stored

Only output produced by the layout-aware route SHALL be stored as a derivative. Flat-text
output SHALL NOT be stored, SHALL NOT overwrite or replace an existing derivative, and
SHALL NOT satisfy the condition that a derivative exists — regardless of its character
count. Serving behaviour when the layout route is unavailable is unchanged by this
capability; only storage is gated.

#### Scenario: Degraded run stores nothing

- **WHEN** extraction falls back to a flat-text engine because the layout route is
  unavailable
- **THEN** the caller is served as it is today, no derivative is stored, and the item
  remains in the "no derivative yet" state rather than acquiring a lossy one

#### Scenario: Degraded run does not overwrite a good one

- **WHEN** an item has a stored derivative and a later extraction falls back to a
  flat-text engine
- **THEN** the stored derivative is left byte-identical and continues to be served to
  readers that ask for stored content

#### Scenario: Character count does not qualify output for storage

- **WHEN** a flat-text run produces output of comparable or greater length than the
  layout route would
- **THEN** it is still not stored — length is never the storage criterion

### Requirement: Backfill of pre-existing items

Items whose PDFs were attached before this capability existed SHALL be able to acquire a
derivative in place. Backfill SHALL NOT delete and recreate items, SHALL NOT write to the
Zotero library at all, and SHALL leave item metadata — notably hand-written `extra`
content, tags, collection membership and notes — byte-identical. Backfill SHALL be
resumable and SHALL report a per-item outcome.

#### Scenario: Backfill leaves the library untouched

- **WHEN** a set of items with hand-edited `extra` content, tags and collection membership
  is backfilled
- **THEN** each gains a derivative in the store, and no write is issued to the Zotero
  library — item metadata, tags, collections and notes are unchanged

#### Scenario: Resumable, with per-item outcomes

- **WHEN** a backfill over a set of items is interrupted and re-run
- **THEN** items that already have a current derivative are not re-extracted, the
  remainder are processed, and each item is reported as stored, skipped-already-present,
  skipped-no-PDF or failed-with-reason

#### Scenario: Item with no PDF

- **WHEN** backfill reaches an item that has metadata but no PDF attachment
- **THEN** it is reported as having no PDF and skipped — not an error, and not an empty
  derivative

### Requirement: Stated search coverage

The tool descriptions and documentation SHALL state which representation full-text search
covers and where it does not apply, so a caller can tell what a negative result means.
The statement SHALL cover, at minimum: that the searched index is Zotero's own and is
never populated by this server or by any derivative; that the full-text clause applies
only to single-word queries, a multi-word query being dropped from full-text matching
entirely rather than degraded; and that matching resolves via parent items, so a top-level
attachment is not matched.

#### Scenario: Coverage is stated at the point of use

- **WHEN** a caller reads the search tool's description of its full-text option
- **THEN** it states that the index searched is Zotero's own flat index, that stored
  derivatives are not searched, that multi-word queries are excluded from full-text
  matching, and that top-level attachments are not matched

#### Scenario: A miss is interpretable

- **WHEN** a full-text search returns no hit for a term that appears inside a table in a
  stored derivative
- **THEN** the documented contract already accounts for this outcome, so the caller treats
  it as a coverage limit rather than as evidence the term is absent from the document

### Requirement: Committed fixtures gate the capability

The change SHALL ship fixtures committed to the repository — at minimum a small
table-bearing PDF and one above the whole-document page cap — and SHALL assert against
them, so the acceptance test runs on any host rather than only against one library.
Assertions SHALL be on structure (table count, page coverage, page anchors), never on
character count. Tests needing an unavailable external engine SHALL skip loudly.

#### Scenario: Gate runs without a personal library

- **WHEN** `cargo test` runs on a clean checkout with no access to any particular Zotero
  library
- **THEN** the store, staleness, serve-from-store, window-walk assembly and
  layout-only-storage tests all run against committed fixtures and pass

#### Scenario: Table fidelity asserted structurally

- **WHEN** the table-bearing fixture is extracted on the layout route
- **THEN** the count of tables in the derivative is asserted directly, and a test that
  distinguished the routes only by character count would fail to detect the difference

#### Scenario: Missing engine skips loudly

- **WHEN** `cargo test` runs on a host without the layout service
- **THEN** layout-route tests skip with a clear message, the store, staleness and backfill
  tests still run and pass, and no test passes by storing or asserting nothing
