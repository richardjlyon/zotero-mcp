## 1. Fixtures first (they gate everything else)

- [ ] 1.1 Commit a small table-bearing PDF fixture with a known table count; record the
  count in a fixture manifest so assertions reference it rather than a magic number.
- [ ] 1.2 Commit a fixture above `pdf_whole_document_max_pages` (a synthetic 60+ page
  document is enough — it need not be a scan) to exercise window-walk assembly.
- [ ] 1.3 Test helper for "layout service absent" so store/staleness/backfill tests run
  everywhere and layout-route tests skip loudly, per existing convention.

## 2. The store (D1)

- [ ] 2.1 `core/derivatives.rs`: store root resolution (platform state dir, overridable
  via a new `Config` field), created on demand. Unit test: root resolves and is writable
  on a host whose Zotero data dir is read-only.
- [ ] 2.2 Content hash of a source PDF + `EXTRACTION_PROFILE` constant sited next to the
  extraction settings it describes. Unit tests: identical bytes hash equal; a changed byte
  changes the key.
- [ ] 2.3 Store key = attachment key + content hash + profile; `get`, `put_atomic`
  (temp-file + rename, mirroring `write_cache_atomic`), `path_for`, `status_for`
  distinguishing not-present / present / failed.
- [ ] 2.4 Sidecar metadata per derivative: producing engine, profile, page count, build
  timestamp, so a served result can report its true provenance (D6 of the sister change).

## 3. Window-walk assembly (D2)

- [ ] 3.1 Lift the walk in `src/main.rs:137-170` into a reusable `build_derivative` in
  `core/pdf.rs` that walks windows, concatenates in page order and returns the assembly
  plus per-window provenance. The CLI then calls it instead of open-coding the loop.
- [ ] 3.2 Atomicity: assemble to a temp file, move into the store only when every page is
  covered. Test: an interrupted build leaves no readable derivative.
- [ ] 3.3 Failure record naming the pages that could not be extracted; a later build
  resumes rather than restarting. Test: a walk with one failing window records the failure
  and stores nothing complete.
- [ ] 3.4 Test: assembled derivative for the >cap fixture equals the concatenation of the
  same windows extracted individually — every page once, in order, anchors correct.

## 4. Serving from the store (D4)

- [ ] 4.1 `PdfTextResult` gains a served-from field (`store` vs `fresh`) plus the producing
  engine/profile from 2.4. Additive; update the wire-shape tests.
- [ ] 4.2 Store lookup in front of `extract_windowed` inside `get_pdf_text`, so the tool,
  `get_pdf_first_pages` and the CLI all benefit with no signature change. Test: two reads,
  one extraction, second declares `store`.
- [ ] 4.3 Satisfy a page-window request by slicing a stored whole-document derivative on
  its `--- p.N ---` anchors. Test: windowed read from store matches a fresh windowed
  extract and makes no engine call.
- [ ] 4.4 Storage gate: only layout-route output is written. Tests — a flat-text run stores
  nothing; a flat-text run leaves an existing derivative byte-identical; a longer flat
  result is still not stored.
- [ ] 4.5 Staleness: replaced PDF content invalidates; bumped profile invalidates; nothing
  else does. Test each, plus a forced-refresh argument that rebuilds and reports `fresh`.

## 5. Addressability

- [ ] 5.1 Derivative-path tool verb returning path + status, never text. Test: response
  size is independent of document length; the path reads back byte-identical to what the
  server serves.
- [ ] 5.2 Distinguish not-yet-extracted / extraction-failed / item-has-no-PDF in the
  response. Test all three.
- [ ] 5.3 State server-locality of the path in the tool description; check whether
  `path_map` (`tools/attachments.rs:296`) warrants a reverse mapping here or an explicit
  "server-local" statement — decide and record which.

## 6. Backfill (D5)

- [ ] 6.1 Backfill verb over a collection or list of item keys: build derivatives, issue
  **no** Zotero writes. Test asserts zero writer-client calls.
- [ ] 6.2 Per-item outcomes (stored / already-present / no-PDF / failed-with-reason) and a
  response that stays inside the MCP size ceiling — counts and statuses, never text.
- [ ] 6.3 Resumability from the store key. Test: interrupt, re-run, already-done items are
  not re-extracted.

## 7. Documentation and search coverage

- [ ] 7.1 Rewrite the `search_items` full-text description to state: Zotero's own index,
  never populated by this server or by derivatives; single-word queries only
  (`core/reader/search.rs:64`); parent-item matching only, so top-level attachments never
  match. Test asserts the description mentions all three.
- [ ] 7.2 Update `get_pdf_text` / `get_pdf_path` descriptions for store-serving and the
  new fields; update `docs/` and `CHANGELOG.md`.

## 8. Verification

- [ ] 8.1 Full `cargo test` green on a host with no layout service (store, staleness,
  serve, backfill, walk-assembly all covered; layout tests skip loudly).
- [ ] 8.2 Manual pass against the real library with a warm service: `W6U9A838` (both
  routes), `X4C6F4EF` (42 tables) and `2PGJT3N2` (24 tables) for fidelity, `U8R34XW9`
  (69 pp) for the walk, `GQTHCXPR` for the no-PDF case. Record counts in the change, not
  in the test gate.
- [ ] 8.3 Confirm on the VM (read-only Zotero mirror) that derivatives build, store and
  serve, and that nothing is written under the mirror.
