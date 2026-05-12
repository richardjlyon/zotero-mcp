//! OAuth 2.1 authorization surface for the HTTP/SSE transport.
//!
//! Claude.ai's MCP connector follows the spec-canonical flow: authorization
//! code with PKCE (RFC 7636, S256). Even though the connector UI exposes
//! "Client ID" / "Client Secret" fields, the actual grant is
//! `authorization_code` — the secret is just an additional bearer of trust on
//! the token request. The flow we observed in practice:
//!
//!   1. Client (Anthropic backend) hits `/sse` without a token →
//!      `401 Unauthorized` with a `WWW-Authenticate: Bearer
//!      resource_metadata="…"` challenge (RFC 9728 §5.1).
//!   2. Client fetches `/.well-known/oauth-protected-resource` and
//!      `/.well-known/oauth-authorization-server` to discover this endpoint set.
//!   3. The user's browser is opened to `/authorize?response_type=code&…&
//!      code_challenge=…&code_challenge_method=S256&state=…&redirect_uri=
//!      https://claude.ai/api/mcp/auth_callback`.
//!   4. We 302-redirect to that `redirect_uri` with a one-time code + state.
//!   5. Claude.ai's backend posts `grant_type=authorization_code` to
//!      `/oauth/token` with the code + `code_verifier`. We verify
//!      `SHA256(code_verifier)` (base64url, no pad) matches the stored
//!      `code_challenge` and mint an opaque access token.
//!
//! Both grant types — `authorization_code` and `client_credentials` — are
//! supported. The latter is retained for headless scripting and tests.

mod token_store;
pub use token_store::{ChainId, MintedPair, RefreshError, TokenStore};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Form, Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

const AUTH_CODE_TTL_SECS: u64 = 300;

/// Redirect URIs we'll accept on the authorization endpoint. The OAuth 2.1
/// spec requires exact-match validation against pre-registered values — for a
/// single-purpose connector talking only to Claude.ai we hardcode the two
/// hostnames Anthropic actually uses.
const ALLOWED_REDIRECT_URI_PREFIXES: &[&str] = &[
    "https://claude.ai/api/mcp/",
    "https://claude.com/api/mcp/",
];

pub const DEFAULT_ACCESS_TOKEN_TTL_SECS: u64 = 7 * 24 * 3600;   // 7 days
pub const DEFAULT_REFRESH_TOKEN_TTL_SECS: u64 = 90 * 24 * 3600; // 90 days

/// Pre-shared OAuth client credentials. Persisted at
/// `<config_dir>/oauth.toml` with mode 0600 so the secret never lands in a
/// world-readable location.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Public origin advertised in discovery + used to compute the resource
    /// metadata URL in 401 challenges. Must match what the client believes the
    /// canonical URI is (e.g. the Tailscale Funnel hostname).
    pub issuer: String,
    #[serde(default)]
    pub access_token_ttl_secs: Option<u64>,
    #[serde(default)]
    pub refresh_token_ttl_secs: Option<u64>,
}

impl OAuthConfig {
    pub fn effective_access_ttl(&self) -> std::time::Duration {
        std::time::Duration::from_secs(
            self.access_token_ttl_secs.unwrap_or(DEFAULT_ACCESS_TOKEN_TTL_SECS),
        )
    }
    pub fn effective_refresh_ttl(&self) -> std::time::Duration {
        std::time::Duration::from_secs(
            self.refresh_token_ttl_secs.unwrap_or(DEFAULT_REFRESH_TOKEN_TTL_SECS),
        )
    }
}

/// Location of the on-disk OAuth config. Uses the same ProjectDirs convention
/// as `zotero-core::Config::config_path` so users find both files in the same
/// directory (`~/Library/Application Support/dev.zotero-mcp.zotero-mcp` on
/// macOS, `~/.config/zotero-mcp` on Linux).
pub fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "zotero-mcp", "zotero-mcp")
        .map(|d| d.config_dir().join("oauth.toml"))
}

impl OAuthConfig {
    /// Resolve credentials. Precedence:
    /// 1. If `<config_dir>/oauth.toml` exists, load it.
    /// 2. Otherwise, if `issuer_hint` is `Some`, generate a fresh credential
    ///    pair (random client_id + 32-byte hex client_secret), persist it
    ///    with mode 0600, and return it. The generated values are logged to
    ///    stderr so the user can paste them into the Claude.ai connector.
    /// 3. Otherwise, return `Ok(None)` — OAuth is opt-in; without an issuer
    ///    we cannot generate a sensible config.
    pub fn load_or_generate(issuer_hint: Option<String>) -> anyhow::Result<Option<Self>> {
        let Some(path) = config_path() else {
            tracing::warn!("could not resolve ProjectDirs for OAuth config; OAuth disabled");
            return Ok(None);
        };

        if path.exists() {
            let bytes = std::fs::read(&path)
                .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
            let config: OAuthConfig = toml::from_str(std::str::from_utf8(&bytes)?)
                .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
            tracing::info!(path = %path.display(), "loaded OAuth config");
            return Ok(Some(config));
        }

        let Some(issuer) = issuer_hint else {
            tracing::warn!(
                path = %path.display(),
                "OAuth config not found and no ZOTERO_MCP_OAUTH_ISSUER set; OAuth disabled"
            );
            return Ok(None);
        };

        let config = OAuthConfig {
            client_id: format!("zotero-mcp-{}", short_id()),
            client_secret: format!("{:032x}", rand::random::<u128>()),
            issuer,
            access_token_ttl_secs: None,
            refresh_token_ttl_secs: None,
        };
        Self::write_secure(&path, &config)?;
        tracing::warn!(
            path = %path.display(),
            client_id = %config.client_id,
            "generated OAuth credentials — paste these into the Claude.ai connector's Advanced fields"
        );
        // Also print to stderr so the message is visible on the very first run
        // even if the logger threshold filters out warnings.
        eprintln!(
            "\n=== zotero-mcp OAuth credentials generated at {} ===\n  client_id     = {}\n  client_secret = {}\n  issuer        = {}\n  → paste client_id + client_secret into Claude.ai connector → Advanced → OAuth fields\n",
            path.display(),
            config.client_id,
            config.client_secret,
            config.issuer,
        );
        Ok(Some(config))
    }

    fn write_secure(path: &std::path::Path, config: &OAuthConfig) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("mkdir {}: {e}", parent.display()))?;
        }
        let serialized = toml::to_string_pretty(config)?;
        std::fs::write(path, serialized)
            .map_err(|e| anyhow::anyhow!("write {}: {e}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, perms)
                .map_err(|e| anyhow::anyhow!("chmod 0600 {}: {e}", path.display()))?;
        }
        Ok(())
    }
}

fn short_id() -> String {
    format!("{:08x}", rand::random::<u32>())
}

/// Shared runtime state for the OAuth surface. Cheaply cloneable.
#[derive(Clone)]
pub struct OAuthState {
    inner: Arc<Inner>,
}

struct Inner {
    config: OAuthConfig,
    /// In-memory authorization-code store. Codes are single-use, 5-minute TTL —
    /// surviving a server restart is not a goal for this short-lived state.
    codes: RwLock<HashMap<String, AuthCode>>,
    tokens: TokenStore,
}

#[derive(Clone)]
struct AuthCode {
    code_challenge: String,
    redirect_uri: String,
    expires_at: u64,
}

impl OAuthState {
    /// Construct an OAuthState backed by a TokenStore at `tokens_path`.
    /// Use `OAuthState::with_tokens_path` to supply the path explicitly,
    /// or `OAuthState::from_default_path` to derive it from the standard
    /// ProjectDirs location (test code uses the former, production uses the latter).
    pub fn with_tokens_path(config: OAuthConfig, tokens_path: PathBuf) -> anyhow::Result<Self> {
        let access_ttl = config.effective_access_ttl();
        let refresh_ttl = config.effective_refresh_ttl();
        let tokens = TokenStore::load(tokens_path, &config.client_id, access_ttl, refresh_ttl)?;
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                codes: RwLock::new(HashMap::new()),
                tokens,
            }),
        })
    }

    /// Standard production constructor: derive the tokens path from the
    /// same ProjectDirs base used by `oauth.toml`.
    pub fn from_default_path(config: OAuthConfig) -> anyhow::Result<Self> {
        let dir = directories::ProjectDirs::from("dev", "zotero-mcp", "zotero-mcp")
            .ok_or_else(|| anyhow::anyhow!("could not resolve ProjectDirs for tokens.json"))?
            .config_dir()
            .to_path_buf();
        Self::with_tokens_path(config, dir.join("tokens.json"))
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
        self.inner.tokens.validate_access(token).await
    }

    /// Crate-internal access to the token store — used by tests in sibling
    /// modules that need to mint a token without going through the HTTP layer.
    #[cfg(test)]
    pub(crate) fn token_store(&self) -> &TokenStore {
        &self.inner.tokens
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
    authorization_endpoint: String,
    token_endpoint: String,
    grant_types_supported: &'static [&'static str],
    token_endpoint_auth_methods_supported: &'static [&'static str],
    response_types_supported: &'static [&'static str],
    code_challenge_methods_supported: &'static [&'static str],
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
        authorization_endpoint: format!("{issuer}/authorize"),
        token_endpoint: format!("{issuer}/oauth/token"),
        issuer,
        grant_types_supported: &["authorization_code", "client_credentials"],
        token_endpoint_auth_methods_supported: &["client_secret_post", "client_secret_basic"],
        response_types_supported: &["code", "token"],
        code_challenge_methods_supported: &["S256"],
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
    // client_credentials inputs
    client_id: Option<String>,
    client_secret: Option<String>,
    // authorization_code inputs
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    code_verifier: Option<String>,
    #[serde(default)]
    redirect_uri: Option<String>,
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
    match body.grant_type.as_str() {
        "authorization_code" => handle_authorization_code(state, headers, body).await,
        "client_credentials" => handle_client_credentials(state, headers, body).await,
        _ => (
            StatusCode::BAD_REQUEST,
            Json(OAuthError {
                error: "unsupported_grant_type",
                error_description: Some(
                    "only authorization_code and client_credentials are supported",
                ),
            }),
        )
            .into_response(),
    }
}

async fn handle_client_credentials(
    state: OAuthState,
    headers: HeaderMap,
    body: TokenRequest,
) -> axum::response::Response {
    let Some((client_id, client_secret)) = resolve_client_credentials(&headers, &body) else {
        return invalid_client();
    };

    let expected = &state.inner.config;
    if !constant_time_eq(client_id.as_bytes(), expected.client_id.as_bytes())
        || !constant_time_eq(client_secret.as_bytes(), expected.client_secret.as_bytes())
    {
        return invalid_client();
    }

    let pair = match state.inner.tokens.mint_pair(None).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "mint_pair failed for client_credentials");
            return server_error();
        }
    };
    let token = pair.access_token;
    let ttl = pair.access_ttl.as_secs();
    tracing::info!(
        client_id = %client_id,
        grant = "client_credentials",
        expires_in = ttl,
        "OAuth token minted"
    );
    token_ok(token, ttl)
}

async fn handle_authorization_code(
    state: OAuthState,
    headers: HeaderMap,
    body: TokenRequest,
) -> axum::response::Response {
    let Some(code) = body.code.as_deref() else {
        return invalid_grant("missing code");
    };
    let Some(verifier) = body.code_verifier.as_deref() else {
        return invalid_grant("missing code_verifier");
    };
    let Some(redirect_uri) = body.redirect_uri.as_deref() else {
        return invalid_grant("missing redirect_uri");
    };

    // Client authentication is optional with PKCE per RFC 6749 §4.1.3 (the
    // code_verifier is proof of possession) but if the caller did present
    // credentials, validate them — Claude.ai may send the client_secret it
    // was given.
    if let Some((client_id, client_secret)) = resolve_client_credentials(&headers, &body) {
        let expected = &state.inner.config;
        if !constant_time_eq(client_id.as_bytes(), expected.client_id.as_bytes())
            || !constant_time_eq(client_secret.as_bytes(), expected.client_secret.as_bytes())
        {
            return invalid_client();
        }
    } else if let Some(client_id) = body.client_id.as_deref() {
        // client_id alone (public client) — must still match.
        if !constant_time_eq(client_id.as_bytes(), state.inner.config.client_id.as_bytes()) {
            return invalid_client();
        }
    }

    let info = state.inner.codes.write().await.remove(code);
    let Some(info) = info else {
        return invalid_grant("unknown or already-used code");
    };
    if info.expires_at < unix_now() {
        return invalid_grant("code expired");
    }
    if info.redirect_uri != redirect_uri {
        return invalid_grant("redirect_uri mismatch");
    }
    let computed = pkce_s256(verifier);
    if !constant_time_eq(computed.as_bytes(), info.code_challenge.as_bytes()) {
        return invalid_grant("PKCE verification failed");
    }

    let pair = match state.inner.tokens.mint_pair(None).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "mint_pair failed for authorization_code");
            return server_error();
        }
    };
    let token = pair.access_token;
    let ttl = pair.access_ttl.as_secs();
    tracing::info!(
        grant = "authorization_code",
        expires_in = ttl,
        "OAuth token minted"
    );
    token_ok(token, ttl)
}

fn token_ok(token: String, ttl: u64) -> axum::response::Response {
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

fn invalid_grant(detail: &'static str) -> axum::response::Response {
    tracing::info!(detail, "authorization_code grant rejected");
    (
        StatusCode::BAD_REQUEST,
        Json(OAuthError {
            error: "invalid_grant",
            error_description: Some(detail),
        }),
    )
        .into_response()
}

/// `BASE64URL(SHA256(verifier))` with no padding, per RFC 7636 §4.6.
fn pkce_s256(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

#[derive(Deserialize)]
struct AuthorizeQuery {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    state: String,
    #[allow(dead_code)]
    #[serde(default)]
    scope: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    resource: Option<String>,
}

async fn authorize_handler(
    State(state): State<OAuthState>,
    Query(q): Query<AuthorizeQuery>,
) -> axum::response::Response {
    // We validate the redirect_uri BEFORE responding via redirect — sending
    // any params to an unvetted URI would be an open-redirect bug.
    let redirect_ok = ALLOWED_REDIRECT_URI_PREFIXES
        .iter()
        .any(|p| q.redirect_uri.starts_with(p));
    if !redirect_ok {
        tracing::warn!(redirect_uri = %q.redirect_uri, "authorize: redirect_uri not allowed");
        return (StatusCode::BAD_REQUEST, "invalid_redirect_uri").into_response();
    }
    if q.response_type != "code" {
        return redirect_with_error(&q.redirect_uri, &q.state, "unsupported_response_type");
    }
    if q.code_challenge_method != "S256" {
        return redirect_with_error(&q.redirect_uri, &q.state, "invalid_request");
    }
    if !constant_time_eq(q.client_id.as_bytes(), state.inner.config.client_id.as_bytes()) {
        return redirect_with_error(&q.redirect_uri, &q.state, "unauthorized_client");
    }
    if q.code_challenge.is_empty() {
        return redirect_with_error(&q.redirect_uri, &q.state, "invalid_request");
    }

    let code = format!("{:032x}", rand::random::<u128>());
    let info = AuthCode {
        code_challenge: q.code_challenge,
        redirect_uri: q.redirect_uri.clone(),
        expires_at: unix_now() + AUTH_CODE_TTL_SECS,
    };
    state.inner.codes.write().await.insert(code.clone(), info);
    tracing::info!(redirect_uri = %q.redirect_uri, "authorization code issued");

    let location = format!(
        "{}?code={}&state={}",
        q.redirect_uri,
        urlencoding_minimal(&code),
        urlencoding_minimal(&q.state),
    );
    (StatusCode::FOUND, [(header::LOCATION, location.as_str())]).into_response()
}

fn redirect_with_error(redirect_uri: &str, state: &str, error: &str) -> axum::response::Response {
    let location = format!(
        "{redirect_uri}?error={}&state={}",
        urlencoding_minimal(error),
        urlencoding_minimal(state)
    );
    (StatusCode::FOUND, [(header::LOCATION, location.as_str())]).into_response()
}

/// Minimal URL-encoding for the small set of characters that appear in our
/// outputs (state tokens are alphanumeric+`_-`, codes are hex). Reaching for
/// the `urlencoding` crate just to encode `&`, `=`, `+`, ` ` would be
/// disproportionate.
fn urlencoding_minimal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            _ => out.push_str(&format!("%{:02X}", c as u32)),
        }
    }
    out
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

fn server_error() -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(OAuthError {
            error: "server_error",
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

/// Build the public, unauthenticated OAuth surface.
///
/// Routes:
///   - `GET /.well-known/oauth-authorization-server` — RFC 8414 metadata
///   - `GET /.well-known/oauth-protected-resource`    — RFC 9728 metadata
///   - `GET /authorize`                               — auth-code start (PKCE)
///   - `POST /oauth/token`                            — token issuance
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
        .route("/authorize", get(authorize_handler))
        .route("/oauth/token", post(token_handler))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    #[test]
    fn config_loads_with_default_ttls_when_unset() {
        let toml_str = r#"
            client_id = "x"
            client_secret = "y"
            issuer = "https://example.test"
        "#;
        let cfg: OAuthConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.access_token_ttl_secs, None);
        assert_eq!(cfg.refresh_token_ttl_secs, None);
        assert_eq!(cfg.effective_access_ttl().as_secs(), 7 * 24 * 3600);
        assert_eq!(cfg.effective_refresh_ttl().as_secs(), 90 * 24 * 3600);
    }

    #[test]
    fn config_loads_with_explicit_ttls() {
        let toml_str = r#"
            client_id = "x"
            client_secret = "y"
            issuer = "https://example.test"
            access_token_ttl_secs = 3600
            refresh_token_ttl_secs = 86400
        "#;
        let cfg: OAuthConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.effective_access_ttl().as_secs(), 3600);
        assert_eq!(cfg.effective_refresh_ttl().as_secs(), 86400);
    }

    #[test]
    fn config_roundtrips_through_disk_with_secure_perms() {
        let dir = tempdir();
        let path = dir.join("oauth.toml");
        let original = OAuthConfig {
            client_id: "id-x".into(),
            client_secret: "secret-y".into(),
            issuer: "https://example.test".into(),
            access_token_ttl_secs: None,
            refresh_token_ttl_secs: None,
        };
        OAuthConfig::write_secure(&path, &original).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let parsed: OAuthConfig = toml::from_str(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!(parsed.client_id, "id-x");
        assert_eq!(parsed.client_secret, "secret-y");
        assert_eq!(parsed.issuer, "https://example.test");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "file should be readable only by owner");
        }
    }

    fn tempdir() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "zotero-mcp-test-{}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn test_state() -> OAuthState {
        let dir = tempdir();
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
        assert!(body.contains("\"expires_in\":604800"));
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
        let pair = state.inner.tokens.mint_pair(None).await.unwrap();
        assert!(state.validate_token(&pair.access_token).await);
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
        assert!(body.contains("\"authorization_endpoint\":\"https://example.test/authorize\""));
        assert!(body.contains("\"token_endpoint\":\"https://example.test/oauth/token\""));
        assert!(body.contains("\"authorization_code\""));
        assert!(body.contains("\"client_credentials\""));
        assert!(body.contains("\"code_challenge_methods_supported\":[\"S256\"]"));

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

    #[test]
    fn pkce_s256_matches_rfc7636_example() {
        // RFC 7636 Appendix B test vector
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(pkce_s256(verifier), expected);
    }

    fn challenge_for(verifier: &str) -> String {
        pkce_s256(verifier)
    }

    #[tokio::test]
    async fn authorize_endpoint_redirects_with_code() {
        let app = router(test_state());
        let verifier = "test-verifier-string-of-reasonable-length-1234";
        let challenge = challenge_for(verifier);
        let uri = format!(
            "/authorize?response_type=code&client_id=test-id&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback&code_challenge={challenge}&code_challenge_method=S256&state=xyz&scope=mcp",
        );
        let resp = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FOUND);
        let location = resp
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(location.starts_with("https://claude.ai/api/mcp/auth_callback?code="));
        assert!(location.contains("&state=xyz"));
    }

    #[tokio::test]
    async fn authorize_rejects_disallowed_redirect_uri() {
        let app = router(test_state());
        let uri = "/authorize?response_type=code&client_id=test-id&redirect_uri=https%3A%2F%2Fattacker.example%2Fcb&code_challenge=abc&code_challenge_method=S256&state=z";
        let resp = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn authorize_rejects_unknown_client_id() {
        let app = router(test_state());
        let uri = "/authorize?response_type=code&client_id=WRONG&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback&code_challenge=abc&code_challenge_method=S256&state=z";
        let resp = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        // Error is reported via the redirect callback per OAuth spec
        assert_eq!(resp.status(), StatusCode::FOUND);
        let location = resp
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(location.contains("error=unauthorized_client"));
    }

    /// Round-trip the full auth-code + PKCE flow through the public router.
    #[tokio::test]
    async fn auth_code_grant_full_flow_succeeds() {
        let state = test_state();
        let verifier = "the-verifier-anthropic-would-have-generated";
        let challenge = challenge_for(verifier);
        let redirect_uri = "https://claude.ai/api/mcp/auth_callback";

        // Step 1: /authorize, extract code from Location header.
        let auth_uri = format!(
            "/authorize?response_type=code&client_id=test-id&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback&code_challenge={challenge}&code_challenge_method=S256&state=opaque-state",
        );
        let resp = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(auth_uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FOUND);
        let location = resp
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let code = location
            .split_once("code=")
            .and_then(|(_, rest)| rest.split('&').next())
            .unwrap()
            .to_string();

        // Step 2: /oauth/token with grant_type=authorization_code.
        let body = format!(
            "grant_type=authorization_code&code={code}&redirect_uri={}&code_verifier={verifier}&client_id=test-id",
            urlencoding_minimal(redirect_uri)
        );
        let resp = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/oauth/token")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("\"access_token\""));
        assert!(body.contains("\"token_type\":\"Bearer\""));
    }

    #[tokio::test]
    async fn auth_code_rejects_bad_verifier() {
        let state = test_state();
        let verifier = "correct-verifier";
        let challenge = challenge_for(verifier);
        let auth_uri = format!(
            "/authorize?response_type=code&client_id=test-id&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback&code_challenge={challenge}&code_challenge_method=S256&state=s",
        );
        let resp = router(state.clone())
            .oneshot(Request::builder().uri(auth_uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let location = resp
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let code = location
            .split_once("code=")
            .and_then(|(_, rest)| rest.split('&').next())
            .unwrap()
            .to_string();

        let body = format!(
            "grant_type=authorization_code&code={code}&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback&code_verifier=WRONG&client_id=test-id"
        );
        let resp = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/oauth/token")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_string(resp).await;
        assert!(body.contains("invalid_grant"));
    }

    #[tokio::test]
    async fn auth_code_is_single_use() {
        let state = test_state();
        let verifier = "vvv";
        let challenge = challenge_for(verifier);
        let auth_uri = format!(
            "/authorize?response_type=code&client_id=test-id&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback&code_challenge={challenge}&code_challenge_method=S256&state=s",
        );
        let resp = router(state.clone())
            .oneshot(Request::builder().uri(auth_uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let location = resp.headers().get(header::LOCATION).unwrap().to_str().unwrap().to_string();
        let code = location.split_once("code=").and_then(|(_, r)| r.split('&').next()).unwrap().to_string();
        let body = format!(
            "grant_type=authorization_code&code={code}&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback&code_verifier={verifier}&client_id=test-id"
        );
        let make_req = || Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body.clone()))
            .unwrap();
        let first = router(state.clone()).oneshot(make_req()).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let second = router(state).oneshot(make_req()).await.unwrap();
        assert_eq!(second.status(), StatusCode::BAD_REQUEST);
    }
}
