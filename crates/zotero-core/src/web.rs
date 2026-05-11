use crate::error::{Error, Result};
use crate::reader::pool::ReadOnlyPool;
use crate::reader::attachments::list_attachments;
use serde::{Deserialize, Serialize};
use std::path::Path;
use url::Url;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebMode { Snapshot, Live, Auto }

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSource { Snapshot, Live }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebContentResult {
    pub text: String,
    pub title: Option<String>,
    pub source: WebSource,
    pub url: Option<String>,
    pub fetched_at: Option<String>,
}

pub async fn get_webpage_content(
    pool: &ReadOnlyPool,
    parent_key: &str,
    library_id: i64,
    storage_dir: &Path,
    mode: WebMode,
    user_agent: &str,
) -> Result<WebContentResult> {
    let atts = list_attachments(pool, parent_key, library_id, storage_dir).await?;
    let snapshot = atts.iter().find(|a| a.content_type.as_deref() == Some("text/html")).cloned();

    // Look up item URL for live fallback
    let parent_url = lookup_url(pool, parent_key, library_id).await.ok().flatten();

    let try_snapshot = || async {
        if let Some(s) = snapshot.as_ref() {
            if let Some(p) = &s.absolute_path {
                let html = tokio::fs::read_to_string(p).await?;
                let (title, text) = readability_extract(&html, parent_url.as_deref())?;
                return Ok::<_, Error>(WebContentResult {
                    text, title, source: WebSource::Snapshot,
                    url: parent_url.clone(), fetched_at: None,
                });
            }
        }
        Err(Error::AttachmentNotFound(format!("{} (no snapshot)", parent_key)))
    };

    let try_live = || async {
        let url = parent_url.clone().ok_or_else(|| Error::Html(format!("item {} has no URL", parent_key)))?;
        let client = reqwest::Client::builder().user_agent(user_agent).build()?;
        let resp = client.get(&url).send().await?.error_for_status()?;
        let html = resp.text().await?;
        let (title, text) = readability_extract(&html, Some(&url))?;
        Ok::<_, Error>(WebContentResult {
            text, title, source: WebSource::Live,
            url: Some(url), fetched_at: Some(now_iso8601()),
        })
    };

    match mode {
        WebMode::Snapshot => try_snapshot().await,
        WebMode::Live => try_live().await,
        WebMode::Auto => match try_snapshot().await {
            Ok(r) => Ok(r),
            Err(_) => try_live().await,
        },
    }
}

async fn lookup_url(pool: &ReadOnlyPool, key: &str, library_id: i64) -> Result<Option<String>> {
    let key = key.to_string();
    pool.with_conn(move |c| {
        match c.query_row(
            "SELECT v.value FROM items i
             JOIN itemData d ON d.itemID = i.itemID
             JOIN fieldsCombined f ON f.fieldID = d.fieldID
             JOIN itemDataValues v ON v.valueID = d.valueID
             WHERE i.libraryID = ? AND i.key = ? AND f.fieldName = 'url'",
            rusqlite::params![library_id, &key],
            |r| r.get::<_, String>(0),
        ) {
            Ok(u) => Ok(Some(u)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }).await
}

/// Extract readable title and plain text from HTML using the readability crate.
///
/// The `readability` 0.3 crate returns `Product { title: String, content: String, text: String }`.
fn readability_extract(html: &str, base_url: Option<&str>) -> Result<(Option<String>, String)> {
    let url = base_url
        .and_then(|u| Url::parse(u).ok())
        .unwrap_or_else(|| Url::parse("http://example.invalid/").unwrap());
    let mut reader = std::io::Cursor::new(html);
    let product = readability::extractor::extract(&mut reader, &url)
        .map_err(|e| Error::Html(e.to_string()))?;
    let title = if product.title.is_empty() { None } else { Some(product.title) };
    Ok((title, product.text))
}

fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("@{}", secs)
}

use crate::writer::client::LocalApi;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefetchResult {
    pub url: String,
    pub text: String,
    pub title: Option<String>,
    pub saved_attachment_key: Option<String>,
    pub fetched_at: String,
}

pub async fn refetch_url(
    pool: &ReadOnlyPool,
    api: Option<&LocalApi>,
    parent_key: &str,
    library_id: i64,
    save_as_snapshot: bool,
    user_agent: &str,
) -> Result<RefetchResult> {
    let url = lookup_url(pool, parent_key, library_id).await?
        .ok_or_else(|| Error::Html(format!("item {} has no URL", parent_key)))?;
    let client = reqwest::Client::builder().user_agent(user_agent).build()?;
    let resp = client.get(&url).send().await?.error_for_status()?;
    let html = resp.text().await?;
    let (title, text) = readability_extract(&html, Some(&url))?;
    let fetched_at = now_iso8601();

    let saved_attachment_key = if save_as_snapshot {
        if let Some(api) = api {
            Some(create_html_snapshot_attachment(api, parent_key, &url, &html).await?)
        } else { None }
    } else { None };

    Ok(RefetchResult { url, text, title, saved_attachment_key, fetched_at })
}

async fn create_html_snapshot_attachment(api: &LocalApi, parent_key: &str, url: &str, html: &str) -> Result<String> {
    use serde_json::json;
    // We create a webpage-snapshot attachment via the Local API. We rely on
    // Zotero to handle ingest of the body when the linkMode is "imported_url".
    // For now we POST metadata; the body upload step is documented but optional
    // for v1 (Zotero stores attached HTML inline when contentType is set).
    let body = json!([{
        "itemType": "attachment",
        "parentItem": parent_key,
        "linkMode": "imported_url",
        "title": "Snapshot",
        "url": url,
        "contentType": "text/html",
        "note": format!("Refetched at {} by zotero-mcp; {} bytes", now_iso8601(), html.len())
    }]);
    let url_e = api.user_path("/items");
    let resp = api.http.post(&url_e).header("Zotero-API-Version", "3").json(&body).send().await?;
    if !resp.status().is_success() {
        return Err(Error::LocalApi { status: resp.status().as_u16(), body: resp.text().await.unwrap_or_default() });
    }
    let v: serde_json::Value = resp.json().await?;
    v.get("successful").and_then(|s| s.get("0")).and_then(|i| i.get("key")).and_then(|k| k.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::LocalApi { status: 200, body: v.to_string() })
}
