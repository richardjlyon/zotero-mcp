use crate::error::{Error, Result};
use crate::writer::client::LocalApi;
use serde_json::Value;

pub async fn update_item_fields(api: &LocalApi, item_key: &str, version: i64, fields: Value) -> Result<()> {
    let url = api.user_path(&format!("/items/{}", item_key));
    let resp = api.http.patch(&url)
        .header("Zotero-API-Version", "3")
        .header("If-Unmodified-Since-Version", version.to_string())
        .json(&fields)
        .send().await?;
    let status = resp.status();
    if status.is_success() { return Ok(()); }
    let body = resp.text().await.unwrap_or_default();
    if status.as_u16() == 412 {
        return Err(Error::VersionConflict(format!("item {} has changed; refresh and retry. body={}", item_key, body)));
    }
    Err(Error::LocalApi { status: status.as_u16(), body })
}
