# Spec: Dependency upgrades — Slice E (replace hand-rolled SSE transport with rmcp's `transport-streamable-http-server`)

**Status:** Approved design, ready for plan-writing.
**Author:** Richard Lyon (with Claude Opus 4.7).
**Date:** 2026-05-13.
**Goal:** Replace `crates/zotero-mcp/src/http_transport.rs` (~150 lines of hand-rolled SSE session management) with rmcp 1.7's `StreamableHttpService` tower::Service. Migrate from the legacy MCP SSE wire protocol (`GET /sse` streaming + `POST /message?sessionId=` for client→server) to the MCP 2025-06-18 streamable-HTTP wire protocol at a single `/mcp` endpoint. Preserve all OAuth + bearer-auth behaviour, including the RFC 9728 §5.1 WWW-Authenticate challenge.

---

## Problem

The Slice C spec's deferred section (verified 2026-05-13 via direct source read of `rmcp-1.7.0/src/transport/streamable_http_server/tower.rs`) confirmed that rmcp 1.7's server-side streamable-HTTP transport is real and substantial:

- `rmcp::transport::streamable_http_server::tower::StreamableHttpService<S, M>` is a `tower_service::Service<Request<RequestBody>>` — drops into axum directly.
- `StreamableHttpServerConfig` covers everything `http_transport.rs` does by hand (SSE keep-alive, per-session service factory, cancellation token) plus features the hand-rolled version lacks (`allowed_hosts` for DNS-rebinding protection, `json_response` mode for skipping SSE framing on simple req-response tools per MCP 2025-06-18 spec).
- Pluggable `SessionManager` trait with reference implementations at `session/{local,never,store}.rs`.

The Anthropic MCP connector framework (used by Claude Cowork, the active consumer) supports both SSE and streamable-HTTP transports per [Anthropic's MCP connector docs](https://platform.claude.com/docs/en/agents-and-tools/mcp-connector). Anthropic has announced "support for SSE may be deprecated in the coming months." This slice future-proofs zotero-mcp against that deprecation and removes ~120 lines of hand-rolled transport plumbing.

The current hand-rolled transport (read 2026-05-13) lives at `crates/zotero-mcp/src/http_transport.rs` and contains:

- **Routes**: `GET /sse` (SSE stream with per-session endpoint event) + `POST /message?sessionId=` (client→server JSON-RPC relay).
- **Session storage**: `Arc<RwLock<HashMap<SessionId, Sender<ClientJsonRpcMessage>>>>`.
- **Per-session spawn pattern**: `tokio::spawn` builds a `(PollSender, ReceiverStream)` tuple transport and calls `ZoteroServer::new(state).serve(transport).await`. The spawn loop owns cleanup (removes session entry when `serve()` returns).
- **Cloudflare quick-tunnel workaround**: prepends a 2KB SSE comment (`:xxxxx...`) to the first SSE chunk so HTTP/2 edge buffering doesn't delay the `endpoint` event by up to the keep-alive interval. This hack is intrinsic to SSE framing — it does not carry over to streamable-HTTP responses written by rmcp's transport.
- **Bearer middleware** (`require_bearer_token`): `from_fn_with_state` axum middleware applied only to `/sse` + `/message`. Reads `Authorization: Bearer <token>`, calls `oauth_state.validate_token`, returns 401 with a specific RFC 9728 §5.1 challenge: `Bearer realm="zotero-mcp", resource_metadata="<url>", scope="mcp"`. Discovery routes (`/.well-known/...`) and `/oauth/token` stay unauthenticated.
- **Legacy static bearer**: a separate `ValidateRequestHeaderLayer::bearer(token)` layer applied globally when `ZOTERO_MCP_BEARER_TOKEN` is set. Independent of the OAuth bearer path.
- **4 tests** in the `tests` module: 3 exercise `require_bearer_token` directly via a stub router (`protected_router`), 1 (`tokens_survive_oauth_state_recreation`) exercises `OAuthState` token persistence and is not transport-specific.

---

## Decisions

1. **Single endpoint at `/mcp`.** Drop `/sse` and `/message` entirely. Cleanest server-side logic; user reconfigures the Cowork connector URL once after the slice ships. Mounting `StreamableHttpService` at `/sse` (URL-preserving) was considered and rejected because the client (Cowork's connector framework) likely makes transport-protocol decisions based on URL convention — a `/sse` URL signals SSE-protocol intent, ambiguous against a streamable-HTTP service. A dual-mount transition (`/sse` SSE + `/mcp` streamable-HTTP simultaneously) was rejected for adding code that needs to be removed in a follow-up regardless.

2. **All transport-config knobs exposed as env vars, with conservative defaults.** This lets the user empirically discover which combination Cowork accepts without recompiling:

   - `ZOTERO_MCP_TRANSPORT_STATEFUL` (parsed as bool, default `true`) → `StreamableHttpServerConfig.stateful_mode`. Default matches the current per-session `ZoteroServer` behaviour.
   - `ZOTERO_MCP_TRANSPORT_JSON` (parsed as bool, default `false`) → `StreamableHttpServerConfig.json_response`. Default uses SSE framing on the wire, which is the closest behavioural analogue to today.
   - `ZOTERO_MCP_ALLOWED_HOSTS` (comma-separated string, default unset) → `StreamableHttpServerConfig.allowed_hosts`. Unset = no Host validation (preserves current behaviour, keeps the Cloudflare quick-tunnel working). Set = explicit allow-list (security upgrade the user opts into).

   **Follow-up after validation.** Once the user confirms which `stateful_mode`/`json_response` combination works with Cowork, file a small cleanup commit that drops the loser env vars and hardcodes the winning defaults. This trims the eventual maintenance burden of carrying both modes.

3. **`sse_keep_alive` = 5s** to match the current setting (preserves Cloudflare-tunnel timing behaviour as closely as possible).

4. **SessionManager = rmcp's `LocalSessionStore`** reference implementation. The current `Arc<RwLock<HashMap<SessionId, ClientTx>>>` pattern is equivalent in semantics and there's no benefit to wrapping it in a custom `SessionManager` impl.

5. **Bearer middleware extracted to a new module.** `require_bearer_token` is currently embedded in `http_transport.rs`. It's reusable as-is — extract to `crates/zotero-mcp/src/bearer.rs` (or `bearer/mod.rs`) and apply as a `from_fn_with_state` tower::Layer on the new `/mcp` route only. OAuth discovery and `/oauth/token` continue to mount unauthenticated. The 3 existing tests (`missing_token_returns_401_with_www_authenticate`, `invalid_bearer_returns_401`, `minted_bearer_passes_through`) move with the middleware. The 4th test (`tokens_survive_oauth_state_recreation`) moves to `oauth/token_store.rs`'s tests — it's an OAuth concern, not a transport concern.

6. **OAuth router unchanged.** `oauth::router(oauth_state)` still mounts independently. Discovery + `/oauth/token` stay where they are.

7. **Legacy static-bearer support preserved.** `ZOTERO_MCP_BEARER_TOKEN` env var still works via `ValidateRequestHeaderLayer::bearer` applied globally. Slice E doesn't break the static-bearer code path; the user can continue to use either OAuth or static-bearer as before.

8. **Cloudflare padding hack removed.** It's incompatible with rmcp's response-writing API (StreamableHttpService writes its own body) and only mattered under SSE framing. With `json_response: true` (plain JSON responses), Cloudflare's SSE buffering is not in play. With `json_response: false` (SSE framing under streamable-HTTP), buffering *may* re-emerge — flagged as a Risk; verify against the actual Cloudflare quick-tunnel after the slice ships.

9. **Single atomic commit** (matches Slice C's pattern for transport-shape changes). The migration cannot be partially landed without leaving the server in a broken state. Commit message format:

   ```
   chore(transport): replace hand-rolled SSE with rmcp StreamableHttpService
   ```

   Followed (in a separate later commit) by the env-var trim follow-up described in Decision 2.

10. **Test gate.** `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp` both pass. Lib-test count may shift down by 2 (the SSE-routing tests removed) and up by 1-2 (a smoke test for `/mcp` accepting a POST). Net delta: -1 to ±0. Note the new number in the commit body.

11. **Reinstall + launchd restart gated on user validation.** The slice mandates an explicit checkpoint between code-commit and `cargo install`: the user opens a new launchd plist with `ZOTERO_MCP_TRANSPORT_STATEFUL`/`ZOTERO_MCP_TRANSPORT_JSON`/`ZOTERO_MCP_ALLOWED_HOSTS` configured, manually validates that Cowork can still call `ping`, and only then commits to the launchd flip. If Cowork fails to connect, the slice still landed (commit is in `main`), but the running binary can stay on the pre-slice SHA until the issue is resolved (env-var adjustment or, worst case, revert).

---

## Migration plan

The implementation discovers exact API shapes by compile-error iteration. This plan documents the structure of the work.

### 1. `Cargo.toml` (workspace, line 21)

Add `"transport-streamable-http-server"` to the rmcp features list. Current line:

```toml
rmcp = { version = "1", features = ["server", "macros", "schemars", "transport-io"] }
```

After:

```toml
rmcp = { version = "1", features = ["server", "macros", "schemars", "transport-io", "transport-streamable-http-server"] }
```

`transport-io` is preserved because `main.rs` still uses `ServiceExt::serve` over stdio for the non-HTTP code path.

### 2. Create `crates/zotero-mcp/src/bearer.rs`

Lift `require_bearer_token` from `http_transport.rs` verbatim (it's already self-contained — takes `OAuthState` via `from_fn_with_state`, returns an axum `Response`, no other transport coupling). Also move the 3 bearer-related tests (`missing_token_returns_401_with_www_authenticate`, `invalid_bearer_returns_401`, `minted_bearer_passes_through`) and the helper functions they use (`test_oauth_state`, `protected_router`) to this file.

Re-export the middleware fn from `lib.rs` so `http_transport::run` can apply it as a layer.

### 3. Rewrite `crates/zotero-mcp/src/http_transport.rs`

Target shape (~30 lines after the file-level doc comment + the `run` function):

```rust
//! HTTP/streamable-HTTP transport for zotero-mcp.
//!
//! Mounts rmcp 1.7's StreamableHttpService at /mcp. The service handles
//! session management, request/response framing, and per-session
//! ZoteroServer spawning. We supply bearer auth as a tower layer on the
//! /mcp route only — OAuth discovery and /oauth/token stay
//! unauthenticated so clients can complete the handshake.

use std::net::SocketAddr;
// ... imports for axum, rmcp::transport::streamable_http_server::tower::*,
//     crate::bearer, crate::oauth, crate::server::ZoteroServer ...

pub async fn run(
    state: AppState,
    addr: SocketAddr,
    bearer: Option<String>,
    oauth_state: Option<OAuthState>,
) -> anyhow::Result<()> {
    let stateful_mode = std::env::var("ZOTERO_MCP_TRANSPORT_STATEFUL")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(true);
    let json_response = std::env::var("ZOTERO_MCP_TRANSPORT_JSON")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(false);
    let allowed_hosts: Vec<String> = std::env::var("ZOTERO_MCP_ALLOWED_HOSTS")
        .ok().map(|s| s.split(',').map(str::trim).map(String::from).collect())
        .unwrap_or_default();

    let config = StreamableHttpServerConfig {
        sse_keep_alive: Some(Duration::from_secs(5)),
        stateful_mode,
        json_response,
        // allowed_hosts: if Vec empty, set to bypass-equivalent
        ..StreamableHttpServerConfig::default()
    };

    let state_clone = state.clone();
    let service = StreamableHttpService::new(
        move || Ok(ZoteroServer::new(state_clone.clone())),
        LocalSessionStore::default(),
        config,
    );

    let mut mcp_route = Router::new()
        .route_service("/mcp", service);
    if let Some(oauth_state) = oauth_state.clone() {
        mcp_route = mcp_route.layer(axum::middleware::from_fn_with_state(
            oauth_state, crate::bearer::require_bearer_token,
        ));
    }

    let mut app = Router::new().merge(mcp_route);
    if let Some(oauth_state) = oauth_state {
        app = app.merge(oauth::router(oauth_state));
    }
    let mut app = app.layer(tower_http::trace::TraceLayer::new_for_http());
    if let Some(token) = bearer {
        #[allow(deprecated)]
        let auth = ValidateRequestHeaderLayer::bearer(&token);
        app = app.layer(auth);
    }

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

The exact API surface of `StreamableHttpService::new` / `StreamableHttpServerConfig` field names is discovered at compile time — read the rmcp source at `~/.cargo/registry/src/.../rmcp-1.7.0/src/transport/streamable_http_server/tower.rs` if doc lookups are slow.

Add 1 smoke test: `/mcp` returns 401 without a valid bearer, 200/202 with one (the existing bearer-test helpers in `bearer.rs` can be reused; the new test exercises the full stack).

### 4. `crates/zotero-mcp/src/main.rs`

No expected changes. The `http_transport::run` signature stays the same. Verify-only.

### 5. `crates/zotero-mcp/src/lib.rs`

Add `pub mod bearer;` next to the existing `pub mod http_transport;`.

### 6. `Cargo.lock`

Joint update via `cargo build` — adding the new rmcp feature pulls in `tokio-stream`, `http`, `http-body`, `http-body-util`, `bytes`, `sse-stream`, `uuid`, `rand`, `tower` (already direct dep). Most of these are already in the tree at this point thanks to Slice C; expect minimal churn.

---

## Risks

1. **Cowork's Cloudflare-tunnel buffering on SSE framing.** The current `:` comment padding hack disappears with this slice. If `json_response: false` is selected and Cloudflare still buffers small SSE chunks until ~2KB accumulates, the first message may be delayed by the 5s keep-alive interval. Mitigation: the env var lets the user flip to `json_response: true` (plain JSON, no SSE framing) without recompiling — verify against the actual tunnel during the validation gate.

2. **Cowork connector URL update.** After this slice ships, the Cowork connector pointing at `https://<tunnel-host>/sse` will return 404. The user must update the URL to `https://<tunnel-host>/mcp` for any client to connect. This is a one-time manual step — document in the commit body and surface to the user during the validation gate.

3. **`#[non_exhaustive]` struct on `StreamableHttpServerConfig`.** Per Slice C's experience, rmcp 1.7's config structs use `#[non_exhaustive]`, so struct-literal initialization may not compile. The plan uses `..StreamableHttpServerConfig::default()` to side-step. If the default values for unspecified fields drift across rmcp versions, this is a silent change — verify the defaults make sense at implementation time.

4. **`route_service` vs `Router::route` for tower services.** axum's exact mounting API for a non-handler `tower::Service` is `Router::route_service("/path", service)`. If the type bounds don't match (e.g. `StreamableHttpService<S, M>` requires generics that conflict with axum's expected `Service<Request<axum::body::Body>>`), expect a compile error about `Service<Request<X>>` bounds. Fix: thread the right body type or wrap with a small adapter. Spec-level escalation: revert if the adapter is more than 20 lines.

5. **Bearer middleware extraction breakage.** Moving `require_bearer_token` to its own module changes the import path. Any internal callers (probably just `http_transport.rs`) need updating. Tests that import `protected_router` or `test_oauth_state` from `http_transport::tests` move with them.

6. **Test count drift.** Lib-test count may shift by -1 to ±0 (lose 2 SSE-routing tests, gain 1-2 smoke tests). Note the new number explicitly in the commit body, same as Slice C.

---

## Validation gate (between code-commit and launchd flip)

After Task 1 lands and the build/tests are clean, **before reinstalling and restarting the launchd service**:

1. Update the launchd plist to include the new env vars under `<key>EnvironmentVariables</key>`:

   ```xml
   <key>ZOTERO_MCP_TRANSPORT_STATEFUL</key> <string>true</string>
   <key>ZOTERO_MCP_TRANSPORT_JSON</key> <string>false</string>
   <!-- Cloudflare tunnel hostname goes here; comma-separated. Or omit for no Host validation. -->
   <key>ZOTERO_MCP_ALLOWED_HOSTS</key> <string></string>
   ```

2. Reload the plist (`launchctl unload && launchctl load`) — but **do not** `cargo install --path` yet. The running binary is still the pre-slice one.

3. Verify the old binary still works (`ping` from Cowork).

4. `cargo install --path crates/zotero-mcp` + `launchctl kickstart -k gui/$(id -u)/com.zotero-mcp.http`. **Now the new binary is live with the new endpoint.**

5. **Manual user step:** update the Cowork connector URL from `…/sse` to `…/mcp`.

6. `ping` from Cowork. If it returns the new SHA, the slice is good. If not:
   - Toggle `ZOTERO_MCP_TRANSPORT_JSON` to `true` (reload plist + kickstart), retry.
   - If still failing, toggle `ZOTERO_MCP_TRANSPORT_STATEFUL` to `false`, retry.
   - If all permutations fail, the issue is upstream (Cowork doesn't accept streamable-HTTP from this tunnel, or rmcp's transport produces output Cowork can't parse). Revert path: roll the launchd plist back to the pre-slice ExecutableArguments (don't reinstall), and report findings here so Slice E can be re-scoped.

---

## Out of scope (deferred)

- **Env-var trim follow-up.** Once Decision 11's validation confirms which `stateful_mode`/`json_response` combination works with Cowork, file a small cleanup commit that drops the unused env-var paths and hardcodes the winner. This is intentionally deferred so the validation gate happens with both options available.

- **`allowed_hosts` security upgrade.** The default behaviour of this slice preserves "no Host validation" via the empty-string sentinel. Tightening to an explicit allow-list (e.g., `127.0.0.1,localhost,<tunnel>.trycloudflare.com`) is a security improvement the user can opt into after validating the basic migration works. Not blocking on this slice.

- **`rusqlite 0.38 → 0.39`** — still blocked by `deadpool-sqlite 0.13`. Belongs in its own slice once one of the upstream paths is viable.

- **OAuth replacement (formerly Slice D)** — abandoned in the Slice C spec amendment (`6efa2f5`). rmcp's `auth` feature is client-only; oauth.rs stays as-is.

---

## Verification checklist (end of slice)

- [ ] One commit lands on `main` with message format `chore(transport): replace hand-rolled SSE with rmcp StreamableHttpService`.
- [ ] The commit passes `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp`.
- [ ] The new lib-test count is explicitly noted in the commit body (delta from Slice C's 105 documented).
- [ ] `Cargo.lock` is included in the commit.
- [ ] `bearer.rs` exists as a new module with its 3 tests passing.
- [ ] `http_transport.rs` has shrunk from ~150 lines to ~30 + smoke test.
- [ ] `oauth.rs` and `oauth/token_store.rs` are unchanged.
- [ ] `main.rs` is unchanged.
- [ ] Validation gate completed: user confirms Cowork's `ping` works against the new `/mcp` endpoint with at least one (stateful_mode, json_response) combination.
- [ ] After validation: launchd service is running with the new binary; user reports back which env-var combination worked so the follow-up trim commit can proceed.

---

## Decisions deferred to implementation

- The exact rmcp 1.7 API names for `StreamableHttpService::new` / `StreamableHttpServerConfig` field accessors. Read `~/.cargo/registry/src/.../rmcp-1.7.0/src/transport/streamable_http_server/tower.rs` (lines 39 and 496 specifically) at implementation time.
- The exact bound shape required to mount `StreamableHttpService<S, M>` on axum's `Router::route_service`. If it fails, the in-bounds fix is a one-file adapter (≤ 20 lines). Beyond that triggers escalation.
- The exact format of `allowed_hosts` — comma-separated string vs. `Vec<String>` vs. some other shape. Look up `StreamableHttpServerConfig.allowed_hosts` field type and parse the env var accordingly.
- Whether `LocalSessionStore` is constructed via `::default()` or `::new()` or via a builder. Compile error names the path.
