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
