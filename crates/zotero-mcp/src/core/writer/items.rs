use crate::core::error::{Error, Result};
use crate::core::writer::client::LocalApi;
use reqwest::Method;
use serde_json::{json, Value};

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

/// Create a new Zotero item.
///
/// `item` is a Zotero-shaped JSON object: must have `itemType`; everything
/// else optional and pass-through. `collection_keys` are merged into the
/// item's `collections` field on creation (caller may also set `collections`
/// directly on `item`; both are unioned).
///
/// Returns `(item_key, version)` on success. Errors map to:
/// - `Error::WriteApiKeyMissing` if no api_key configured.
/// - `Error::LocalApi { status, body }` for any 4xx/5xx from Zotero.
pub async fn create_item(
    api: &LocalApi,
    item: &Value,
    collection_keys: &[String],
) -> Result<(String, i64)> {
    // Merge collection_keys into the item (unioned with any existing field).
    let mut item_obj = item
        .as_object()
        .ok_or_else(|| Error::LocalApi {
            status: 0,
            body: "create_item: item must be a JSON object".into(),
        })?
        .clone();

    if !collection_keys.is_empty() {
        let mut existing: Vec<String> = item_obj
            .get("collections")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        for k in collection_keys {
            if !existing.contains(k) {
                existing.push(k.clone());
            }
        }
        item_obj.insert("collections".into(), json!(existing));
    }

    let body = json!([Value::Object(item_obj)]);
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
    let entry = v
        .get("successful")
        .and_then(|s| s.get("0"))
        .ok_or_else(|| Error::LocalApi {
            status: status.as_u16(),
            body: v.to_string(),
        })?;
    let key = entry
        .get("key")
        .and_then(|k| k.as_str())
        .ok_or_else(|| Error::LocalApi {
            status: status.as_u16(),
            body: v.to_string(),
        })?
        .to_string();
    let version = entry
        .get("version")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    Ok((key, version))
}
