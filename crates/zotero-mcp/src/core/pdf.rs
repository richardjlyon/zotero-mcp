use crate::core::error::{Error, Result};
use crate::core::reader::pool::ReadOnlyPool;
use crate::core::reader::attachments::resolve_path;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Failed(s) => f.write_str(s),
            EngineError::Timeout(secs) => write!(f, "timed out after {}s", secs),
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
            Err(je) if je.is_panic() => Err(EngineError::Failed(format!("pdf-extract panicked: {}", je))),
            Err(je) => Err(EngineError::Failed(format!("pdf-extract task cancelled: {}", je))),
        }
    }
}

/// The two engines wired up for a running server. Construct once at
/// startup with `PdfEngines::build(&cfg.zotero)`; cheap to clone (every
/// field is `Arc`).
#[derive(Clone)]
pub struct PdfEngines {
    primary: Arc<dyn PdfEngine>,
    fallback: FallbackState,
}

#[derive(Clone)]
pub enum FallbackState {
    /// pdftotext is on PATH (or the config override resolved) and the
    /// fallback is enabled.
    Ready(Arc<dyn PdfEngine>),
    /// Fallback is enabled in config but pdftotext was not found.
    BinaryMissing,
    /// User has explicitly disabled the fallback in config.
    Disabled,
}

impl PdfEngines {
    pub fn primary(&self) -> &Arc<dyn PdfEngine> {
        &self.primary
    }

    pub fn fallback(&self) -> &FallbackState {
        &self.fallback
    }

    /// Build the engine bundle from configuration. Resolves `pdftotext`:
    ///
    /// 1. `cfg.pdftotext_path` if set and the file exists.
    /// 2. `which::which("pdftotext")` on PATH.
    /// 3. Otherwise, the fallback is unavailable (`FallbackState::BinaryMissing`).
    ///
    /// Honors `cfg.pdftotext_fallback`: when false, the fallback is
    /// `Disabled` regardless of discovery.
    pub fn build(cfg: &crate::core::config::ZoteroConfig) -> Self {
        let primary: Arc<dyn PdfEngine> = Arc::new(PdfExtractEngine);

        let fallback = if !cfg.pdftotext_fallback {
            tracing::debug!("pdftotext fallback disabled by config");
            FallbackState::Disabled
        } else {
            match resolve_pdftotext(cfg.pdftotext_path.as_deref()) {
                Some(bin) => {
                    tracing::info!(path = %bin.display(), "pdftotext fallback enabled");
                    FallbackState::Ready(Arc::new(PdftotextEngine::new(bin)))
                }
                None => {
                    tracing::info!(
                        "pdftotext not on PATH; PDF extraction has no fallback. \
                         Install Poppler (`brew install poppler` or `apt install poppler-utils`) \
                         for resilient extraction."
                    );
                    FallbackState::BinaryMissing
                }
            }
        };

        Self { primary, fallback }
    }
}

fn resolve_pdftotext(override_path: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = override_path {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
        tracing::warn!(
            path = p,
            "configured pdftotext_path does not exist; falling back to PATH lookup"
        );
    }
    which::which("pdftotext").ok()
}

/// Out-of-process PDF text extraction via Poppler's `pdftotext` binary.
///
/// Used as the fallback when `pdf-extract` fails. Honors a wall-clock
/// timeout and caps captured output at `max_bytes` to bound memory.
pub struct PdftotextEngine {
    binary: PathBuf,
    timeout: Duration,
    max_bytes: usize,
}

impl PdftotextEngine {
    pub fn new(binary: PathBuf) -> Self {
        Self {
            binary,
            timeout: Duration::from_secs(60),
            max_bytes: 50 * 1024 * 1024,
        }
    }

    #[doc(hidden)] // Test-only constructor (public for integration tests in tests/).
    pub fn with_timeout(binary: PathBuf, timeout: Duration) -> Self {
        Self { binary, timeout, max_bytes: 50 * 1024 * 1024 }
    }
}

#[async_trait]
impl PdfEngine for PdftotextEngine {
    async fn extract(&self, path: &Path) -> std::result::Result<String, EngineError> {
        use tokio::io::AsyncReadExt;
        use tokio::process::Command;

        let mut child = Command::new(&self.binary)
            .arg("-enc").arg("UTF-8")
            .arg("-q")
            .arg("--")
            .arg(path)
            .arg("-")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| EngineError::Failed(format!("failed to spawn pdftotext: {}", e)))?;

        let mut stdout_pipe = child.stdout.take()
            .ok_or_else(|| EngineError::Failed("pdftotext stdout missing".into()))?;
        let mut stderr_pipe = child.stderr.take()
            .ok_or_else(|| EngineError::Failed("pdftotext stderr missing".into()))?;

        let max_bytes = self.max_bytes;
        let stdout_task = tokio::spawn(async move {
            let mut buf = Vec::with_capacity(64 * 1024);
            let mut limited = (&mut stdout_pipe).take(max_bytes as u64);
            limited.read_to_end(&mut buf).await.map(|_| buf)
        });
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::with_capacity(1024);
            let mut limited = (&mut stderr_pipe).take(4096);
            limited.read_to_end(&mut buf).await.map(|_| buf)
        });

        let timeout_secs = self.timeout.as_secs();
        let extraction = async move {
            let status = child.wait().await.map_err(|e| format!("pdftotext wait failed: {}", e))?;
            let stdout = stdout_task.await
                .map_err(|e| format!("pdftotext stdout task panicked: {}", e))?
                .map_err(|e| format!("pdftotext stdout read failed: {}", e))?;
            let stderr = stderr_task.await
                .map_err(|e| format!("pdftotext stderr task panicked: {}", e))?
                .map_err(|e| format!("pdftotext stderr read failed: {}", e))?;
            Ok::<_, String>((status, stdout, stderr))
        };

        let (status, stdout, stderr) = match tokio::time::timeout(self.timeout, extraction).await {
            Ok(Ok(t)) => t,
            Ok(Err(msg)) => return Err(EngineError::Failed(msg)),
            Err(_) => return Err(EngineError::Timeout(timeout_secs)),
        };

        if !status.success() {
            let serr = String::from_utf8_lossy(&stderr);
            return Err(EngineError::Failed(format!(
                "pdftotext exited {}: {}",
                status, serr.trim()
            )));
        }

        if stdout.is_empty() {
            // Empty stdout most commonly means pdftotext failed silently; an
            // image-only (scanned) PDF would also extract no text. We surface
            // the same error in both cases — OCR is out of scope.
            return Err(EngineError::Failed("pdftotext produced empty output".into()));
        }

        String::from_utf8(stdout)
            .map_err(|e| EngineError::Failed(format!("pdftotext output not valid UTF-8: {}", e)))
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
