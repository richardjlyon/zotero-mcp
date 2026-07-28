## 0. Preconditions

- [ ] 0.1 Confirm `persisted-extraction-artifacts` has shipped, and that task 9.4 of
  `scan-and-large-pdf-extraction` (repointing `zref`'s arbiter) is closed. Do not start
  before both — a break landing on an arbiter mid-rewire makes any failure ambiguous.
- [ ] 0.2 Verify on the Pi host that no layout route is configured, so it lands in the
  unchanged posture. Record the check; do not assume it.

## 1. The three postures (D1, D2)

- [x] 1.1 `Error::LayoutRouteUnavailable { path, endpoint }` beside `PdfDocumentTooLarge`
  (`core/error.rs:120`); message names the endpoint, says a flat engine cannot express
  tables, and names `allow_degraded` as the remedy.
- [x] 1.2 Gate in `extract_core` (`core/pdf.rs:1370`): configured-and-unhealthy without
  opt-in returns the new error; every other posture is unchanged. Distinguish on
  `engines.docling().is_some()` — no new probing, no new config.
- [x] 1.3 Tests for all five rows of the D1 table, including `plain=true` succeeding
  without the opt-in and no-route-configured succeeding without it.

## 2. The opt-in (D3)

- [x] 2.1 `allow_degraded: bool` (default false) on `PdfTextArgs` and `FirstPagesArgs`,
  threaded to `extract_core`. Update tool descriptions to state the `plain` /
  `allow_degraded` distinction in one sentence.
- [x] 2.2 Wire-shape and tool-surface tests updated for the new argument.
- [x] 2.3 Test: with the opt-in set on an unhealthy configured route, the flat chain runs,
  is labelled, and reports `complete: false` exactly as before.

## 3. The text stays clean (D4)

- [x] 3.1 Test: no degraded path appends any marker, banner or warning to `r.text` — the
  text contains only document content, on every posture.
- [x] 3.2 Confirm the CLI's stderr header remains the degradation channel and already
  carries route and completeness (`src/main.rs`), and that stdout is untouched.

## 4. CLI posture (D5)

- [x] 4.1 `pdf-text --allow-degraded`, passed through; without it a
  `LayoutRouteUnavailable` prints the error to stderr and exits non-zero.
- [x] 4.2 Window walk aborts on the first refused window rather than streaming a stream
  with an unmarked hole. Test the abort and the exit code.

## 5. Store interaction (D6)

- [x] 5.1 A store hit bypasses the gate entirely (stored content is layout-faithful by
  construction) and reports the engine/profile that produced it, not the engine that would
  run now. Test: cold service + current derivative still serves successfully.

## 6. Migration and release (D7)

- [x] 6.1 CHANGELOG entry marked BREAKING, naming the argument, the new error, and the
  two known consumers.
- [x] 6.2 Migration note in `docs/`: what changes for a caller on a host with a configured
  layout route, and the one-argument escape.
- [x] 6.3 Minor version bump; ship separately from the persistence change.

## 7. Regression suite

- [x] 7.1 Table-fidelity regression over the committed fixtures from the sister change:
  assert table count and structure, never character count.
- [x] 7.2 Test that a character-count-only comparison would fail to distinguish the routes
  — the guard against the 3,496-vs-3,299 trap.
- [x] 7.3 `cargo test` green on a host with no layout service; layout-dependent cases skip
  loudly.

## 8. Verification

- [x] 8.1 Covered by `tests/degradation_contract.rs`, which simulates the
  configured-but-unavailable posture with a dead endpoint so all three postures are
  asserted on any host. Confirmation against the real service being stopped is still
  worth doing at install time.
- [ ] 8.2 Confirm the Pi arbiter path is unaffected in practice, not just in theory.
