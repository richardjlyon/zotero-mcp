## Why

A cold Docling service silently downgrades a caller. Observed on `W6U9A838` (Bloomfield
et al. 2022, ESSD) on 2026-07-27: the same call minutes apart returned
`source: live_extract` / `format: plain` / no page anchors, then `source: docling` /
`format: markdown` / page anchors. Both were successes. Only a metadata field
distinguished them.

**The loss is structural, not volumetric.** Character counts were comparable (page 1:
3,496 flat vs 3,299 markdown), so no size check catches it. What the flat path drops is
tables: Docling parses 24 tables from `2PGJT3N2` and 42 from `X4C6F4EF`, the flat chain
none. In the corpus that exposed this, the load-bearing content *is* tabular — generation
capacity by technology and year, cost tables, survey distributions. A silent fallback
returns the prose, drops the numbers, and reports success.

The existing spec already requires the result to be labelled and marked `complete: false`
when a flat engine ran. That is necessary and insufficient: **correctness currently
depends on the caller noticing an optional field on an otherwise-successful response.** A
caller that reads only the text — which includes an LLM handed the tool result — will use
degraded text believing it complete, and will report "not in the document" for content
that is in the document. That is the fabrication and false-deletion risk the extraction
work exists to close, arriving through the one door left open. Because it depends on
service warmth, it is intermittent, does not reproduce reliably, and survives a spot check.

This is deliberately split from `persisted-extraction-artifacts`, which fixes the
persistence problem from the same incident. That change refuses to *store* flat text and
is non-breaking. This one changes what a caller is *served*, which is a breaking change to
a tool contract, and it lands on consumers that are mid-rewire (the `pdf-text` CLI and the
Pi `zotero-pdf-scan` skill, whose arbiter repointing is still open at task 9.4 of
`scan-and-large-pdf-extraction`). It should ship second, on its own version bump.

## What Changes

- **Flat-text output requires opt-in.** By default, extraction that can only complete via
  the flat-text chain fails loudly, naming the unavailable layout route and the opt-in as
  the remedy — rather than returning table-free text as an ordinary success. **BREAKING**
  for any caller that today receives flat text when the service is cold.
- **The opt-in is explicit and per-call**, so a caller that wants degraded text (the Pi
  arbiter walking a scan on a host with no layout route, for instance) says so and gets
  it, still labelled and still `complete: false`. The existing `plain=true` route — a
  caller deliberately asking for flat output — continues to work unchanged and is not
  affected by this change.
- **"No layout route configured" is a distinct, non-failing state.** Loud failure applies
  only when a layout route is configured and unavailable. A host with no Docling service
  at all — CI, and any machine without the GPU host — behaves as it does today, so the
  golden-set requirement that flat-text tests still run and pass continues to hold.
- **The degradation marker stays out of the extracted text.** It is carried in the result
  envelope and in the error, never injected into the document text. The text is the
  arbiter that downstream fact-checking reads; adding a banner to it would put sentences
  into the artefact that are not in the document. This reverses an earlier draft of this
  requirement.
- **A migration note ships with it**, naming the version, the opt-in argument, and what the
  two known consumers must do.
- **A table-fidelity regression** over committed fixtures asserts what actually
  distinguishes the routes — tables present versus absent, asserted structurally, never by
  character count.

Non-goals: no change to extraction itself (windowing, OCR, formula enrichment, page
anchors all stand); no change to the completeness report's contents; no change to
`plain=true`; nothing about storage or persistence.

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `pdf-extraction`: tightens "Labelled route and preserved fallback" so a flat-text
  fallback is not deliverable as an unremarkable success — default-deny with an explicit
  per-call opt-in, scoped to the case where a layout route is configured but unavailable.
  Adds a table-fidelity regression requirement over committed fixtures.

## Impact

- **Code**: `crates/zotero-mcp/src/core/pdf.rs` (route selection and the fallback delivery
  decision, ~`:1370` onward; a new error variant for layout-route-unavailable);
  `src/tools/attachments.rs` (`PdfTextArgs` gains the opt-in; tool description);
  `src/server.rs` (tool description); `src/main.rs` (the CLI must choose and state its
  posture); `CHANGELOG.md` and `docs/`.
- **APIs/contracts**: **BREAKING**. A call that today returns flat text as success will,
  by default, return an error on a host with a configured-but-unreachable layout route.
  Ships on its own minor version bump with a migration note. Additive opt-in argument;
  no existing argument changes meaning.
- **Known consumers to migrate**: the `zotero-mcp pdf-text` CLI subcommand and the Pi
  `zotero-pdf-scan` skill (both shipped by `scan-and-large-pdf-extraction`). The Pi host
  has no layout route configured, so it lands in the non-failing "not configured" state —
  but this must be verified, not assumed, before the change ships.
- **Sequencing**: ships after `persisted-extraction-artifacts`, and after task 9.4 of
  `scan-and-large-pdf-extraction` (repointing `zref`'s arbiter) is closed.
- **Tests / fixtures**: reuses the committed table-bearing fixtures from
  `persisted-extraction-artifacts`; adds coverage for all three postures — layout route
  absent, configured-and-cold without opt-in, configured-and-cold with opt-in.
