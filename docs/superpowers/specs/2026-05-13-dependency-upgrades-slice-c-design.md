# Spec: Dependency upgrades — Slice C (rmcp + schemars joint bump, minimal port)

**Status:** Approved design, ready for plan-writing.
**Author:** Richard Lyon (with Claude Opus 4.7).
**Date:** 2026-05-13.
**Goal:** Bring `rmcp 0.1.5 → 1.7.0` and `schemars 0.8 → 1.x` to their latest releases in a single atomic commit. Minimal port — convert the `#[tool(...)]` macro syntax and verify the 27 `#[derive(JsonSchema)]` derives still compile and the one schemars-API consumer (`tests/schema_shape.rs`) is ported. No new feature adoption on this slice: `oauth.rs` and `http_transport.rs` stay as-is.

---

## Problem

Slice A's spec (commit `6f75756`) scoped four follow-on slices:

- **Slice B:** `reqwest 0.12 → 0.13`, `toml 0.8 → 1` — completed (`c4a6c41`).
- **Slice C:** `schemars 0.8 → 1.x` — assumed independent.
- **Slice D:** `rmcp 0.1 → 1.x` — assumed independent.
- **Deferred:** `rusqlite 0.38 → 0.39` (blocked by `deadpool-sqlite 0.13`).

Research during Slice C brainstorming surfaced a coupling Slice A's spec missed: `rmcp 0.1.5` requires `schemars ^0.8` and its `#[tool]` macros consume `schemars 0.8::JsonSchema` from the codebase's `Args` structs (used in `crates/zotero-mcp/src/server.rs` and via `#[derive(JsonSchema)]` across `tools/{enrichment,citations,attachments,writes,search}.rs`). `rmcp 1.7.0` requires `schemars ^1.0`. Bumping schemars alone would derive `schemars 1.x::JsonSchema` on the Args structs while rmcp's macros still expect the `schemars 0.8` trait — a compile-time trait mismatch with no clean isolation path.

The two slices are therefore merged: this slice (renamed "Slice C") performs the joint upgrade. The original Slice D as a separate concept is folded in.

A 2026-05-13 dependency surface inventory (via Explore agent) confirmed the touch surface:

- **69 `#[tool(...)]` attribute sites** in `crates/zotero-mcp/src/server.rs` (1 struct-level `tool_box`, 1 struct-level on the `ServerHandler` impl, 40 method-level `description = "..."` attrs, 27 parameter-level `aggr` attrs).
- **27 `#[derive(JsonSchema)]` sites** across `tools/attachments.rs` (7), `tools/enrichment.rs` (8), `tools/citations.rs` (2), `tools/search.rs` (5), `tools/writes.rs` (5). **No field-level `#[schemars(...)]` attributes** — all customisation is via `#[serde(...)]`.
- **No custom `JsonSchema` impls.** All schemars usage is derive-based.
- **One direct schemars API consumer:** `tests/schema_shape.rs` uses `schemars::schema_for!` and navigates the returned schema's structure — this is the one site that must be ported by hand under schemars 1.0's `Schema = serde_json::Value` wrapper API.

---

## Decisions

1. **Single atomic commit.** Joint bumps cannot be bisected per-crate (the version constraint between rmcp and schemars makes any intermediate state un-buildable), so attempting to split adds risk without benefit. Message format:

   ```
   chore(deps): bump rmcp 0.1 → 1.7 (jointly with schemars 0.8 → 1)
   ```

   Body covers: macro-shape migration summary (X → Y forms), schemars derive verification, test-count delta vs Slice B baseline, lockfile churn.

2. **Test gate.** `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp` must both pass before the commit lands. The current lib-test baseline (after Slice B) is `105 passed; 0 failed`. The 3 tests in `tests/schema_shape.rs` may need rewriting under the new `Schema = serde_json::Value` API — if their shape changes, establish a new baseline rather than treating drift as regression. State the new lib-test count explicitly in the commit body.

3. **Minimal port philosophy.** No new feature adoption. `oauth.rs` (~700 lines, OAuth 2.1 + PKCE + RFC 9728) and `http_transport.rs` (~150 lines, custom Axum SSE/HTTP transport) stay as-is. rmcp 1.x's native `auth` and `transport-streamable-http-server` features are explicitly out of scope — see the "Deferred to later slices" section for the planned follow-ups.

4. **Escalation rule** (tighter than Slice A/B, because joint bumps cannot be partially landed). If any of these surface during implementation, revert the entire slice (`git checkout -- Cargo.toml crates/zotero-mcp/Cargo.toml Cargo.lock <touched sources>`) and add a "Deferred (rmcp+schemars joint bump)" entry to this spec with the specific blocker named:

   - `ServerHandler` trait signature change in rmcp 1.x requiring redesign of the manual `list_resources` / `read_resource` block in `server.rs`.
   - `#[tool]` macro output incompatible with the current async return shape `Result<CallToolResult, McpError>`.
   - schemars 1.x cannot derive `JsonSchema` on `Vec<serde_json::Map<String, Value>>` (used in `ProposeArgs.candidates` and `EnrichArgs.candidates` in `tools/enrichment.rs`).
   - rmcp 1.x's transport bounds require a `Send + 'static` constraint that `http_transport.rs`'s tuple transport doesn't satisfy.
   - Anything that requires touching `oauth.rs` or `http_transport.rs` beyond a verify-and-confirm pass (because that lives in deferred slices).

   Accept defeat early. A joint bump that escalates mid-way is more painful than one that's deferred cleanly.

5. **No server reinstall during the slice.** Reinstall + launchd restart only at the very end (matches Slice A/B pattern).

6. **Workspace-vs-crate placement.** `rmcp` lives in workspace deps (`Cargo.toml` line 21); `schemars` lives in `crates/zotero-mcp/Cargo.toml` (per Slice A's decision 7). Both files are edited in this slice.

7. **rmcp feature flags.** rmcp 1.x split `schemars` out of the `server` feature into its own opt-in feature. The new workspace pin must enable it explicitly:

   ```toml
   rmcp = { version = "1", features = ["server", "macros", "schemars", "transport-io"] }
   ```

   (`macros` was implicit via `server`'s defaults in 0.1; pinning explicitly so future feature-rearrangements don't silently disable the `#[tool]` macros. `transport-io` carries forward unchanged.)

---

## Migration plan

The implementation discovers exact diffs by compile-error iteration. This plan documents the shape of the work, not the exact edits.

### 1. `Cargo.toml` (workspace, line 21)

Bump:

```toml
rmcp = { version = "0.1", features = ["server", "transport-io"] }
```

to:

```toml
rmcp = { version = "1", features = ["server", "macros", "schemars", "transport-io"] }
```

### 2. `crates/zotero-mcp/Cargo.toml`

Bump `schemars = "0.8"` to `schemars = "1"`.

### 3. `crates/zotero-mcp/src/server.rs` (largest single-file change)

Three macro-shape conversions across 69 attribute sites:

| Before (rmcp 0.1) | After (rmcp 1.x) | Approximate sites |
|---|---|---|
| `#[tool(tool_box)]` on `impl ZoteroServer { ... }` | `#[tool_router]` (struct-level) | 1 |
| `#[tool(description = "...")]` on each handler method | `#[tool(description = "...")]` (unchanged) | 40 |
| `#[tool(aggr)] args: SearchArgs` (parameter attribute) | `Parameters(args): Parameters<SearchArgs>` (Rust pattern destructure — not an attribute) | 27 |
| `impl ServerHandler for ZoteroServer { /* custom get_info, list_resources, read_resource */ }` | `#[tool_handler] impl ServerHandler for ZoteroServer { /* custom list_resources, read_resource — get_info auto-generated by macro */ }` — IF `#[tool_handler]` permits manual method overrides; otherwise keep the manual block as-is, non-blocking. | 1 block |

The `Parameters<T>` conversion is mechanical but invasive — every handler signature changes its parameter pattern. Discovery: compile fails on the first un-ported site, the fix is applied uniformly across the file in one pass.

### 4. `crates/zotero-mcp/src/main.rs`

Verify-only. The runtime wiring at `main.rs:43-83` uses `ServiceExt::serve` (stdio path) and `ServiceExt::serve` + `.waiting` (HTTP path). These are unchanged in rmcp 1.x per the migration docs. If the import path of `ServiceExt` shifted, update the `use rmcp::...` line. No structural changes expected.

### 5. `crates/zotero-mcp/src/http_transport.rs`

Verify-only. Confirms that:

- `rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage}` are still present at those paths.
- The tuple transport `(PollSender<ServerJsonRpcMessage>, ReceiverStream<ClientJsonRpcMessage>)` still satisfies `serve()`'s generic bounds.
- The per-session `tokio::spawn` + `.serve().await? + .waiting().await?` pattern still works.

If `ClientJsonRpcMessage`/`ServerJsonRpcMessage` were renamed, update the `use` line — single-line, mechanical, still in-bounds. Anything beyond that (e.g. transport now requires a wrapper type, not a raw tuple) triggers escalation per decision 4.

### 6. `crates/zotero-mcp/src/tools/{search,attachments,citations,enrichment,writes}.rs` (5 files)

27 bare `#[derive(JsonSchema)]` sites should compile unchanged under schemars 1.x. The `use schemars::JsonSchema` imports are unchanged. Verify-only. Anything else (e.g. schemars 1.x refuses `Vec<serde_json::Map<String, Value>>`) triggers escalation.

### 7. The 5 tool modules' use of `rmcp::Error`

Each of `tools/{attachments,citations,enrichment,search,writes}.rs` uses `rmcp::Error::{internal_error, invalid_params}` constructors. Per rmcp 1.5 docs, the `Error` type is still at `rmcp::Error` and these constructors are stable. Verify-only.

### 8. `crates/zotero-mcp/tests/schema_shape.rs`

The one hands-on schemars port. Pattern shifts from typed-struct accessors to JSON Value navigation:

**Before (schemars 0.8):**
```rust
let schema = schema_for!(CreateItemArgs);
assert_eq!(schema.schema.instance_type, Some(InstanceType::Object.into()));
// then field access on the typed schema struct
```

**After (schemars 1.0):**
```rust
let schema = schema_for!(CreateItemArgs);
assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
// then JSON Value navigation
```

3 tests need this transform. They may end up shorter (Value navigation is more concise than typed-struct chains).

### 9. `Cargo.lock`

Joint update via `cargo update -p rmcp -p schemars`. Expect: rmcp 0.1.5 → 1.7.0; rmcp-macros 0.1.5 → matching 1.x; schemars 0.8.22 → 1.x. Possible transitives: `paste` → `pastey` (rmcp's macros switched), new `oauth2`/`url` if `auth` were enabled (it isn't, so not expected), `tokio-stream` may bump.

---

## Risks

1. **Macro-port mass.** 69 attribute sites is the largest single-file diff so far in this dep-upgrade campaign. The `aggr` → `Parameters` conversion changes parameter syntax, not just an attribute spelling — every handler signature shifts. Mitigation: do the conversion in one pass, lean on the compiler.

2. **`#[tool_handler]` compatibility with manual `ServerHandler` methods.** Unknown whether the macro permits override methods like the codebase's `list_resources` and `read_resource`. If it doesn't, keep the manual `impl ServerHandler` block — `get_info()` is small enough to hand-write. Non-blocking.

3. **schemars derive on `serde_json::Value` types.** `ProposeArgs.candidates: Vec<serde_json::Map<String, Value>>` and `EnrichArgs.candidates: Vec<serde_json::Map<String, Value>>` are the tricky shapes. schemars 0.8 generates `additionalProperties: true` for these. If 1.x fails or produces semantically-different output that breaks downstream MCP clients, escalate.

4. **Schema shape test fragility.** The 3 tests in `tests/schema_shape.rs` may pass at a new equilibrium but produce different schema JSON than before. The tests check structural assertions, not byte-equivalence — semantic stability is sufficient. If the assertions need to be relaxed because the new shape is reasonable, do so and note in the commit body.

5. **Transport tuple bounds.** rmcp 1.x's `ServiceExt::serve` may require `Sink: Send + 'static` or similar where 0.1 required less. The per-session `tokio::spawn` in `http_transport.rs` already runs on a multi-threaded runtime, so `Send + 'static` is likely already satisfied. Verify at compile time.

6. **rmcp 1.x repository move.** Per Slice A's spec, rmcp moved from `4t145/rmcp` to `modelcontextprotocol/rust-sdk`. The crate name and crates.io publishing are unchanged, but the `homepage`/`repository` fields shift. No code impact; informational only.

---

## Out of scope (deferred to later slices)

These are documented here so the next slice doesn't re-derive the decision.

- **Slice D: Researched and abandoned (2026-05-13).** The original plan was to adopt rmcp 1.x's `auth` feature to replace `crates/zotero-mcp/src/oauth.rs` (~1280 lines + 657 lines in `oauth/token_store.rs`). A direct read of `rmcp-1.7.0/src/transport/auth.rs` showed the entire `auth` module is client-side: it provides `AuthClient<C>`, `AuthorizationManager` (with `get_access_token`, `configure_client_id`, `with_client`), `AuthorizationSession::get_authorization_url`, `OAuthClientConfig::with_client_secret`, `CredentialStore` / `StateStore` for an MCP *client*'s received tokens and PKCE state, and `WWWAuthenticateParams` for *parsing* WWW-Authenticate response headers. There is no server-side authorization endpoint, token endpoint, or bearer-validation middleware — `grep -r 'oauth\|OAuth\|bearer\|authoriz'` across the whole rmcp 1.7 source returns only client-side files. zotero-mcp is the OAuth *provider* (it issues tokens that Claude Desktop and other MCP clients authenticate with), so the direction mismatches. rmcp 1.x cannot replace oauth.rs. If the maintenance burden of the hand-rolled OAuth ever becomes pressing, a future investigation should look at server-side OAuth provider crates (e.g. `oxide-auth`, custom axum-based scaffolding) rather than rmcp.

- **Slice E (queued, premise VERIFIED 2026-05-13): Adopt rmcp 1.x's `transport-streamable-http-server` feature** to replace `crates/zotero-mcp/src/http_transport.rs` (~150 lines). Direct read of the rmcp 1.7.0 source confirmed the replacement is real and substantial:
    - `rmcp::transport::streamable_http_server::tower::StreamableHttpService<S, M>` is a `tower_service::Service<Request<RequestBody>>` — drops directly into an axum HTTP server as a tower layer.
    - `StreamableHttpServerConfig` covers everything `http_transport.rs` does by hand (SSE keep-alive, per-session service factory via `service_factory: Arc<dyn Fn() -> Result<S, std::io::Error>>`, cancellation token) **plus features the hand-rolled version lacks**: `allowed_hosts` DNS-rebinding protection (defaults to loopback-only), and `json_response` mode (skips SSE framing for simple request-response tools per MCP 2025-06-18 spec).
    - Pluggable `SessionManager` trait with three reference implementations at `session/{local,never,store}.rs` — matches the current `Arc<RwLock<HashMap<SessionId, ClientTx>>>` pattern.
    - Required Cargo feature: `["server", "macros", "schemars", "transport-streamable-http-server"]` (replaces the current `"transport-io"`, which can probably stay if stdio-mode is also retained).
    
    **The real unknown is the wire protocol shift, not the API surface.** rmcp 0.1's transport is legacy MCP SSE (`GET /sse` streaming + `POST /message?sessionId=` for client→server). rmcp 1.x's `StreamableHttpService` implements the new MCP 2025-06-18 streamable-HTTP spec (single endpoint, bidirectional streaming via either SSE-framed responses or plain JSON). This is client-visible — whether Claude Desktop and other configured MCP clients accept streamable-HTTP determines whether the bump is transparent or breaks the connection.
    
    **Gate before designing Slice E:** confirm Claude Desktop (or whichever MCP client is in use) supports the streamable-HTTP transport. If yes, the slice is straightforward (~150 lines deleted, ~20 lines of `StreamableHttpService` construction + axum mount). If not, the slice is blocked on client support — document and park.

- **`rusqlite 0.38 → 0.39`** — still blocked by `deadpool-sqlite 0.13` pinning `rusqlite = "0.38"`. Resolution paths: wait for deadpool update, replace deadpool-sqlite with another pool, or fork. Belongs in its own slice once one of those paths is viable.

---

## Verification checklist (end of slice)

- [ ] One commit lands on `main` with the documented message format.
- [ ] The commit individually passes `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp`.
- [ ] The new lib-test count is explicitly noted in the commit body (if it shifted from `105 passed; 0 failed` because of `schema_shape.rs` rewrites, document the new number).
- [ ] `Cargo.lock` is included in the commit, not deferred.
- [ ] If the slice escalates, this spec's "Deferred (rmcp+schemars joint bump)" section is amended with the specific blocker, and main branch returns cleanly to its pre-slice SHA.
- [ ] After the commit: `cargo install --path crates/zotero-mcp`, restart launchd service, confirm `ping` returns the new SHA.
- [ ] MSRV of `rmcp 1.7` and `schemars 1.x` is checked and accepted (project uses `channel = "stable"`, so any reasonably-recent MSRV is fine).

---

## Decisions deferred to implementation

- The exact `#[tool_router]` / `#[tool_handler]` shape (variant with `(server_handler)` argument vs. explicit pair) is discovered when the first conversion compiles. Both syntactically-valid forms exist per rmcp 1.5 docs.
- Whether the `impl ServerHandler` block keeps its manual `get_info()` or accepts the macro-generated one is decided when the first conversion runs. Either option meets the spec's intent.
- Whether `schema_for!`'s return type change requires only the 3 test bodies to update or also their assertion shapes (some assertions may need to be relaxed from strict equality to semantic checks). Decided per-test when the new schema output is observed.
- Whether the `Parameters<T>` destructuring needs `Parameters<T>` to also derive `Default` (the rmcp 1.5 docs example shows it does — `#[derive(Deserialize, schemars::JsonSchema, Default)]`). If so, the 27 `Args` structs may need a `Default` derive added. One-line per struct, mechanical, in-bounds.
