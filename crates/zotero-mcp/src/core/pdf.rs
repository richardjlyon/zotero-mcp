use crate::core::error::{Error, Result};
use crate::core::reader::attachments::resolve_path;
use crate::core::reader::pool::ReadOnlyPool;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PdfTextSource {
    ZoteroCache,
    LiveExtract,
    /// Recovered via Poppler's `pdftotext` after `pdf-extract` failed.
    PdftotextFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
            Err(je) if je.is_panic() => {
                Err(EngineError::Failed(format!("pdf-extract panicked: {}", je)))
            }
            Err(je) => Err(EngineError::Failed(format!(
                "pdf-extract task cancelled: {}",
                je
            ))),
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
        Self {
            binary,
            timeout,
            max_bytes: 50 * 1024 * 1024,
        }
    }
}

#[async_trait]
impl PdfEngine for PdftotextEngine {
    async fn extract(&self, path: &Path) -> std::result::Result<String, EngineError> {
        use tokio::io::AsyncReadExt;
        use tokio::process::Command;

        let mut child = Command::new(&self.binary)
            .arg("-enc")
            .arg("UTF-8")
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

        let mut stdout_pipe = child
            .stdout
            .take()
            .ok_or_else(|| EngineError::Failed("pdftotext stdout missing".into()))?;
        let mut stderr_pipe = child
            .stderr
            .take()
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
            let status = child
                .wait()
                .await
                .map_err(|e| format!("pdftotext wait failed: {}", e))?;
            let stdout = stdout_task
                .await
                .map_err(|e| format!("pdftotext stdout task panicked: {}", e))?
                .map_err(|e| format!("pdftotext stdout read failed: {}", e))?;
            let stderr = stderr_task
                .await
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
                status,
                serr.trim()
            )));
        }

        if stdout.is_empty() {
            // Empty stdout most commonly means pdftotext failed silently; an
            // image-only (scanned) PDF would also extract no text. We surface
            // the same error in both cases — OCR is out of scope.
            return Err(EngineError::Failed(
                "pdftotext produced empty output".into(),
            ));
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
    engines: &PdfEngines,
) -> Result<PdfTextResult> {
    let pdf_path = resolve_path(pool, parent_key, library_id, storage_dir).await?;
    let storage_item_dir = pdf_path
        .parent()
        .ok_or_else(|| Error::AttachmentNotFound(parent_key.into()))?
        .to_path_buf();
    extract(&pdf_path, &storage_item_dir, engines).await
}

pub async fn get_pdf_first_pages(
    pool: &ReadOnlyPool,
    parent_key: &str,
    library_id: i64,
    storage_dir: &Path,
    n_pages: usize,
    engines: &PdfEngines,
) -> Result<PdfTextResult> {
    let full = get_pdf_text(pool, parent_key, library_id, storage_dir, engines).await?;
    let cap = (n_pages * 3500).min(full.text.len());
    let mut text: String = full.text.chars().take(cap).collect();
    if text.len() < full.text.len() {
        text.push_str("\n[... truncated ...]");
    }
    Ok(PdfTextResult {
        text,
        source: full.source,
        character_count: cap,
    })
}

/// Core orchestrator: cache check → primary engine → fallback engine.
async fn extract(
    pdf_path: &Path,
    storage_item_dir: &Path,
    engines: &PdfEngines,
) -> Result<PdfTextResult> {
    // 1. Cache hit.
    let cache = storage_item_dir.join(".zotero-ft-cache");
    if cache.exists() {
        let text = tokio::fs::read_to_string(&cache).await?;
        let n = text.chars().count();
        return Ok(PdfTextResult {
            text,
            source: PdfTextSource::ZoteroCache,
            character_count: n,
        });
    }

    // 2. Primary engine.
    let primary_err = match engines.primary().extract(pdf_path).await {
        Ok(text) => {
            let n = text.chars().count();
            return Ok(PdfTextResult {
                text,
                source: PdfTextSource::LiveExtract,
                character_count: n,
            });
        }
        Err(e) => e.to_string(),
    };

    // 3. Fallback engine.
    let fallback = match engines.fallback() {
        FallbackState::Ready(eng) => eng,
        FallbackState::Disabled => {
            return Err(Error::Pdf(primary_err));
        }
        FallbackState::BinaryMissing => {
            tracing::warn!(
                error = %primary_err,
                path = %pdf_path.display(),
                "pdf-extract failed and pdftotext fallback is unavailable"
            );
            return Err(Error::PdftotextMissing);
        }
    };

    tracing::warn!(
        error = %primary_err,
        path = %pdf_path.display(),
        "pdf-extract failed; trying pdftotext fallback"
    );

    let text = match fallback.extract(pdf_path).await {
        Ok(t) => t,
        Err(EngineError::Timeout(secs)) => {
            return Err(Error::PdftotextTimeout(
                secs,
                pdf_path.display().to_string(),
            ));
        }
        Err(EngineError::Failed(msg)) => {
            return Err(Error::PdfAllEnginesFailed {
                pdf_extract: primary_err,
                pdftotext: msg,
            });
        }
    };

    // 4. Cache write (best-effort).
    if let Err(e) = write_cache_atomic(&cache, &text).await {
        tracing::warn!(
            path = %cache.display(),
            error = %e,
            "failed to write .zotero-ft-cache after pdftotext fallback"
        );
    } else {
        tracing::info!(
            path = %cache.display(),
            "wrote .zotero-ft-cache after pdftotext fallback"
        );
    }

    let n = text.chars().count();
    Ok(PdfTextResult {
        text,
        source: PdfTextSource::PdftotextFallback,
        character_count: n,
    })
}

/// Write the cache via tmp-file + rename so a kill mid-write doesn't leave
/// a partial cache for Zotero to consume.
async fn write_cache_atomic(cache: &Path, text: &str) -> std::io::Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!(".zotero-ft-cache.tmp.{}.{}", std::process::id(), nanos);
    let tmp = cache.with_file_name(tmp_name);

    let mut content = text.to_owned();
    if !content.ends_with('\n') {
        content.push('\n');
    }

    // If the write fails, do our best to remove the stale tmp file — best-effort.
    if let Err(e) = tokio::fs::write(&tmp, content).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }
    tokio::fs::rename(&tmp, cache).await
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

#[cfg(test)]
mod orchestrator_tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// A stub engine that returns a queued sequence of results, one per call.
    struct StubEngine {
        queue: Mutex<Vec<std::result::Result<String, EngineError>>>,
    }

    impl StubEngine {
        fn new(results: Vec<std::result::Result<String, EngineError>>) -> Arc<dyn PdfEngine> {
            Arc::new(Self {
                queue: Mutex::new(results),
            })
        }
        fn ok(text: &str) -> Arc<dyn PdfEngine> {
            Self::new(vec![Ok(text.into())])
        }
        fn fail(msg: &str) -> Arc<dyn PdfEngine> {
            Self::new(vec![Err(EngineError::Failed(msg.into()))])
        }
        fn timeout(secs: u64) -> Arc<dyn PdfEngine> {
            Self::new(vec![Err(EngineError::Timeout(secs))])
        }
        fn never() -> Arc<dyn PdfEngine> {
            // Returns a panic on call so we can assert "not called".
            struct Panicker;
            #[async_trait]
            impl PdfEngine for Panicker {
                async fn extract(&self, _: &Path) -> std::result::Result<String, EngineError> {
                    panic!("engine should not have been called");
                }
            }
            Arc::new(Panicker)
        }
    }

    #[async_trait]
    impl PdfEngine for StubEngine {
        async fn extract(&self, _: &Path) -> std::result::Result<String, EngineError> {
            self.queue.lock().unwrap().remove(0)
        }
    }

    fn write_dummy_pdf(dir: &Path) -> PathBuf {
        let p = dir.join("dummy.pdf");
        std::fs::write(&p, b"%PDF-1.4\n%dummy\n").unwrap();
        p
    }

    fn engines_with(primary: Arc<dyn PdfEngine>, fallback: FallbackState) -> PdfEngines {
        PdfEngines { primary, fallback }
    }

    #[tokio::test]
    async fn cache_hit_short_circuits_both_engines() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".zotero-ft-cache"), "cached body\n").unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let engines = engines_with(
            StubEngine::never(),
            FallbackState::Ready(StubEngine::never()),
        );
        let r = extract(&pdf, dir.path(), &engines).await.unwrap();

        assert_eq!(r.source, PdfTextSource::ZoteroCache);
        assert!(r.text.contains("cached body"));
    }

    #[tokio::test]
    async fn primary_success_does_not_write_cache() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let engines = engines_with(
            StubEngine::ok("primary text"),
            FallbackState::Ready(StubEngine::never()),
        );
        let r = extract(&pdf, dir.path(), &engines).await.unwrap();

        assert_eq!(r.source, PdfTextSource::LiveExtract);
        assert_eq!(r.text, "primary text");
        assert!(
            !dir.path().join(".zotero-ft-cache").exists(),
            "cache must not be written on primary success"
        );
    }

    #[tokio::test]
    async fn fallback_success_writes_cache() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let engines = engines_with(
            StubEngine::fail("unhandled function type 4"),
            FallbackState::Ready(StubEngine::ok("recovered text")),
        );
        let r = extract(&pdf, dir.path(), &engines).await.unwrap();

        assert_eq!(r.source, PdfTextSource::PdftotextFallback);
        assert_eq!(r.text, "recovered text");
        let cached = std::fs::read_to_string(dir.path().join(".zotero-ft-cache")).unwrap();
        assert!(cached.contains("recovered text"));
        assert!(cached.ends_with('\n'));
    }

    #[tokio::test]
    async fn both_engines_failed_returns_composite_error() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let engines = engines_with(
            StubEngine::fail("primary boom"),
            FallbackState::Ready(StubEngine::fail("pdftotext boom")),
        );
        let err = extract(&pdf, dir.path(), &engines).await.unwrap_err();

        match err {
            Error::PdfAllEnginesFailed {
                pdf_extract,
                pdftotext,
            } => {
                assert_eq!(pdf_extract, "primary boom");
                assert_eq!(pdftotext, "pdftotext boom");
            }
            other => panic!("unexpected error variant: {:?}", other),
        }
    }

    #[tokio::test]
    async fn fallback_binary_missing_returns_pdftotext_missing() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let engines = engines_with(StubEngine::fail("primary"), FallbackState::BinaryMissing);
        let err = extract(&pdf, dir.path(), &engines).await.unwrap_err();
        assert!(matches!(err, Error::PdftotextMissing));
    }

    #[tokio::test]
    async fn fallback_disabled_returns_legacy_pdf_error() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let engines = engines_with(StubEngine::fail("primary"), FallbackState::Disabled);
        let err = extract(&pdf, dir.path(), &engines).await.unwrap_err();
        match err {
            Error::Pdf(msg) => assert_eq!(msg, "primary"),
            other => panic!("expected Error::Pdf, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn fallback_timeout_maps_to_pdftotext_timeout_variant() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let engines = engines_with(
            StubEngine::fail("primary"),
            FallbackState::Ready(StubEngine::timeout(42)),
        );
        let err = extract(&pdf, dir.path(), &engines).await.unwrap_err();
        match err {
            Error::PdftotextTimeout(secs, _) => assert_eq!(secs, 42),
            other => panic!("expected PdftotextTimeout, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn cache_write_failure_does_not_propagate() {
        // Use a directory path that doesn't exist so the rename inside
        // write_cache_atomic fails — text should still be returned.
        let dir = TempDir::new().unwrap();
        let nonexistent = dir.path().join("missing_subdir");
        let pdf = write_dummy_pdf(dir.path());

        let engines = engines_with(
            StubEngine::fail("primary"),
            FallbackState::Ready(StubEngine::ok("rescued")),
        );
        let r = extract(&pdf, &nonexistent, &engines).await.unwrap();
        assert_eq!(r.source, PdfTextSource::PdftotextFallback);
        assert_eq!(r.text, "rescued");
        assert!(!nonexistent.join(".zotero-ft-cache").exists());
    }
}
