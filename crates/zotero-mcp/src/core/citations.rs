use crate::core::error::{Error, Result};
use crate::core::writer::client::LocalApi;

pub async fn format_citation(api: &LocalApi, item_key: &str, style: &str, format: &str) -> Result<String> {
    let url = api.user_path(&format!("/items/{}", item_key));
    let resp = api.http.get(&url)
        .header("Zotero-API-Version", "3")
        .query(&[("format", format), ("style", style)])
        .send().await?;
    if !resp.status().is_success() {
        return Err(Error::LocalApi { status: resp.status().as_u16(), body: resp.text().await.unwrap_or_default() });
    }
    Ok(resp.text().await?)
}

pub async fn format_bibliography(api: &LocalApi, item_keys: &[String], style: &str, format: &str) -> Result<String> {
    let url = api.user_path("/items");
    let keys = item_keys.join(",");
    let resp = api.http.get(&url)
        .header("Zotero-API-Version", "3")
        .query(&[("itemKey", keys.as_str()), ("format", format), ("style", style)])
        .send().await?;
    if !resp.status().is_success() {
        return Err(Error::LocalApi { status: resp.status().as_u16(), body: resp.text().await.unwrap_or_default() });
    }
    Ok(resp.text().await?)
}
