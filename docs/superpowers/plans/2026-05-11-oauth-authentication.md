# Plan: OAuth 2.1 authentication for zotero-mcp HTTP transport

**Status:** Ready to execute in a fresh session
**Branch:** `feature/zotero-mcp-implementation` (continue here)
**Goal:** Authenticate requests to the HTTP/SSE transport so the public Tailscale Funnel URL is no longer effectively unrestricted.

---

## Why this plan exists

The HTTP/SSE transport (`crates/zotero-mcp/src/http_transport.rs`) currently runs without authentication. Tailscale Funnel publishes the server at `https://laptop.stoat-minnow.ts.net` — a public URL — so anyone who learns it has full read/write access to the user's Zotero library.

A previous attempt added a `tower-http` bearer-token layer over every route. That broke Claude.ai's "Add custom connector" flow because Claude.ai probes `/.well-known/oauth-authorization-server` and similar paths before sending any auth, and a blanket 401 made the server look unreachable. So we ripped the bearer check out and shipped without auth as a stopgap.

Claude.ai's connector dialog has fields for **OAuth Client ID** and **OAuth Client Secret** under "Advanced." This is the supported way to authenticate a remote MCP server. We need to expose the OAuth 2.1 surface Claude.ai expects.

---

## Current state at session start

- Repo root: `/Users/rjl/Code/mcp-zotero`
- Server installed at: `/Users/rjl/.cargo/bin/zotero-mcp`
- Running under launchd: `~/Library/LaunchAgents/com.zotero-mcp.http.plist`
  - Env vars: `ZOTERO_MCP_HTTP=127.0.0.1:8765`, `RUST_LOG=info,tower_http=debug`
  - **No `ZOTERO_MCP_BEARER_TOKEN`** — server runs without auth.
- Public URL: `https://laptop.stoat-minnow.ts.net/sse` (Tailscale Funnel)
- Logs: `~/Library/Logs/zotero-mcp/http.{out,err}.log`
- Latest commit on branch: `e1f4d72` ("fix(mcp): make bearer auth optional, add request tracing")

The Claude.ai connector is already registered and Cowork uses it daily. We must not break that during the migration — Cowork will need to pick up the new auth shape, but the URL should stay the same.

---

## Architecture target

Implement the minimum OAuth 2.1 surface that Claude.ai's MCP connector accepts. Almost certainly the **Client Credentials grant** (RFC 6749 §4.4), because:

- The "Advanced" form exposes `client_id` + `client_secret` only. There's no UI for redirect URIs, scopes, or interactive consent.
- MCP servers run headless; there's no browser to redirect.
- Client Credentials is the canonical "machine to machine" flow.

The flow we'll support:

1. **Discovery (public, unauthenticated):**
   `GET /.well-known/oauth-authorization-server` → JSON document advertising the token endpoint and supported grant types.
2. **Token endpoint (public, unauthenticated):**
   `POST /oauth/token` with `grant_type=client_credentials`, `client_id=...`, `client_secret=...`.
   On success: `{"access_token":"<opaque>", "token_type":"Bearer", "expires_in":<seconds>}`.
3. **Resource endpoints (authenticated):**
   `GET /sse`, `POST /message?sessionId=...` — both gated by `Authorization: Bearer <access_token>`. The access token is validated against the in-memory token store.

We will NOT implement dynamic client registration (RFC 7591) in the first pass. One pre-registered `client_id` + `client_secret` pair, stored in a config file. If Claude.ai's connector turns out to require DCR, add `POST /oauth/register` (issuing the same pre-shared pair) as a small addendum.

---

## Pre-implementation research (15 minutes, before any code)

The MCP spec and Claude.ai's connector behaviour aren't fully documented. Confirm assumptions before coding:

1. **Read the MCP authorization spec.** The current authoritative version is at
   https://modelcontextprotocol.io/specification (search for "Authorization"). Key questions:
   - Required discovery document fields?
   - Required token endpoint behaviour?
   - Required token format (opaque vs JWT)?
   - Required `WWW-Authenticate` header on 401?
   - Spec version — match what `rmcp 0.1.5` declares as `ProtocolVersion::default()` (2024-11-05) if there's a version-specific auth section.

2. **Empirically: what does Claude.ai actually request?**
   The simplest experiment: add a temporary handler that logs (method, path, headers, body) for **any path that isn't `/sse` or `/message`**, returns 404. Then re-add the connector at Claude.ai with non-empty placeholder OAuth fields and watch the log. Each probe Claude.ai sends becomes a guide to the endpoint we must implement.

   To temporarily add: in `http_transport.rs`, wrap the router in a fallback handler:
   ```rust
   let app = Router::new()
       .route("/sse", get(sse_handler))
       .route("/message", post(post_handler))
       .fallback(|req: axum::extract::Request| async move {
           tracing::info!(
               "unknown route probed: {} {} (headers: {:?})",
               req.method(), req.uri(), req.headers()
           );
           axum::http::StatusCode::NOT_FOUND
       })
       .with_state(shared)
       ...
   ```

3. **Decide token format**:
   - **Opaque** (a 32-byte random hex string stored in an in-memory `DashMap<String, TokenInfo>`): simpler, easy to revoke.
   - **JWT** (HMAC-signed with a server secret): stateless but more code.

   Recommend opaque for v1. Switch to JWT later if multi-instance concerns emerge.

---

## Implementation plan

Done in roughly this order. Each task is a small atomic commit.

### Task A. Add discovery + probe-logging fallback

Goal: see exactly what Claude.ai probes when the OAuth fields are populated.

- **Files:**
  - `crates/zotero-mcp/src/http_transport.rs` — add `fallback` handler that logs unknown routes
  - `crates/zotero-mcp/src/oauth.rs` (new) — module skeleton with one handler:
    `GET /.well-known/oauth-authorization-server` returning a hand-coded JSON document
- **Discovery JSON** (initial guess, refine per spec):
  ```json
  {
    "issuer": "https://laptop.stoat-minnow.ts.net",
    "authorization_endpoint": "https://laptop.stoat-minnow.ts.net/oauth/authorize",
    "token_endpoint": "https://laptop.stoat-minnow.ts.net/oauth/token",
    "registration_endpoint": "https://laptop.stoat-minnow.ts.net/oauth/register",
    "grant_types_supported": ["client_credentials"],
    "token_endpoint_auth_methods_supported": ["client_secret_post", "client_secret_basic"],
    "response_types_supported": ["token"],
    "scopes_supported": ["mcp"]
  }
  ```
  The issuer URL must match what Claude.ai probes — we may need to read it from config.
- **Re-add the connector at Claude.ai** with placeholder OAuth values; watch the log to learn the exact request sequence.

**Commit:** `feat(mcp): add OAuth discovery endpoint + fallback probe logging`

### Task B. Implement the token endpoint

- **Files:** `crates/zotero-mcp/src/oauth.rs`
- **Endpoint:** `POST /oauth/token`
  - Accept form-urlencoded body with `grant_type`, `client_id`, `client_secret`.
  - Also accept HTTP Basic auth (`Authorization: Basic <base64(client_id:client_secret)>`) per spec — Claude.ai may use either.
  - Validate `client_id`/`client_secret` against a config file (see Task D for storage).
  - On success: generate a 32-byte hex token, store in `DashMap<String, ExpiresAt>`, return JSON.
  - On failure: 401 with `{"error":"invalid_client"}`.

- **Token TTL:** 1 hour. Claude.ai will refresh as needed.

**Commit:** `feat(mcp): implement OAuth /token endpoint (client_credentials grant)`

### Task C. Gate /sse and /message on the bearer token

- **Files:** `crates/zotero-mcp/src/http_transport.rs`
- Replace the no-op auth path with a custom middleware that:
  1. Reads `Authorization: Bearer <token>` header.
  2. Looks up `<token>` in the in-memory store.
  3. Checks expiry.
  4. Passes through on hit, returns 401 + `WWW-Authenticate: Bearer realm="zotero-mcp"` on miss.
- The middleware MUST only apply to `/sse` and `/message`, not to `/.well-known/...` or `/oauth/...`.

Use `axum::middleware::from_fn_with_state` or apply `tower_http::auth::AsyncRequireAuthorizationLayer::new(...)` only on the protected sub-router (i.e., build a nested `Router` for the protected routes and `.merge()` with the public router).

**Commit:** `feat(mcp): bearer-token middleware on resource routes only`

### Task D. Credential storage + config

- **Files:**
  - `crates/zotero-mcp/src/oauth.rs`
  - `crates/zotero-mcp/src/main.rs` (load config at startup)
- Config schema (new TOML in `~/.config/zotero-mcp/oauth.toml`, or an env var):
  ```toml
  client_id = "zotero-mcp-cowork"
  client_secret = "<32-byte hex>"
  issuer = "https://laptop.stoat-minnow.ts.net"
  ```
- Generate on first run if missing. Permissions `0600`.
- Hand the values to `oauth::Router::build(config)` so the auth functions can verify against them.

**Commit:** `feat(mcp): persist OAuth client credentials in ~/.config/zotero-mcp/oauth.toml`

### Task E. Test end-to-end

1. Restart the launchd service.
2. Manual: `curl` the discovery endpoint, get the JSON. Test that it parses.
3. Manual: `curl -X POST` the token endpoint with the credentials, get an access token back.
4. Manual: use that token to `GET /sse` — should succeed; without the token should be 401.
5. Wire it into Claude.ai: re-edit the connector, paste the `client_id`/`client_secret` into the dialog. Restart Cowork. Confirm Zotero tools still work.
6. Probe negative case: bad credentials → 401 with `WWW-Authenticate` header.

**Commit:** `test(mcp): end-to-end OAuth flow verified (no code change)` — empty commit purely for the changelog.

### Task F. Cleanup + docs

- Remove the fallback probe-logger from Task A.
- Update `docs/CLAUDE_COWORK_SETUP.md` to document the OAuth setup (where the credentials live, how to rotate, how to revoke all tokens).
- Update the launchd plist if needed.

**Commit:** `docs(mcp): document OAuth configuration for Cowork setup`

---

## Pitfalls to watch

1. **CORS:** Claude.ai's web app may make cross-origin requests to the token endpoint. If so, `tower-http::cors::CorsLayer::permissive()` on the OAuth router will fix it. Watch for CORS errors in the user's browser console (devtools → network tab on the connector dialog).

2. **Issuer URL mismatch:** OAuth spec requires the `iss` in the token response to match the `issuer` in the discovery doc. Both must match what the client believes the server's base URL is — i.e., the Tailscale Funnel URL, not `http://127.0.0.1:8765`. If Claude.ai validates strictly, an `iss` mismatch will fail the handshake.

3. **HTTPS in URLs:** All URLs in the discovery doc must be `https://...` (the public Tailscale URL). The server itself binds to `http://127.0.0.1:8765`; that's fine — Funnel terminates TLS in front.

4. **Auth applies only to protected routes:** Easy to accidentally re-gate the discovery and token endpoints. The middleware must be scoped to a sub-router that excludes `/.well-known/*` and `/oauth/*`.

5. **Token rotation while Cowork is mid-conversation:** When an access token expires (default 1 hr), Claude.ai should silently refresh via the token endpoint. Make sure the refresh path returns a new token without invalidating the old one until expiry — otherwise long-running tool calls will fail.

6. **Don't break Claude Desktop:** Claude Desktop launches the same binary in stdio mode. The OAuth code only matters when `ZOTERO_MCP_HTTP` is set; verify that the stdio path is unchanged. The `cargo test` workspace should still pass.

7. **Pre-shared secret discovery:** The user needs to know `client_id` and `client_secret` to paste into Claude.ai. After generating them, print to stderr at startup with a clear message (no token leak in logs — print only to the user-visible install/setup step). Or document `cat ~/.config/zotero-mcp/oauth.toml`.

---

## Out of scope (defer to a later plan)

- Dynamic client registration (`POST /oauth/register`)
- Authorization Code flow with PKCE (needs a user-facing consent screen — not relevant for a single-user backend)
- Multi-user tenancy
- Refresh tokens (Claude.ai re-runs client_credentials whenever the access token nears expiry)
- Audit logging beyond what `tower-http::trace` already provides
- JWT-formatted tokens (opaque is fine)
- Persistent token store across restarts (in-memory is fine; clients will re-acquire after a server restart)

---

## Quick start for the fresh session

1. Read this file.
2. Read `crates/zotero-mcp/src/http_transport.rs` to understand the current router shape.
3. Read the MCP authorization spec at modelcontextprotocol.io.
4. Start with Task A — add probe logging, ask the user to re-trigger the Claude.ai connector dialog, capture the actual probe sequence.
5. Iterate from there.

Branch already checked out: `feature/zotero-mcp-implementation`. Tip the user at the end to merge & rotate the Funnel if they want a fresh URL post-secret-leak (`tailscale funnel off && tailscale funnel --bg 8765` won't actually change the hostname — they'd need to rename the machine in Tailscale, but that's only worth it if they suspect the URL is already compromised).
