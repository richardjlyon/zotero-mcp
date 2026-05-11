//! OAuth 2.1 authorization surface for the HTTP/SSE transport.
//!
//! Claude.ai's "Add custom connector" dialog exposes `client_id`/`client_secret`
//! fields in its Advanced section. That UI shape — pre-shared credentials, no
//! redirect URI, no consent screen — maps cleanly onto the OAuth 2.1
//! Client Credentials grant (RFC 6749 §4.4). The MCP authorization spec
//! (2025-11-25) requires Authorization Server Metadata (RFC 8414) and
//! Protected Resource Metadata (RFC 9728) discovery so clients can find these
//! endpoints; both are served unauthenticated.
//!
//! Status: Task A scaffolding. Discovery documents only. Token issuance and
//! validation arrive in Task B/C.

use axum::{Json, Router, routing::get};
use serde::Serialize;

/// Issuer + endpoint base used to render absolute URLs in the discovery
/// documents. For Cowork this is the Tailscale Funnel URL — Funnel terminates
/// TLS in front of the local 127.0.0.1 bind, so all advertised URLs must be
/// `https://` and must match what the client believes the public origin is.
///
/// Task A keeps this hard-coded; Task D moves it into the on-disk config.
const ISSUER: &str = "https://laptop.stoat-minnow.ts.net";

#[derive(Serialize)]
struct AuthorizationServerMetadata {
    issuer: &'static str,
    token_endpoint: String,
    grant_types_supported: &'static [&'static str],
    token_endpoint_auth_methods_supported: &'static [&'static str],
    response_types_supported: &'static [&'static str],
    scopes_supported: &'static [&'static str],
}

#[derive(Serialize)]
struct ProtectedResourceMetadata {
    resource: &'static str,
    authorization_servers: Vec<&'static str>,
    bearer_methods_supported: &'static [&'static str],
    scopes_supported: &'static [&'static str],
}

async fn authorization_server_metadata() -> Json<AuthorizationServerMetadata> {
    Json(AuthorizationServerMetadata {
        issuer: ISSUER,
        token_endpoint: format!("{ISSUER}/oauth/token"),
        grant_types_supported: &["client_credentials"],
        token_endpoint_auth_methods_supported: &["client_secret_post", "client_secret_basic"],
        response_types_supported: &["token"],
        scopes_supported: &["mcp"],
    })
}

async fn protected_resource_metadata() -> Json<ProtectedResourceMetadata> {
    Json(ProtectedResourceMetadata {
        resource: ISSUER,
        authorization_servers: vec![ISSUER],
        bearer_methods_supported: &["header"],
        scopes_supported: &["mcp"],
    })
}

/// Build the public, unauthenticated OAuth surface: discovery only for Task A.
/// Token endpoint joins this router in Task B.
pub fn router() -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource_metadata),
        )
}
