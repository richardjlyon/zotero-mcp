## Context

`extract_core` (`core/pdf.rs:1356`) tries the layout route first and falls through to the
flat-text chain on any health or convert failure:

```rust
if let Some(docling) = engines.docling().filter(|_| !plain) {
    if docling.healthy().await { ... }
}
// falls through to .zotero-ft-cache -> pdf-extract -> pdftotext
```

Two distinct situations reach the same fall-through. `engines.docling()` is `None` when no
service is configured — normal on CI, on the Pi, on any host without the GPU box. It is
`Some` but unhealthy when a service *is* configured and is cold or down. Today both
produce an identical labelled, `complete: false` success. Only the second is a problem;
conflating them would break every host that has no layout route at all.

The two known consumers both read `r.text` directly: the `pdf-text` CLI prints it to
stdout and walks windows on `PdfDocumentTooLarge` (`src/main.rs:118-170`), and the Pi
`zotero-pdf-scan` skill consumes that stdout as the arbiter text for fact-checking.

## Goals / Non-Goals

**Goals:**

- A caller reading only the text cannot mistake flat-text output for layout-faithful
  output.
- Hosts with no layout route configured keep working exactly as now.
- The extracted text stays free of server-added prose.
- The break is specified: version, argument name, and what each known consumer does.

**Non-Goals:**

- Changing extraction itself, the completeness report's contents, or `plain=true`.
- Anything about storage — that is `persisted-extraction-artifacts`.
- Retrying or warming a cold service.

## Decisions

### D1 — Three postures, distinguished at the point of route selection

The gate lives where the route is already chosen, in `extract_core`:

| Layout route | Opt-in | Outcome |
|---|---|---|
| not configured (`docling()` is `None`) | n/a | flat chain, labelled, `complete: false` — unchanged |
| configured, healthy | n/a | layout route, as now |
| configured, unhealthy | absent | **loud error** naming the route and the opt-in |
| configured, unhealthy | present | flat chain, labelled, `complete: false` |
| any, `plain=true` | n/a | flat chain — a deliberate caller choice, unchanged |

The distinction is exactly `engines.docling().is_some()`, which the code already computes;
no new probing and no new configuration.

### D2 — A new error variant, not a repurposed one

`Error::LayoutRouteUnavailable { path, endpoint }`, rendering a message that names the
configured endpoint, states that flat-text output cannot express tables, and names the
opt-in argument as the remedy. It sits beside `PdfDocumentTooLarge`
(`core/error.rs:120`), which is the precedent: a loud, actionable refusal rather than a
degraded success. Reusing an existing variant would make the two indistinguishable to the
CLI's match arms.

### D3 — Opt-in argument: `allow_degraded`, default false, on `PdfTextArgs` and `FirstPagesArgs`

Named for what the caller is accepting rather than for the engine, so it stays honest if
the route stack changes. It is *not* `plain`: `plain=true` means "give me flat output on
purpose" and already works; `allow_degraded` means "I would rather have flat output than
nothing if the layout route is down". Both may be set; `plain` short-circuits first.

### D4 — The marker stays out of the text

Degradation is reported through `source`, `completeness` and the error — never appended
to `r.text`. An earlier draft of this change required an in-band banner; that was wrong.
The CLI prints `r.text` verbatim to stdout and the Pi skill fact-checks against that
stdout, so a banner would insert sentences into the arbiter that are not in the document
— manufacturing exactly the kind of false content the extraction work exists to prevent.
The CLI's existing stderr header line is the right channel and already carries `route` and
`complete`.

### D5 — CLI posture: default-deny, with a flag, and a walk that stops on the first refusal

`pdf-text` gains `--allow-degraded` and passes it through. Without it, a
`LayoutRouteUnavailable` aborts with the error on stderr and a non-zero exit, so a
scripted arbiter fetch fails visibly rather than emitting table-free text that looks fine.
In a window walk, the first refused window aborts the walk — a partial stdout stream with
a silent hole is worse than no stream. The Pi host has no layout route configured, so it
lands in the unchanged first row of D1; this is to be **verified on the host**, not
assumed, before the change ships.

### D6 — Serving a stored derivative reports the route that produced it

Once `persisted-extraction-artifacts` lands, a served derivative was produced by a route
that ran earlier. The result must report the engine and profile that actually produced the
bytes, not the engine that would run now. A store hit is never subject to this gate: it is
by construction layout-faithful, since only layout output is ever stored.

### D7 — Sequencing and version

Ships after `persisted-extraction-artifacts` and after task 9.4 of
`scan-and-large-pdf-extraction` (repointing `zref`'s arbiter) is closed, on its own minor
version bump, with a CHANGELOG entry naming the argument and the affected callers. Landing
a breaking change on the arbiter while its wiring is in flight would make any downstream
failure ambiguous between the two changes.

## Risks / Trade-offs

- **A configured-but-flaky service turns transient into fatal.** A caller who previously
  got degraded text now gets an error. That is the intent, but it makes availability of
  the GPU host a hard dependency for anyone who has configured it. Mitigation: the opt-in
  is per call, so a caller who genuinely prefers degraded text over nothing has a
  one-argument escape.
- **Misconfiguration is now load-bearing.** A host with a stale endpoint in its config is
  "configured but unavailable" and fails loudly, where before it silently worked. This is
  arguably an improvement — a stale endpoint should be noticed — but it will surface as a
  breakage on first upgrade.
- **The `plain` / `allow_degraded` pair is a subtle distinction** and will be got wrong by
  callers. Mitigation: the tool description states both in one sentence, and the error
  message names `allow_degraded` explicitly rather than describing it.
- **Aborting a window walk loses completed windows.** A 400-page walk that fails at window
  18 discards 17 windows of good text. Accepted: the alternative is a stream with an
  unmarked gap. Once the store exists, completed windows survive in it anyway.
