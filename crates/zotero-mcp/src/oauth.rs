//! OAuth 2.1 authorization surface for the HTTP/SSE transport.
//!
//! Claude.ai's "Add custom connector" dialog exposes `client_id`/`client_secret`
//! fields in its Advanced section. That UI shape — pre-shared credentials, no
//! redirect URI, no consent screen — maps cleanly onto the OAuth 2.1
//! Client Credentials grant (RFC 6749 §4.4). The MCP authorization spec
//! (2025-11-25) requires Authorization Server Metadata (RFC 8414) and
//! Protected Resource Metadata (RFC 9728) discovery so clients can find these
//! endpoints; both are served unauthenticated.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Form, Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

const TOKEN_TTL_SECS: u64 = 3600;

/// Pre-shared OAuth client credentials loaded from config or env. Task D moves
/// this onto disk; for now it can also come from environment variables so the
/// path is testable in isolation.
#[derive(Clone, Debug)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Public origin advertised in discovery + used to compute the resource
    /// metadata URL in 401 challenges. Must match what the client believes the
    /// canonical URI is (e.g. the Tailscale Funnel hostname).
    pub issuer: String,
}

/// Shared runtime state for the OAuth surface. Cheaply cloneable.
#[derive(Clone)]
pub struct OAuthState {
    inner: Arc<Inner>,
}

struct Inner {
    config: OAuthConfig,
    /// In-memory access-token store. Tokens are opaque 32-byte hex strings;
    /// values are unix expiry timestamps. Restarts invalidate every token —
    /// clients re-acquire on the next 401.
    tokens: RwLock<HashMap<String, u64>>,
}

impl OAuthState {
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                tokens: RwLock::new(HashMap::new()),
            }),
        }
    }

    pub fn issuer(&self) -> &str {
        &self.inner.config.issuer
    }

    /// URL the WWW-Authenticate challenge points clients at for resource
    /// metadata. Defined by RFC 9728 §5.1.
    pub fn resource_metadata_url(&self) -> String {
        format!(
            "{}/.well-known/oauth-protected-resource",
            self.inner.config.issuer
        )
    }

    /// Returns true iff the token was previously issued and is not expired.
    pub async fn validate_token(&self, token: &str) -> bool {
        let now = unix_now();
        let map = self.inner.tokens.read().await;
        map.get(token).is_some_and(|exp| *exp > now)
    }

    async fn mint_token(&self) -> (String, u64) {
        let token = format!("{:032x}", rand::random::<u128>());
        let exp = unix_now() + TOKEN_TTL_SECS;
        self.inner
            .tokens
            .write()
            .await
            .insert(token.clone(), exp);
        (token, TOKEN_TTL_SECS)
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Serialize)]
struct AuthorizationServerMetadata {
    issuer: String,
    token_endpoint: String,
    grant_types_supported: &'static [&'static str],
    token_endpoint_auth_methods_supported: &'static [&'static str],
    response_types_supported: &'static [&'static str],
    scopes_supported: &'static [&'static str],
}

#[derive(Serialize)]
struct ProtectedResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    bearer_methods_supported: &'static [&'static str],
    scopes_supported: &'static [&'static str],
}

async fn authorization_server_metadata(
    State(state): State<OAuthState>,
) -> Json<AuthorizationServerMetadata> {
    let issuer = state.issuer().to_string();
    Json(AuthorizationServerMetadata {
        token_endpoint: format!("{issuer}/oauth/token"),
        issuer,
        grant_types_supported: &["client_credentials"],
        token_endpoint_auth_methods_supported: &["client_secret_post", "client_secret_basic"],
        response_types_supported: &["token"],
        scopes_supported: &["mcp"],
    })
}

async fn protected_resource_metadata(
    State(state): State<OAuthState>,
) -> Json<ProtectedResourceMetadata> {
    let issuer = state.issuer().to_string();
    Json(ProtectedResourceMetadata {
        authorization_servers: vec![issuer.clone()],
        resource: issuer,
        bearer_methods_supported: &["header"],
        scopes_supported: &["mcp"],
    })
}

#[derive(Deserialize)]
struct TokenRequest {
    grant_type: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    /// RFC 8707 Resource Indicator. Accepted but not enforced — single-resource
    /// server.
    #[allow(dead_code)]
    #[serde(default)]
    resource: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: u64,
    scope: &'static str,
}

#[derive(Serialize)]
struct OAuthError {
    error: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_description: Option<&'static str>,
}

async fn token_handler(
    State(state): State<OAuthState>,
    headers: HeaderMap,
    Form(body): Form<TokenRequest>,
) -> axum::response::Response {
    if body.grant_type != "client_credentials" {
        return (
            StatusCode::BAD_REQUEST,
            Json(OAuthError {
                error: "unsupported_grant_type",
                error_description: Some("only client_credentials is supported"),
            }),
        )
            .into_response();
    }

    let Some((client_id, client_secret)) = resolve_client_credentials(&headers, &body) else {
        return invalid_client();
    };

    let expected = &state.inner.config;
    if !constant_time_eq(client_id.as_bytes(), expected.client_id.as_bytes())
        || !constant_time_eq(client_secret.as_bytes(), expected.client_secret.as_bytes())
    {
        return invalid_client();
    }

    let (token, ttl) = state.mint_token().await;
    tracing::info!(client_id = %client_id, expires_in = ttl, "OAuth token minted");
    (
        StatusCode::OK,
        Json(TokenResponse {
            access_token: token,
            token_type: "Bearer",
            expires_in: ttl,
            scope: "mcp",
        }),
    )
        .into_response()
}

/// Pull `client_id`/`client_secret` from the form body (client_secret_post)
/// or fall back to the `Authorization: Basic …` header (client_secret_basic).
fn resolve_client_credentials(
    headers: &HeaderMap,
    body: &TokenRequest,
) -> Option<(String, String)> {
    if let (Some(id), Some(secret)) = (body.client_id.as_ref(), body.client_secret.as_ref()) {
        return Some((id.clone(), secret.clone()));
    }
    let auth = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let encoded = auth.strip_prefix("Basic ")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let decoded = String::from_utf8(bytes).ok()?;
    let (id, secret) = decoded.split_once(':')?;
    Some((id.to_string(), secret.to_string()))
}

fn invalid_client() -> axum::response::Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Basic realm=\"oauth/token\"")],
        Json(OAuthError {
            error: "invalid_client",
            error_description: None,
        }),
    )
        .into_response()
}

/// Length-aware equality that avoids byte-by-byte short-circuit. For
/// fixed-length pre-shared secrets the length disclosure is irrelevant.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Build the public, unauthenticated OAuth surface (discovery + token).
pub fn router(state: OAuthState) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource_metadata),
        )
        .route("/oauth/token", post(token_handler))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_state() -> OAuthState {
        OAuthState::new(OAuthConfig {
            client_id: "test-id".into(),
            client_secret: "test-secret".into(),
            issuer: "https://example.test".into(),
        })
    }

    async fn body_string(resp: axum::response::Response) -> String {
        let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn token_endpoint_issues_for_valid_credentials_via_body() {
        let app = router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/oauth/token")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "grant_type=client_credentials&client_id=test-id&client_secret=test-secret",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("\"access_token\""), "body was: {body}");
        assert!(body.contains("\"token_type\":\"Bearer\""));
        assert!(body.contains("\"expires_in\":3600"));
    }

    #[tokio::test]
    async fn token_endpoint_accepts_basic_auth() {
        let app = router(test_state());
        let basic = base64::engine::general_purpose::STANDARD.encode("test-id:test-secret");
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/oauth/token")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("authorization", format!("Basic {basic}"))
                    .body(Body::from("grant_type=client_credentials"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn token_endpoint_rejects_bad_secret() {
        let app = router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/oauth/token")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "grant_type=client_credentials&client_id=test-id&client_secret=WRONG",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = body_string(resp).await;
        assert!(body.contains("\"error\":\"invalid_client\""));
    }

    #[tokio::test]
    async fn token_endpoint_rejects_unsupported_grant() {
        let app = router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/oauth/token")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "grant_type=password&client_id=test-id&client_secret=test-secret",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_string(resp).await;
        assert!(body.contains("unsupported_grant_type"));
    }

    #[tokio::test]
    async fn minted_token_validates_then_expires() {
        let state = test_state();
        let (token, _ttl) = state.mint_token().await;
        assert!(state.validate_token(&token).await);
        assert!(!state.validate_token("not-issued").await);
    }

    #[tokio::test]
    async fn discovery_documents_advertise_correct_endpoints() {
        let app = router(test_state());
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/.well-known/oauth-authorization-server")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("\"issuer\":\"https://example.test\""));
        assert!(body.contains("\"token_endpoint\":\"https://example.test/oauth/token\""));
        assert!(body.contains("\"client_credentials\""));

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/oauth-protected-resource")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("\"resource\":\"https://example.test\""));
        assert!(body.contains("\"authorization_servers\":[\"https://example.test\"]"));
    }
}
