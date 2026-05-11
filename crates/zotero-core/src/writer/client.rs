use crate::error::{Error, Result};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone)]
pub struct LocalApi {
    pub base: String,
    pub user_id: i64,
    pub http: Client,
}

impl LocalApi {
    pub fn new(base: impl Into<String>, user_id: i64) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?;
        Ok(Self { base: base.into(), user_id, http })
    }

    pub fn user_path(&self, suffix: &str) -> String {
        format!("{}/api/users/{}{}", self.base, self.user_id, suffix)
    }

    pub async fn list_items_raw(&self, query: &str, start: i64, limit: i64) -> Result<Value> {
        let url = self.user_path("/items");
        let resp = self.http.get(&url)
            .header("Zotero-API-Version", "3")
            .query(&[("q", query), ("start", &start.to_string()), ("limit", &limit.to_string())])
            .send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::LocalApi { status: status.as_u16(), body });
        }
        Ok(resp.json().await?)
    }
}
