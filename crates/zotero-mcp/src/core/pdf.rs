use crate::core::error::{Error, Result};
use crate::core::reader::pool::ReadOnlyPool;
use crate::core::reader::attachments::resolve_path;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfTextSource {
    ZoteroCache,
    LiveExtract,
    /// Recovered via Poppler's `pdftotext` after `pdf-extract` failed.
    PdftotextFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfTextResult {
    pub text: String,
    pub source: PdfTextSource,
    pub character_count: usize,
}

/// A PDF text extraction engine. Implementors are stateless and reusable.
#[async_trait]
pub trait PdfEngine: Send + Sync {
    /// Extract plain UTF-8 text from the PDF at `path`. Returns
    /// `Err(EngineError)` on failure; the orchestrator decides how to
    /// surface it to the caller.
    async fn extract(&self, path: &Path) -> std::result::Result<String, EngineError>;
}

/// Failure modes that an engine can report. The orchestrator maps these
/// to user-facing `Error` variants.
#[derive(Debug, Clone)]
pub enum EngineError {
    /// Generic failure; carries a display-formatted reason.
    Failed(String),
    /// The engine exceeded its configured timeout. `u64` is the timeout
    /// in seconds (only `PdftotextEngine` produces this).
    Timeout(u64),
}

impl EngineError {
    pub fn display(&self) -> String {
        match self {
            EngineError::Failed(s) => s.clone(),
            EngineError::Timeout(secs) => format!("timed out after {}s", secs),
        }
    }
}

/// In-process PDF text extraction via the `pdf-extract` crate. This is the
/// primary engine; failures are recoverable by the `pdftotext` fallback.
///
/// `pdf-extract` is known to panic on PDFs that use uncommon features
/// (e.g. PostScript Calculator (Type 4) functions). The orchestrator runs
/// this engine inside `tokio::task::spawn_blocking` so panics are caught
/// at the task boundary and returned as `EngineError::Failed`.
pub struct PdfExtractEngine;

#[async_trait]
impl PdfEngine for PdfExtractEngine {
    async fn extract(&self, path: &Path) -> std::result::Result<String, EngineError> {
        let path = path.to_path_buf();
        let join = tokio::task::spawn_blocking(move || {
            pdf_extract::extract_text(&path).map_err(|e| e.to_string())
        })
        .await;

        match join {
            Ok(Ok(text)) => Ok(text),
            Ok(Err(msg)) => Err(EngineError::Failed(msg)),
            // A panic inside the blocking task surfaces as a JoinError.
            Err(je) => Err(EngineError::Failed(format!("pdf-extract panicked: {}", je))),
        }
    }
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

#[cfg(test)]
mod engine_tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn pdf_extract_engine_returns_failed_for_non_pdf() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"this is not a pdf").unwrap();
        let path = f.path().to_path_buf();

        let eng = PdfExtractEngine;
        let res = eng.extract(&path).await;
        assert!(matches!(res, Err(EngineError::Failed(_))));
    }
}
