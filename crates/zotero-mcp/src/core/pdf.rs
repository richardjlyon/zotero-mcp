use crate::core::error::{Error, Result};
use crate::core::reader::pool::ReadOnlyPool;
use crate::core::reader::attachments::resolve_path;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PdfTextSource {
    ZoteroCache,
    LiveExtract,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfTextResult {
    pub text: String,
    pub source: PdfTextSource,
    pub character_count: usize,
}

pub async fn get_pdf_text(
    pool: &ReadOnlyPool,
    parent_key: &str,
    library_id: i64,
    storage_dir: &Path,
) -> Result<PdfTextResult> {
    let pdf_path = resolve_path(pool, parent_key, library_id, storage_dir).await?;
    let storage_item_dir = pdf_path.parent().ok_or_else(|| Error::AttachmentNotFound(parent_key.into()))?.to_path_buf();
    extract(&pdf_path, &storage_item_dir).await
}

async fn extract(pdf_path: &Path, storage_item_dir: &Path) -> Result<PdfTextResult> {
    let cache = storage_item_dir.join(".zotero-ft-cache");
    if cache.exists() {
        let text = tokio::fs::read_to_string(&cache).await?;
        let n = text.chars().count();
        return Ok(PdfTextResult { text, source: PdfTextSource::ZoteroCache, character_count: n });
    }
    let pdf_path = pdf_path.to_path_buf();
    let text = tokio::task::spawn_blocking(move || {
        pdf_extract::extract_text(&pdf_path).map_err(|e| Error::Pdf(e.to_string()))
    }).await.map_err(|e| Error::Pdf(e.to_string()))??;
    let n = text.chars().count();
    Ok(PdfTextResult { text, source: PdfTextSource::LiveExtract, character_count: n })
}

pub async fn get_pdf_first_pages(
    pool: &ReadOnlyPool,
    parent_key: &str,
    library_id: i64,
    storage_dir: &Path,
    n_pages: usize,
) -> Result<PdfTextResult> {
    let full = get_pdf_text(pool, parent_key, library_id, storage_dir).await?;
    // Approximate: take roughly 3500 chars per page from the cache, or use pdf-extract for true pages.
    // First N pages estimate: 3500 chars/page; cap at full length.
    let cap = (n_pages * 3500).min(full.text.len());
    let mut text: String = full.text.chars().take(cap).collect();
    if text.len() < full.text.len() { text.push_str("\n[... truncated ...]"); }
    Ok(PdfTextResult { text, source: full.source, character_count: cap })
}

pub fn cache_path_for(storage_dir: &Path, parent_key: &str) -> PathBuf {
    storage_dir.join(parent_key).join(".zotero-ft-cache")
}
