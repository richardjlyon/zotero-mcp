use crate::core::error::{Error, Result};
use crate::core::writer::client::LocalApi;
use crate::core::writer::items::update_item_fields;
use reqwest::Method;
use serde_json::{json, Value};

/// Fetch tags/collections/version from the **web** API so the version we use
/// for `If-Unmodified-Since-Version` matches the server we're about to write
/// against. Reading from the local DB risks a 412 if local is behind the
/// cloud (sync hasn't run since the last edit elsewhere).
async fn fetch_item_meta(api: &LocalApi, key: &str) -> Result<(Vec<String>, Vec<String>, i64)> {
    let resp = api
        .write_request(Method::GET, &format!("/items/{key}"))?
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(Error::LocalApi {
            status: resp.status().as_u16(),
            body: resp.text().await.unwrap_or_default(),
        });
    }
    let v: Value = resp.json().await?;
    let data = v.get("data").cloned().unwrap_or_default();
    let version = data.get("version").and_then(|x| x.as_i64()).unwrap_or(0);
    let tags = data.get("tags").and_then(|t| t.as_array()).map(|arr|
        arr.iter().filter_map(|e| e.get("tag").and_then(|s| s.as_str()).map(String::from)).collect()
    ).unwrap_or_default();
    let collections = data.get("collections").and_then(|c| c.as_array()).map(|arr|
        arr.iter().filter_map(|x| x.as_str().map(String::from)).collect()
    ).unwrap_or_default();
    Ok((tags, collections, version))
}

pub async fn add_tags(api: &LocalApi, key: &str, new_tags: &[String]) -> Result<()> {
    let (mut existing, _coll, version) = fetch_item_meta(api, key).await?;
    for t in new_tags {
        if !existing.iter().any(|e| e == t) { existing.push(t.clone()); }
    }
    let json_tags: Vec<Value> = existing.into_iter().map(|t| json!({"tag": t})).collect();
    update_item_fields(api, key, version, json!({ "tags": json_tags })).await
}

pub async fn remove_tags(api: &LocalApi, key: &str, tags_to_remove: &[String]) -> Result<()> {
    let (existing, _coll, version) = fetch_item_meta(api, key).await?;
    let kept: Vec<Value> = existing.into_iter()
        .filter(|t| !tags_to_remove.iter().any(|r| r == t))
        .map(|t| json!({"tag": t})).collect();
    update_item_fields(api, key, version, json!({ "tags": kept })).await
}

pub async fn add_to_collection(api: &LocalApi, key: &str, collection_key: &str) -> Result<()> {
    let (_tags, mut colls, version) = fetch_item_meta(api, key).await?;
    if !colls.iter().any(|c| c == collection_key) { colls.push(collection_key.into()); }
    update_item_fields(api, key, version, json!({ "collections": colls })).await
}

pub async fn remove_from_collection(api: &LocalApi, key: &str, collection_key: &str) -> Result<()> {
    let (_tags, colls, version) = fetch_item_meta(api, key).await?;
    let kept: Vec<String> = colls.into_iter().filter(|c| c != collection_key).collect();
    update_item_fields(api, key, version, json!({ "collections": kept })).await
}
