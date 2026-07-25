# Decision brief: Slice G's unresolved wire-format question

**Status:** OPTIONS — awaiting Richard's decision. No tool migrated until it is made.
**Author:** Claude Opus 5.
**Date:** 2026-07-25.
**Prior work:** `docs/superpowers/specs/2026-05-13-dependency-upgrades-slice-g-design.md` (see its "Implementation result" section), `docs/superpowers/plans/2026-05-13-dependency-upgrades-slice-g.md`, `2026-05-13-tool-output-normalisation-design.md`.

---

## Why this is a decision and not a task

Slice G migrated 10 of 22 tools to a strict typed output (`Json<T>`), which gives each tool an `outputSchema` on `tools/list` and a `structured_content` field on every response. The remaining 12 stalled on one question the slice could not answer for itself, and the stakes are not academic: a malformed schema has already cost this server its entire tool list once. Claude Code's validator rejected the `tools/list` response wholesale, so the server showed as *connected with zero usable tools* — a failure mode that looks like a broken connection rather than a bad schema.

The 12 split into two groups with different problems.

**Group 1 — nine list-returning tools:** `search_items`, `list_recent_items`, `list_collections`, `list_tags`, `list_attachments`, `list_annotations`, `search_crossref`, `search_semantic_scholar`, `find_weak_metadata_items`. Each returns a JSON array.

**Group 2 — three lookup tools:** `lookup_doi`, `lookup_isbn`, `lookup_arxiv`. Each returns an object whose *shape* depends on the `format` argument.

---

## The constraint, verified today against the rmcp 1.7 source

The MCP spec requires a tool's `outputSchema` to have a root `"type": "object"`. rmcp enforces it and the enforcement is a **panic at startup**, not a warning: `handler/server/common.rs:66` (`schema_for_output`) returns an error for any other root type, and `router/tool/tool_traits.rs:61` unwraps that with `panic!`. So a wrong choice here does not degrade the server — it stops it booting.

Three further facts, each checked rather than assumed (I ran a throwaway schema probe to confirm, then deleted it):

1. `schemars` renders `Vec<T>` as `"type": "array"`. Every Group 1 tool therefore panics if migrated as-is. This part of the old spec is correct.
2. `serde_json::Value` produces a schema with **no root `type` at all** — also fatal. But `serde_json::Map<String, Value>` produces `{"type": "object", "additionalProperties": true}`, which passes. **This unblocks Group 2 without the `format`-parameter redesign the old spec assumed was needed** (see "Group 2" below).
3. `Content::json(v)` and `CallToolResult::structured(v)`'s `content[0]` are byte-identical for the same value (`v.to_string()` in both cases). So the *only* wire difference an envelope introduces is the extra `{"items": …}` layer around the array — no encoding change, no ordering change.

I also found something the old spec missed: `#[tool(output_schema = <expr>)]` accepts an explicit schema expression, independent of the return type (`rmcp-macros-1.7.0/src/tool.rs:307`). That makes Option 3 below possible, where the old spec assumed the return type was the only route.

---

## Group 1: the nine list tools — three real options

### Option 1 — Uniform envelope, accept the `content[0]` shape change *(recommended)*

One generic type used by all nine:

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ListResult<T> {
    pub items: Vec<T>,
    /// How many items this response carries.
    pub count: usize,
    /// True when `count` reached the requested limit, so there may be more.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub possibly_truncated: bool,
}
```

`search_items` returns `Json<ListResult<SearchHit>>`, and so on. Verified: this produces a root `type: object` schema and passes rmcp's check.

**What changes on the wire.** `content[0].text` goes from `[{…},{…}]` to `{"items":[{…},{…}],"count":2}` for these nine tools. `structured_content` gains the same object. `outputSchema` appears on `tools/list`.

**What it buys beyond consistency.** A real gap closes: `search_items` today gives no indication whether its `limit` truncated the results — an empty-looking answer and a capped answer are indistinguishable. `count` and `possibly_truncated` fix that, which is worth having on its own terms. The forced wrapper becomes a genuine improvement rather than ceremony.

**What it costs.** Any consumer parsing `content[0].text` as an array breaks. Who actually does that? The MCP clients here are Claude Code and Cowork, and both hand tool content to the model rather than parsing it in code — the model reads `{"items": …}` as easily as `[…]`, and now also gets a schema telling it the shape in advance. I found no code in this repo or the docs that parses tool content programmatically. The old spec called this "risk of silent breakage in production", which I think overstated it: the risk is real in principle but has no identified consumer. It is nonetheless the one thing that cannot be un-shipped quietly, which is why it is your call and not mine.

### Option 2 — Leave the nine on `CallToolResult` permanently

Change nothing for Group 1; declare the current split the intended end state and close Slice G as "10 typed, 9 deliberately untyped".

**For:** zero risk, zero work, no wire change ever.
**Against:** the nine include the most-used tools in the server (`search_items` above all), so the tools whose output shape a client would most benefit from knowing in advance are precisely the ones left undescribed. The inconsistency also stays a standing invitation for someone to "finish the migration" later without re-deriving this analysis — the exact loop we are in now.

### Option 3 — Bare array in `content`, envelope in `structured_content`

Hand-roll an `IntoCallToolResult` impl that puts the unchanged bare array in `content[0].text` while setting `structured_content` to `{"items": …}`, and attach the envelope schema via `#[tool(output_schema = …)]`.

**For:** no wire change at all *and* an `outputSchema`. On paper it dominates Option 1.
**Against:** `content[0]` and `structured_content` would then deliberately disagree — MCP says the content block SHOULD carry the serialised structured content for backwards compatibility, so this is a knowing deviation from the spec rather than a clever use of it. A client that cross-checks the two, or a future rmcp version that asserts the relationship, breaks in a way that is hard to diagnose. It also means nine hand-built schemas and a bespoke wrapper type instead of one derive — more code in exactly the area where a mistake takes the whole tool list down. I would not choose deliberate spec divergence to avoid a shape change with no identified consumer.

---

## Group 2: the three lookup tools — no longer blocked, but one thing to weigh

The old spec deferred these pending a redesign of the `format` argument. That turns out to be unnecessary: both `format` variants serialise to a JSON object, and `Json<Map<String, Value>>` passes rmcp's object-root check (fact 2 above). They could migrate today with **no wire change whatsoever** — the payload is the same object, and the schema would be the honest but uninformative `{"type": "object", "additionalProperties": true}`.

The catch is an interaction with the lookup-resilience work that landed today. Those tools now return a *structured failure* on total failure: an error result whose body is a `lookup_failed` object carrying the attempt trail and a `suggestion` the caller branches on. That body lives in the tool's content, which requires returning `CallToolResult`. Migrating to `Json<T>` forces the failure onto a different channel — `ErrorData.data`, whose visibility to the model is client-dependent and which I have not verified for Claude Code.

**My recommendation for Group 2: leave all three on `CallToolResult`, deliberately and documented.** The schema they would gain advertises "some object", which tells a client nothing it did not already assume, while the structured failure body tells the model exactly what to do next. Trading a real capability for an empty schema is a bad trade. This is a decision to *not* migrate rather than a deferral — worth writing down as such, so it stops resurfacing as unfinished business.

If you would rather have all 22 uniformly typed, the honest cost is: verify whether Claude Code surfaces `ErrorData.data` to the model, and if it does not, the resilience work's `suggestion` becomes invisible and the skill loses the branch it was built for.

---

## Recommendation in one paragraph

Take **Option 1** for the nine list tools — one `ListResult<T>` envelope, uniform `items` / `count` / `possibly_truncated`, accepting that `content[0].text` gains an object wrapper for those nine — and **leave the three lookup tools as they are**, recording that as a decision rather than a deferral. That ends Slice G at 19 of 22 tools strictly typed, with the remaining three untyped for a stated reason. The truncation signal is worth having regardless, the model reads either shape without difficulty, and no identified consumer parses the old shape.

## What I would do alongside it, whichever option you pick

The failure that motivated all this — one bad schema silently costing the entire tool list — is still not guarded against. `tests/schema_shape.rs` checks a handful of types by hand. A single test that walks **every** registered tool and asserts each input and output schema has an object root would have caught the original incident at `cargo test` rather than in production. Cheap, and it makes any future migration safe to attempt. I would add it as the first step of the migration rather than the last.

---

## Verified references

| Claim | Where |
|---|---|
| `outputSchema` must have root `type: object`, enforced by panic | `rmcp-1.7.0/src/handler/server/common.rs:66`, `router/tool/tool_traits.rs:61` |
| `Vec<T>` → `"type": "array"`; `Map<String,Value>` → `"type": "object"`; `Value` → no `type` | schema probe run 2026-07-25 |
| `Content::json(v)` ≡ `content[0]` of `CallToolResult::structured(v)` | `rmcp-1.7.0/src/model.rs:2864`, probe |
| Explicit schema override exists | `rmcp-macros-1.7.0/src/tool.rs:307` (`output_schema` attribute) |
| 10 tools already migrated; 12 outstanding | `2026-05-13-dependency-upgrades-slice-g-design.md` §"Implementation result" |
