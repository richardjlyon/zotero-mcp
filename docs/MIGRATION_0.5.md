# Migrating to 0.5.0 — degraded extraction is now an error

One behavioural change needs a decision from you. Everything else in 0.5.0 is
additive.

## What changed

When the layout-aware extraction route (Docling) is **configured on the server
but unavailable** — cold, down, or a convert that fails — `get_pdf_text` and
`get_pdf_first_pages` now return an error instead of quietly falling back to
flat text.

Before, both outcomes were successes. The only thing distinguishing them was a
metadata field:

| | cold service | warm service |
|---|---|---|
| `source` | `live_extract` | `docling` |
| `format` | `plain` | `markdown` |
| `page_anchors` | false | true |
| tables in output | **none** | all of them |
| HTTP result | success | success |

The loss is structural, not volumetric — on the case that prompted this, the
flat run produced *more* characters (3,496 vs 3,299 on page one) with every
table gone. So no size check catches it, and because it depends on service
warmth it is intermittent and survives a spot check. A caller reading only the
text will use table-free output believing it complete, and report "not in the
document" for content that is in the document.

## What you need to do

**Nothing, if your host has no layout route configured.** CI, and any machine
without `DOCLING_URL` or `docling_url` set, behave exactly as before: the
flat-text chain runs, is labelled, and is marked incomplete. The new error can
only occur where a layout route was configured and was expected to work.

**If you have a layout route configured**, decide per caller:

- *Correctness matters more than availability* (fact-checking, citation work,
  anything where a missing table becomes a false claim): do nothing. A cold
  service now fails loudly and you retry when it is back.
- *Some text beats no text*: pass `allow_degraded=true`. The flat chain runs as
  before, the result is labelled, and `completeness.complete` is false.
- *You wanted flat output all along*: keep using `plain=true`. It is unchanged
  and was never gated — it means "give me flat output on purpose", which is a
  different thing from tolerating a degraded substitute.

On the CLI: `zotero-mcp pdf-text` gains `--allow-degraded`. Without it, a
refusal prints the error to stderr and exits non-zero rather than emitting
table-free text that looks fine.

## Two things that are deliberately *not* affected

- **The extracted text carries no server-added warning.** Degradation is
  reported in the result envelope and in the error, never inside the document
  text — that text is what downstream fact-checking treats as authoritative,
  and a banner inside it would insert sentences that are not in the document.
- **A stored derivative is served regardless.** Once a document has been
  extracted layout-faithfully, later reads come from the durable store and a
  cold service cannot make it unreadable.

## Known consumers

- `zotero-mcp pdf-text` — the CLI above.
- The Pi `zotero-pdf-scan` skill, which reads that CLI's stdout as its
  fact-check arbiter. The Pi host has no layout route configured, so it lands
  in the unchanged posture — **verify this on the host before upgrading it**
  rather than assuming it.
