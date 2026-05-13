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
