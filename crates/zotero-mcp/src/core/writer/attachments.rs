//! Attachment-creation primitives.
//!
//! - [`attach_link`]: single POST that creates a `linked_url` child attachment
//!   (URL only, no bytes).
//! - [`attach_file`]: file-on-disk attachment, supporting both `imported_file`
//!   (3-step upload to Zotero's cloud) and `linked_file` (path reference only).

use crate::core::error::{Error, Result};
use crate::core::writer::client::LocalApi;
use reqwest::Method;
use serde_json::{json, Value};

/// Attach a URL as a `linked_url` child to an existing parent item.
///
/// One POST; no bytes transfer. Returns the new attachment item key.
pub async fn attach_link(
    api: &LocalApi,
    parent_key: &str,
    url: &str,
    title: Option<&str>,
) -> Result<String> {
    let title = title.unwrap_or(url);
    let body = json!([{
        "itemType": "attachment",
        "parentItem": parent_key,
        "linkMode": "linked_url",
        "url": url,
        "title": title,
        "tags": [],
        "relations": {}
    }]);
    let resp = api
        .write_request(Method::POST, "/items")?
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(Error::LocalApi {
            status: status.as_u16(),
            body: body_text,
        });
    }
    let v: Value = resp.json().await?;
    v.get("successful")
        .and_then(|s| s.get("0"))
        .and_then(|i| i.get("key"))
        .and_then(|k| k.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::LocalApi {
            status: status.as_u16(),
            body: v.to_string(),
        })
}
