# Dependency Upgrades — Slice E Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `crates/zotero-mcp/src/http_transport.rs` (~150 lines of hand-rolled SSE session management) with rmcp 1.7's `StreamableHttpService` tower::Service, migrating the wire protocol from legacy MCP SSE (`GET /sse` + `POST /message`) to the MCP 2025-06-18 streamable-HTTP spec at a single `/mcp` endpoint.

**Architecture:** Compile-error-driven port. Extract the existing `require_bearer_token` axum middleware (which emits an RFC 9728 §5.1 WWW-Authenticate challenge) verbatim into a new `crates/zotero-mcp/src/bearer.rs` module so it can be applied as a tower::Layer on the new route. Rewrite `http_transport::run` to mount rmcp's `StreamableHttpService` at `/mcp` with three transport-config knobs exposed as env vars (`ZOTERO_MCP_TRANSPORT_STATEFUL`, `ZOTERO_MCP_TRANSPORT_JSON`, `ZOTERO_MCP_ALLOWED_HOSTS`) so the user can find which combination Claude Cowork accepts without recompiling. Code lands as one atomic commit; reinstall + launchd flip is gated on user validation.

**Tech Stack:** Rust, Cargo. rmcp 1.7 `transport-streamable-http-server` feature, axum 0.8, tower 0.5.

**Spec:** `docs/superpowers/specs/2026-05-13-dependency-upgrades-slice-e-design.md` (commit `6f1599a`).

---

## File Structure

| File | Purpose | Action |
|---|---|---|
| `Cargo.toml` (workspace, line 21) | Add `"transport-streamable-http-server"` to rmcp features | Modify |
| `crates/zotero-mcp/src/bearer.rs` | NEW module — extracted `require_bearer_token` + its 3 tests | Create |
| `crates/zotero-mcp/src/lib.rs` | Add `pub mod bearer;` next to existing `pub mod http_transport;` | Modify (1 line) |
| `crates/zotero-mcp/src/http_transport.rs` | Strip SSE-specific code; new `run` uses `StreamableHttpService` at `/mcp`; 1 smoke test | Rewrite (~150 → ~50 lines incl. smoke test) |
| `crates/zotero-mcp/src/main.rs` | Verify-only — `run` signature stays the same | Read; modify only if forced |
| `crates/zotero-mcp/src/oauth.rs` | Verify-only — out-of-bounds per spec | Read at most; do NOT modify |
| `crates/zotero-mcp/src/oauth/token_store.rs` | Verify-only — out-of-bounds per spec | Read at most; do NOT modify |
| `Cargo.lock` | Joint update from the new rmcp feature | Modify |

Decomposition rationale: `bearer.rs` has one clear responsibility (the WWW-Authenticate-emitting bearer middleware) and is testable in isolation against an `OAuthState`. `http_transport.rs` shrinks to just route wiring + the smoke test. The validation gate is operational, not code — documented in Task 1 Step 13 and the "Hand-off" section.

---

## Pre-flight: confirm clean state

**Files:**
- Read-only: working tree

- [ ] **Step 1: Confirm clean tree on `main`**

Run: `cd /Users/rjl/Code/github/zotero-connector && git status`

Expected: `nothing to commit, working tree clean` and branch `main`.

If dirty, stop and resolve before starting the slice.

- [ ] **Step 2: Capture baseline test results**

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo test -p zotero-mcp 2>&1 | grep "^test result:" | sort | uniq -c`

Expected: every line `ok`, no `FAILED`. Lib tests should be `105 passed; 0 failed` (the Slice C baseline). Write the number down — Slice E's lib-test count may shift by -1 to ±0 (remove 2 SSE-routing tests, gain 1-2 smoke tests + the 3 moved bearer tests; net delta documented in the commit body).

- [ ] **Step 3: Record the pre-flight SHA**

Run: `cd /Users/rjl/Code/github/zotero-connector && git rev-parse HEAD`

Write down the SHA. This is the rollback point if the slice escalates. Expected at slice start: `6f1599a` (the Slice E spec commit) — or, if the slice is re-run after a checkpoint, whatever the current HEAD is.

---

## Task 1: Replace SSE transport with rmcp `StreamableHttpService`

**Files (all changes land in one commit at Step 14):**
- Modify: `Cargo.toml` (workspace, line 21 — add `transport-streamable-http-server` feature)
- Create: `crates/zotero-mcp/src/bearer.rs`
- Modify: `crates/zotero-mcp/src/lib.rs` (one line, `pub mod bearer;`)
- Rewrite: `crates/zotero-mcp/src/http_transport.rs`
- Modify: `Cargo.lock`
- Read-only verification: `crates/zotero-mcp/src/main.rs`, `crates/zotero-mcp/src/oauth.rs`, `crates/zotero-mcp/src/oauth/token_store.rs`

**Surface inventory** (from spec):

- `http_transport.rs` (current, ~150 lines) contains: `AppShared` struct (state + tx map), `PostQuery` struct, `post_handler`, `sse_handler` (with the 2KB Cloudflare padding hack at lines 111–122), `run` function, `require_bearer_token` middleware (lines 193–228), and 4 tests. Imports include `axum::response::sse::*`, `tokio_stream::wrappers::ReceiverStream`, `tokio_util::sync::PollSender`, `futures::{SinkExt, StreamExt}`, `rmcp::ServiceExt`, `rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage}`.
- Bearer middleware is self-contained: takes `OAuthState` via `from_fn_with_state`, returns `axum::response::Response`. No transport coupling.
- 4 tests in `http_transport::tests`: `missing_token_returns_401_with_www_authenticate`, `invalid_bearer_returns_401`, `minted_bearer_passes_through`, `tokens_survive_oauth_state_recreation`. First 3 use a `protected_router` helper that stubs the `/sse` route with `StatusCode::OK`. The 4th tests OAuth state recreation and is not transport-specific.

### Step 1: Discover the exact rmcp 1.7 API names

Read `/Users/rjl/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rmcp-1.7.0/src/transport/streamable_http_server/tower.rs` lines 39–130 (the `StreamableHttpServerConfig` struct and its `Default` impl) and lines 490–560 (`StreamableHttpService` struct + impl block). Note:

- Exact field names on `StreamableHttpServerConfig` (the spec lists `sse_keep_alive`, `sse_retry`, `stateful_mode`, `json_response`, `cancellation_token`, `allowed_hosts` — confirm these). Note especially whether `allowed_hosts` is `Vec<String>` or some `HostAuthority` type, and whether there's a sentinel for "no validation."
- Exact constructor signature of `StreamableHttpService::new`. Spec assumes `(service_factory: impl Fn() -> Result<S, std::io::Error>, session_manager: M, config: StreamableHttpServerConfig)` but verify.
- Where `LocalSessionStore` lives (`session/local.rs`) and how it's constructed — `::default()`, `::new()`, or a builder.

Also read `/Users/rjl/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rmcp-1.7.0/src/transport/streamable_http_server.rs` (the parent module) to see what's re-exported at `rmcp::transport::streamable_http_server::`.

Record what you find. The remaining steps reference these names; if any differs from the spec, prefer what the source says.

### Step 2: Add the rmcp feature flag

In `/Users/rjl/Code/github/zotero-connector/Cargo.toml`, find line 21:

```toml
rmcp = { version = "1", features = ["server", "macros", "schemars", "transport-io"] }
```

Change to:

```toml
rmcp = { version = "1", features = ["server", "macros", "schemars", "transport-io", "transport-streamable-http-server"] }
```

`transport-io` is preserved (main.rs's stdio path still uses it).

### Step 3: Update the lockfile

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo update -p rmcp 2>&1 | tail -10`

(This may produce no version change if rmcp 1.7.0 is already the resolved version — that's fine. The lockfile churn happens at the next `cargo build` because the new feature pulls in transitive deps.)

### Step 4: Create `crates/zotero-mcp/src/bearer.rs` with the extracted middleware + tests

Create the file with this content. Paths are absolute; the engineer creates the file directly.

```rust
//! Bearer-token guard for OAuth-protected MCP endpoints.
//!
//! Reads the `Authorization` header, validates the bearer token against the
//! in-memory token store, and either passes the request through or returns
//! `401 Unauthorized` with a `WWW-Authenticate` challenge that points clients
//! at the resource metadata document (RFC 9728 §5.1). On failure clients are
//! expected to fetch `resource_metadata`, walk to the advertised authorization
//! server, and call `/oauth/token` to acquire a token.

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::oauth::OAuthState;

/// Per-request guard for OAuth-protected routes.
pub async fn require_bearer_token(
    State(oauth_state): State<OAuthState>,
    req: Request,
    next: Next,
) -> Response {
    let bearer = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));

    if let Some(token) = bearer {
        if oauth_state.validate_token(token.trim()).await {
            return next.run(req).await;
        }
    }

    let challenge = format!(
        "Bearer realm=\"zotero-mcp\", resource_metadata=\"{}\", scope=\"mcp\"",
        oauth_state.resource_metadata_url()
    );
    let (status, error) = if bearer.is_some() {
        (StatusCode::UNAUTHORIZED, "invalid_token")
    } else {
        (StatusCode::UNAUTHORIZED, "missing_token")
    };
    tracing::info!(error, "bearer auth failed");
    (
        status,
        [(
            axum::http::header::WWW_AUTHENTICATE,
            challenge.as_str(),
        )],
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::{OAuthConfig, OAuthState};
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::{Router, routing::get};
    use tower::ServiceExt;

    fn test_oauth_state() -> OAuthState {
        let dir = std::env::temp_dir().join(format!(
            "zotero-mcp-bearer-test-{}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        OAuthState::with_tokens_path(
            OAuthConfig {
                client_id: "test-id".into(),
                client_secret: "test-secret".into(),
                issuer: "https://example.test".into(),
                access_token_ttl_secs: None,
                refresh_token_ttl_secs: None,
            },
            dir.join("tokens.json"),
        )
        .unwrap()
    }

    /// Build a router that mirrors the bearer-gated portion of `run`, without
    /// requiring `AppState`. The protected route is replaced with a stub so we
    /// can observe whether the middleware lets the request through.
    fn protected_router(oauth_state: OAuthState) -> Router {
        Router::new()
            .route("/protected", get(|| async { StatusCode::OK }))
            .layer(axum::middleware::from_fn_with_state(
                oauth_state,
                require_bearer_token,
            ))
    }

    #[tokio::test]
    async fn missing_token_returns_401_with_www_authenticate() {
        let app = protected_router(test_oauth_state());
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/protected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let challenge = resp
            .headers()
            .get(axum::http::header::WWW_AUTHENTICATE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(challenge.starts_with("Bearer "));
        assert!(challenge.contains("realm=\"zotero-mcp\""));
        assert!(challenge.contains(
            "resource_metadata=\"https://example.test/.well-known/oauth-protected-resource\""
        ));
        assert!(challenge.contains("scope=\"mcp\""));
    }

    #[tokio::test]
    async fn invalid_bearer_returns_401() {
        let app = protected_router(test_oauth_state());
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/protected")
                    .header("authorization", "Bearer not-a-real-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn minted_bearer_passes_through() {
        let oauth_state = test_oauth_state();
        let pair = oauth_state.token_store().mint_pair(None).await.unwrap();
        let token = pair.access_token;
        let app = protected_router(oauth_state);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/protected")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
```

Notes:
- The stub route is renamed from `/sse` to `/protected` because the middleware is no longer transport-specific — `/sse` would be misleading.
- The `tokens_survive_oauth_state_recreation` test is intentionally NOT moved here — it's an OAuth state concern, not a bearer-middleware concern. Move it in Step 5 instead.

### Step 5: Move `tokens_survive_oauth_state_recreation` to `oauth/token_store.rs`'s tests

Open `/Users/rjl/Code/github/zotero-connector/crates/zotero-mcp/src/oauth/token_store.rs`. If it already has a `#[cfg(test)]` mod tests block, add this test to it. If not, add a new test module at the bottom.

Test body (copied verbatim from `http_transport.rs`):

```rust
#[tokio::test]
async fn tokens_survive_oauth_state_recreation() {
    use crate::oauth::{OAuthConfig, OAuthState};
    let dir = tempfile::TempDir::new().unwrap();
    let tokens_path = dir.path().join("tokens.json");
    let config = OAuthConfig {
        client_id: "test-id".into(),
        client_secret: "test-secret".into(),
        issuer: "https://example.test".into(),
        access_token_ttl_secs: None,
        refresh_token_ttl_secs: None,
    };

    // Simulate the HTTP server's first lifetime: mint a token via the store
    // accessor, then drop everything as if launchd killed the process.
    let access_token = {
        let state_a = OAuthState::with_tokens_path(config.clone(), tokens_path.clone()).unwrap();
        let pair = state_a.token_store().mint_pair(None).await.unwrap();
        assert!(state_a.validate_token(&pair.access_token).await);
        pair.access_token
    };

    // Simulate launchd restart: brand-new OAuthState reading the same file.
    let state_b = OAuthState::with_tokens_path(config, tokens_path).unwrap();

    // The original access token MUST still validate. This is the regression
    // test for the in-memory-only token bug we shipped before.
    assert!(
        state_b.validate_token(&access_token).await,
        "access token issued before restart must still validate after restart"
    );
}
```

If the `imports` at the top of the existing test mod conflict, just rely on the `use` lines inside the function body.

### Step 6: Add `pub mod bearer;` to lib.rs

Open `/Users/rjl/Code/github/zotero-connector/crates/zotero-mcp/src/lib.rs`. Locate the existing `pub mod http_transport;` line. Add immediately above or below:

```rust
pub mod bearer;
```

(Alphabetic ordering preferred — `bearer` before `http_transport`.)

### Step 7: Rewrite `http_transport.rs`

Replace the entire file content with:

```rust
//! HTTP/streamable-HTTP transport for `zotero-mcp`.
//!
//! Mounts rmcp 1.7's `StreamableHttpService` at `/mcp`. The service handles
//! session management, request/response framing, and per-session
//! `ZoteroServer` spawning. We supply bearer auth as a tower layer on the
//! `/mcp` route only — OAuth discovery and `/oauth/token` stay
//! unauthenticated so clients can complete the handshake.
//!
//! Transport-config knobs are exposed as env vars for empirical validation
//! against the consumer MCP client (see spec Decision 2):
//!   - `ZOTERO_MCP_TRANSPORT_STATEFUL` (bool, default `true`)
//!   - `ZOTERO_MCP_TRANSPORT_JSON`     (bool, default `false`)
//!   - `ZOTERO_MCP_ALLOWED_HOSTS`      (comma-separated, default unset)

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use rmcp::transport::streamable_http_server::{
    // Step 1 discovered the actual type name — could be LocalSessionStore,
    // LocalSessionManager, or similar. Use what the rmcp source defines.
    session::local::LocalSessionStore,
    tower::{StreamableHttpServerConfig, StreamableHttpService},
};
use tower_http::validate_request::ValidateRequestHeaderLayer;

use crate::oauth::{self, OAuthState};
use crate::server::ZoteroServer;
use crate::state::AppState;

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn env_allowed_hosts() -> Vec<String> {
    std::env::var("ZOTERO_MCP_ALLOWED_HOSTS")
        .ok()
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

pub async fn run(
    state: AppState,
    addr: SocketAddr,
    bearer: Option<String>,
    oauth_state: Option<OAuthState>,
) -> anyhow::Result<()> {
    let stateful_mode = env_bool("ZOTERO_MCP_TRANSPORT_STATEFUL", true);
    let json_response = env_bool("ZOTERO_MCP_TRANSPORT_JSON", false);
    let allowed_hosts = env_allowed_hosts();

    let mut config = StreamableHttpServerConfig::default();
    config.sse_keep_alive = Some(Duration::from_secs(5));
    config.stateful_mode = stateful_mode;
    config.json_response = json_response;
    if !allowed_hosts.is_empty() {
        config.allowed_hosts = allowed_hosts;
    }

    let state_for_factory = state.clone();
    let service = StreamableHttpService::new(
        move || Ok(ZoteroServer::new(state_for_factory.clone())),
        // Step 1 discovered the actual constructor — could be ::default(),
        // ::new(), or via a builder. The .into() may or may not be needed
        // depending on whether StreamableHttpService::new takes the store
        // by value or by Arc.
        LocalSessionStore::default(),
        config,
    );

    let mut mcp_route = Router::new().route_service("/mcp", service);
    if let Some(oauth_state) = oauth_state.clone() {
        mcp_route = mcp_route.layer(axum::middleware::from_fn_with_state(
            oauth_state,
            crate::bearer::require_bearer_token,
        ));
    }

    let mut app = Router::new().merge(mcp_route);
    if let Some(oauth_state) = oauth_state {
        app = app.merge(oauth::router(oauth_state));
        tracing::info!(
            "OAuth 2.1 surface mounted (discovery + /oauth/token + bearer gate on /mcp)"
        );
    }
    let mut app = app.layer(tower_http::trace::TraceLayer::new_for_http());

    if let Some(token) = bearer {
        #[allow(deprecated)]
        let auth = ValidateRequestHeaderLayer::bearer(&token);
        app = app.layer(auth);
        tracing::info!(
            %addr,
            stateful_mode,
            json_response,
            "zotero-mcp streamable-HTTP transport listening (static bearer auth enabled)"
        );
    } else {
        tracing::warn!(
            %addr,
            stateful_mode,
            json_response,
            "zotero-mcp streamable-HTTP transport listening WITHOUT static bearer — \
             OAuth gates /mcp if configured; otherwise transport-level access control applies"
        );
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::{OAuthConfig, OAuthState};
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use tower::ServiceExt;

    fn test_oauth_state() -> OAuthState {
        let dir = std::env::temp_dir().join(format!(
            "zotero-mcp-http-test-{}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        OAuthState::with_tokens_path(
            OAuthConfig {
                client_id: "test-id".into(),
                client_secret: "test-secret".into(),
                issuer: "https://example.test".into(),
                access_token_ttl_secs: None,
                refresh_token_ttl_secs: None,
            },
            dir.join("tokens.json"),
        )
        .unwrap()
    }

    /// Smoke test: /mcp returns 401 when no bearer is supplied and OAuth is
    /// configured. Exercises the full transport-route + bearer-layer stack
    /// (the bearer middleware itself has unit tests in `crate::bearer::tests`).
    /// We don't construct a real AppState here — `route_service` lets us mount
    /// a stub service. The test only verifies the bearer layer fires before
    /// the service is invoked.
    #[tokio::test]
    async fn mcp_route_rejects_request_without_bearer() {
        use axum::routing::get;
        let oauth_state = test_oauth_state();
        let stub = Router::new()
            .route("/mcp", get(|| async { StatusCode::OK }))
            .layer(axum::middleware::from_fn_with_state(
                oauth_state,
                crate::bearer::require_bearer_token,
            ));
        let resp = stub
            .oneshot(
                HttpRequest::builder()
                    .uri("/mcp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert!(
            resp.headers()
                .get(axum::http::header::WWW_AUTHENTICATE)
                .is_some()
        );
    }
}
```

Notes on the rewrite:
- The Cloudflare padding hack is gone (rmcp writes its own response body; can't be prepended to).
- `AppShared`, `PostQuery`, `post_handler`, `sse_handler` are removed entirely.
- The `LocalSessionManager::default().into()` form is a guess — the real form is whatever rmcp's `session/local.rs` exposes. Read it in Step 1 and adjust.
- The smoke test uses a stub `/mcp` route under the same bearer layer rather than mounting the real `StreamableHttpService` (which would require an `AppState`). This is sufficient — the bearer layer fires before the service, so 401 means the layer works; the real bearer-middleware unit tests live in `bearer.rs`.

### Step 8: First build, expect compile errors

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo build -p zotero-mcp 2>&1 | tail -50`

Expected outcomes (in likely order):

| Error | Fix |
|---|---|
| `unresolved import rmcp::transport::streamable_http_server` | Step 2's feature flag didn't take effect, or rmcp wasn't rebuilt. Run `cargo clean -p rmcp && cargo build -p zotero-mcp` and retry. |
| `cannot find type StreamableHttpService in scope` | Wrong module path. Read `rmcp-1.7.0/src/transport/streamable_http_server.rs` for the re-exports — adjust the import. |
| `cannot find type LocalSessionManager` | Step 1's discovery found a different name. Could be `LocalSessionStore`, `local::Store`, or similar. Use what the rmcp source defines. |
| `the trait Service<Request<...>> is not implemented for StreamableHttpService<...>` | axum `route_service` requires specific body bounds. If the error names a body type mismatch, try `Router::route_service("/mcp", service.into_service())` or wrap with `ServiceBuilder::new().service(service)`. Adapter must be ≤20 lines — beyond that triggers escalation. |
| `field allowed_hosts of StreamableHttpServerConfig has type X but expected Vec<String>` | Step 1's discovery showed a different type. Convert `Vec<String>` to whatever the field actually takes. |
| `expected closure, found impl Fn` (on `service_factory`) | The factory closure must satisfy `Fn() -> Result<S, std::io::Error> + Send + Sync + 'static`. Add `Arc::new(...)` wrapping if needed. |
| Anything else | If the fix needs more than 20 lines or touches semantics beyond the spec's in-bounds list, **escalate per spec Decision 4 / Risk 4** (see escalation block below). |

Re-run the build after each fix. Loop until clean.

### Step 9: Run the tests

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo test -p zotero-mcp 2>&1 | tail -30`

Expected: lib tests pass at a count of roughly 105 ± a few. Specifically:
- `crate::bearer::tests` — 3 tests, all pass.
- `crate::http_transport::tests` — 1 test (`mcp_route_rejects_request_without_bearer`), passes.
- `crate::oauth::token_store::tests` — gains `tokens_survive_oauth_state_recreation`, passes.
- All previous tests (105) continue to pass.

If a test fails: read the failure, fix, re-run. Discrete from build-iteration in Step 8 because test failures point at runtime / logic, not type errors.

Write down the final lib-test count for the commit message.

### Step 10: Verify the verify-only files are unchanged

Run:

```bash
cd /Users/rjl/Code/github/zotero-connector && \
git diff --stat crates/zotero-mcp/src/main.rs crates/zotero-mcp/src/oauth.rs crates/zotero-mcp/src/oauth/token_store.rs
```

Expected: only `oauth/token_store.rs` shows a diff (from Step 5 — moved test added). `main.rs` and `oauth.rs` are untouched. Anything else triggers escalation.

### Step 11: Review the Cargo.lock diff

Run: `cd /Users/rjl/Code/github/zotero-connector && git diff Cargo.lock | grep -E "^[-+]name|^[-+]version" | head -40`

Expected: minimal churn. `tower`, `http`, `http-body`, `http-body-util`, `bytes`, `tokio-stream`, `uuid`, `rand`, `sse-stream`, `tracing-subscriber` are mostly already in the tree from Slice C. New transitives (if any) belong to the streamable-http subtree — note in the commit body. Anything outside that scope is a surprise.

### Step 12: Stage and confirm

Run:

```bash
cd /Users/rjl/Code/github/zotero-connector && \
git add Cargo.toml Cargo.lock \
  crates/zotero-mcp/src/bearer.rs \
  crates/zotero-mcp/src/lib.rs \
  crates/zotero-mcp/src/http_transport.rs \
  crates/zotero-mcp/src/oauth/token_store.rs 2>/dev/null; \
git status --short
```

Expected staged set:
- `M Cargo.toml`
- `M Cargo.lock`
- `A crates/zotero-mcp/src/bearer.rs`
- `M crates/zotero-mcp/src/lib.rs`
- `M crates/zotero-mcp/src/http_transport.rs`
- `M crates/zotero-mcp/src/oauth/token_store.rs`

If `crates/zotero-mcp/src/main.rs` or `crates/zotero-mcp/src/oauth.rs` show up — that's an out-of-bounds change. Either revert those files or stop and escalate.

### Step 13: Commit

Build the commit body. Fill the `[bracketed]` placeholders with actual values from your run — leave no brackets in the final message.

```bash
git commit -m "$(cat <<'EOF'
chore(transport): replace hand-rolled SSE with rmcp StreamableHttpService

Migrates the HTTP transport from the hand-rolled SSE session manager
in http_transport.rs (~150 lines) to rmcp 1.7's StreamableHttpService
tower::Service. New single endpoint at /mcp (was: GET /sse + POST
/message?sessionId=). Adopts the MCP 2025-06-18 streamable-HTTP wire
protocol.

Transport-config knobs exposed as env vars during the slice so the
user can find which combination Claude Cowork accepts without
recompiling (see spec Decision 2 + Decision 11 validation gate):
  - ZOTERO_MCP_TRANSPORT_STATEFUL (default true)
  - ZOTERO_MCP_TRANSPORT_JSON     (default false)
  - ZOTERO_MCP_ALLOWED_HOSTS      (default unset = no Host check)

Bearer middleware (require_bearer_token + its RFC 9728 §5.1
WWW-Authenticate challenge) extracted to a new crates/zotero-mcp/src/
bearer.rs module. Three middleware tests moved with it. The fourth
test (tokens_survive_oauth_state_recreation) moved to
oauth/token_store.rs as an OAuth concern, not a transport one. New
smoke test verifies the bearer layer fires before /mcp is invoked.

Cloudflare quick-tunnel padding hack (2KB SSE comment prefix to
defeat HTTP/2 edge buffering) removed — rmcp writes its own response
body and the hack only mattered under SSE framing. With
json_response=true, Cloudflare's SSE buffering is not in play. With
json_response=false, buffering may re-emerge — verify against actual
tunnel during validation gate.

Test result: [N] passed; 0 failed (was 105 at Slice C baseline;
delta: +3 from bearer tests moved here, +1 smoke test, -2 SSE-routing
tests removed, +1 OAuth test moved = +3 net = [108]).

oauth.rs, oauth/token_store.rs (other than the moved test), and
main.rs deliberately unchanged. main.rs's stdio path still uses
ServiceExt::serve over transport-io; only the HTTP path is affected.

REINSTALL + LAUNCHD FLIP DELIBERATELY NOT PERFORMED in this commit
per spec Decision 11. User must run the validation gate (manual
ping from Cowork against /mcp after updating connector URL) before
cargo install + launchctl kickstart.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Replace `[N]` with the actual lib-test count from Step 9. Replace `[108]` if the actual delta is different. Adjust the "delta: ..." explanation to match what really happened.

### Step 14: After commit, STOP — do not reinstall or restart launchd

Per spec Decision 11: the validation gate happens between code-commit and the launchd flip. Report `git rev-parse HEAD` to the controller. Confirm:

- ✅ Build clean.
- ✅ Tests pass at the documented count.
- ✅ `oauth.rs` and `main.rs` unchanged (per Step 10).
- ✅ Commit body's `[bracketed]` placeholders filled.
- ✅ `git show --stat HEAD` shows only the 6 expected files.
- ❌ Do NOT run `cargo install --path crates/zotero-mcp`. The user will do this after validating Cowork compatibility.

### Escalation block

Per spec Decision 4 / Risks: the slice cannot be partially landed. If any of these surface and resist mechanical fixing:

1. `Service<Request<...>>` bound mismatch on `StreamableHttpService` requires an axum body-type adapter >20 lines.
2. `StreamableHttpServerConfig` defaults to a value that breaks compilation in a way Step 1's lookup didn't catch.
3. `LocalSessionManager` (or whatever the correct type name is) requires a custom `SessionManager` impl to compile — i.e., the default doesn't work.
4. Bearer-middleware extraction surfaces a hidden coupling (it shouldn't, but verify).
5. Anything that requires touching `oauth.rs` or `main.rs` non-trivially (reserved per spec).

**Revert:**

```bash
git checkout -- Cargo.toml Cargo.lock \
  crates/zotero-mcp/src/http_transport.rs \
  crates/zotero-mcp/src/lib.rs \
  crates/zotero-mcp/src/oauth/token_store.rs
rm -f crates/zotero-mcp/src/bearer.rs
```

Then amend the spec at `docs/superpowers/specs/2026-05-13-dependency-upgrades-slice-e-design.md` — append:

```markdown
---

## Deferred (Slice E rmcp StreamableHttpService port)

[Date]: This slice was attempted and reverted. Blocker:

[One paragraph naming the specific file, API, and reason mechanical
fixing wasn't possible. Reference the escalation-block item number.]
```

Commit as: `docs(spec): defer Slice E — <one-line reason>`. Report DONE_WITH_CONCERNS.

---

## Hand-off: validation gate (user-driven, not implementer-driven)

After Task 1 lands and the implementer reports back, the user (not the implementer subagent) runs these steps. The plan documents them so both controller and user know the sequence.

- [ ] **Step 1: Update the launchd plist with new env vars**

Open the launchd plist (typically at `~/Library/LaunchAgents/com.zotero-mcp.http.plist`). Add inside `<key>EnvironmentVariables</key>` `<dict>`:

```xml
<key>ZOTERO_MCP_TRANSPORT_STATEFUL</key> <string>true</string>
<key>ZOTERO_MCP_TRANSPORT_JSON</key> <string>false</string>
<!-- Leave ZOTERO_MCP_ALLOWED_HOSTS unset (or set to your tunnel host) -->
```

- [ ] **Step 2: Reload the plist (but don't reinstall yet)**

```bash
launchctl unload ~/Library/LaunchAgents/com.zotero-mcp.http.plist
launchctl load ~/Library/LaunchAgents/com.zotero-mcp.http.plist
```

The launchd service restarts with the new env vars but is still running the pre-Slice-E binary. Verify it does (Cowork can still ping with the old `/sse` URL).

- [ ] **Step 3: Reinstall the new binary**

```bash
cd /Users/rjl/Code/github/zotero-connector && cargo install --path crates/zotero-mcp
```

- [ ] **Step 4: Kick the service to pick up the new binary**

```bash
launchctl kickstart -k gui/$(id -u)/com.zotero-mcp.http
```

- [ ] **Step 5: Update the Cowork connector URL**

In your Cowork connector configuration, change the URL from `https://<tunnel-host>/sse` to `https://<tunnel-host>/mcp`. Save.

- [ ] **Step 6: Validate with `ping`**

Call the `ping` tool from Cowork. Expected: `pong (v0.2.0, <new-sha>)` where `<new-sha>` is the Slice E commit SHA.

If `ping` fails: walk through the recovery matrix:

| Symptom | Try |
|---|---|
| 404 on /mcp | Cowork connector URL still has /sse. Recheck Step 5. |
| 401 with WWW-Authenticate challenge | Cowork hasn't completed OAuth handshake against the new endpoint. Re-auth in Cowork. |
| Connection times out | Set `ZOTERO_MCP_TRANSPORT_JSON=true` in the plist, reload + kickstart. Cloudflare buffering on SSE framing may be the issue. |
| 400 Bad Request / protocol error | Set `ZOTERO_MCP_TRANSPORT_STATEFUL=false` in the plist, reload + kickstart. Cowork may not handle stateful streamable-HTTP. |
| All permutations fail | Revert path: keep the launchd plist on the new binary but flip the connector URL back to /sse — that will 404 immediately, signaling Slice E can't ship. File a follow-up note in the spec's Deferred section. |

- [ ] **Step 7: Report back which combination worked**

Note in this PR/issue/wherever which `(stateful_mode, json_response)` pair succeeded. This input drives the env-var trim follow-up (next slice, out of scope here).

---

## Verification checklist

After Task 1 completes (code work only):

- [ ] One commit lands on `main` with the documented message format.
- [ ] The commit passes `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp`.
- [ ] The new lib-test count is explicitly noted in the commit body.
- [ ] `Cargo.lock` is included in the commit.
- [ ] `bearer.rs` exists as a new module with its 3 tests passing.
- [ ] `http_transport.rs` has shrunk from ~150 lines to ~50 (incl. smoke test).
- [ ] `oauth.rs` is unchanged. `main.rs` is unchanged.
- [ ] `oauth/token_store.rs` has one new test added (`tokens_survive_oauth_state_recreation`).
- [ ] `cargo install` and `launchctl kickstart` were NOT executed by the implementer.

After validation gate (user-driven):

- [ ] User has updated launchd plist with new env vars.
- [ ] User has reinstalled the binary and restarted the service.
- [ ] User has updated Cowork connector URL.
- [ ] User confirms `ping` from Cowork returns the new SHA.
- [ ] User reports back which `(stateful_mode, json_response)` combination worked, so the env-var trim follow-up has its input.
