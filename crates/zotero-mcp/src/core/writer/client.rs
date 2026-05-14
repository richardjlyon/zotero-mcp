use crate::core::error::{Error, Result};
use reqwest::{Client, Method, RequestBuilder};
use serde_json::Value;
use std::time::Duration;

/// Two-faced Zotero HTTP client.
///
/// **Reads** use Zotero's local HTTP server (`http://localhost:23119/api/...`).
/// Cheap, no auth needed, but the local server only implements read methods
/// — `PATCH`/`POST`/`DELETE` return `501 Not Implemented`.
///
/// **Writes** therefore go through the Zotero Web API
/// (`https://api.zotero.org/users/{id}/...`) authenticated with the user's
/// API key. Writes propagate back to the local copy via Zotero's own sync.
///
/// Construct with [`LocalApi::new`], then optionally chain
/// [`LocalApi::with_web_base`] and [`LocalApi::with_api_key`]. When no
/// `api_key` is set, write methods return [`Error::WriteApiKeyMissing`].
#[derive(Clone)]
pub struct LocalApi {
    pub local_base: String,
    pub web_base: String,
    pub user_id: i64,
    pub api_key: Option<String>,
    pub http: Client,
}

impl LocalApi {
    pub fn new(local_base: impl Into<String>, user_id: i64) -> Result<Self> {
        let http = Client::builder().timeout(Duration::from_secs(10)).build()?;
        Ok(Self {
            local_base: local_base.into(),
            web_base: "https://api.zotero.org".into(),
            user_id,
            api_key: None,
            http,
        })
    }

    /// Override the web-API base URL. Useful for tests (point at a mock
    /// server) or self-hosted Zotero proxies.
    pub fn with_web_base(mut self, web_base: impl Into<String>) -> Self {
        self.web_base = web_base.into();
        self
    }

    /// Provide the user's Zotero Web API key.
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        let key = api_key.into();
        self.api_key = if key.is_empty() { None } else { Some(key) };
        self
    }

    /// URL on the **local** server (read endpoints).
    pub fn user_path(&self, suffix: &str) -> String {
        format!("{}/api/users/{}{}", self.local_base, self.user_id, suffix)
    }

    /// URL on the **web** API (write endpoints).
    pub fn web_user_path(&self, suffix: &str) -> String {
        format!("{}/users/{}{}", self.web_base, self.user_id, suffix)
    }

    /// Build a request to the Zotero Web API with Bearer auth and the right
    /// API version header. Returns [`Error::WriteApiKeyMissing`] if no key
    /// has been configured.
    pub fn write_request(&self, method: Method, suffix: &str) -> Result<RequestBuilder> {
        let key = self.api_key.as_ref().ok_or(Error::WriteApiKeyMissing)?;
        Ok(self
            .http
            .request(method, self.web_user_path(suffix))
            .header("Zotero-API-Version", "3")
            .header("Authorization", format!("Bearer {key}")))
    }

    pub async fn list_items_raw(&self, query: &str, start: i64, limit: i64) -> Result<Value> {
        let url = self.user_path("/items");
        let resp = self
            .http
            .get(&url)
            .header("Zotero-API-Version", "3")
            .query(&[
                ("q", query),
                ("start", &start.to_string()),
                ("limit", &limit.to_string()),
            ])
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::LocalApi {
                status: status.as_u16(),
                body,
            });
        }
        Ok(resp.json().await?)
    }
}
