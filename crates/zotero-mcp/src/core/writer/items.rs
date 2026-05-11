use crate::core::error::{Error, Result};
use crate::core::writer::client::LocalApi;
use reqwest::Method;
use serde_json::Value;

pub async fn update_item_fields(
    api: &LocalApi,
    item_key: &str,
    version: i64,
    fields: Value,
) -> Result<()> {
    let resp = api
        .write_request(Method::PATCH, &format!("/items/{item_key}"))?
        .header("If-Unmodified-Since-Version", version.to_string())
        .json(&fields)
        .send()
        .await?;
    handle_write_response(item_key, resp).await
}

/// Move an item (or note attachment) to Zotero's trash via DELETE.
/// Recoverable — trashed items remain in the user's library and can be
/// restored from the Trash collection in Zotero's UI until they're
/// permanently emptied.
pub async fn delete_item(api: &LocalApi, item_key: &str, version: i64) -> Result<()> {
    let resp = api
        .write_request(Method::DELETE, &format!("/items/{item_key}"))?
        .header("If-Unmodified-Since-Version", version.to_string())
        .send()
        .await?;
    handle_write_response(item_key, resp).await
}

async fn handle_write_response(item_key: &str, resp: reqwest::Response) -> Result<()> {
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let body = resp.text().await.unwrap_or_default();
    if status.as_u16() == 412 {
        return Err(Error::VersionConflict(format!(
            "item {item_key} has changed; refresh and retry. body={body}"
        )));
    }
    Err(Error::LocalApi { status: status.as_u16(), body })
}
