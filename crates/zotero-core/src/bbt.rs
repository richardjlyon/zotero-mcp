use crate::error::{Error, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone)]
pub struct BbtClient {
    base: String,
    http: reqwest::Client,
}

impl BbtClient {
    pub fn new(base: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()?;
        Ok(Self { base: base.into(), http })
    }

    pub async fn citationkeys(&self, keys: &[String]) -> Result<HashMap<String, String>> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "item.citationkey",
            "params": [keys],
            "id": 1
        });
        let url = format!("{}/better-bibtex/json-rpc", self.base);
        let resp = self.http.post(&url).json(&payload).send().await
            .map_err(|_| Error::BbtUnavailable)?;
        let body: Value = resp.json().await?;
        if let Some(err) = body.get("error") {
            return Err(Error::Bbt(err.to_string()));
        }
        let map: HashMap<String, String> = body.get("result")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(map)
    }
}
