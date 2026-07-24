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
    /// Layout-aware markdown via the Docling service.
    Docling,
    /// OCR pre-step (`ocrmypdf`) followed by Docling extraction.
    OcrThenDocling,
}

/// Output format of an extraction result. Markdown is produced by the
/// layout-aware (Docling) route; the flat-text chain produces plain text.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PdfFormat {
    Markdown,
    Plain,
}

/// Placeholder Docling emits (post-assembly) where a formula region was
/// detected but not decoded to LaTeX.
pub const FORMULA_NOT_DECODED_MARKER: &str = "<!-- formula-not-decoded -->";
/// Placeholder Docling emits where a figure/chart image was detected but
/// not transcribed.
pub const IMAGE_MARKER: &str = "<!-- image -->";
/// Pages whose non-marker character count falls below this floor are
/// flagged as possible content drops (`low_text_pages`).
pub const LOW_TEXT_CHAR_FLOOR: usize = 100;
/// Sentinel passed to Docling as `md_page_break_placeholder`; replaced
/// with `--- p.N ---` anchors during assembly.
pub const DOCLING_PAGE_BREAK_SENTINEL: &str = "<!-- docling-page-break -->";

/// Minimum non-whitespace characters (markers and page anchors excluded)
/// an extraction must yield to count as having extracted anything. Any
/// route whose output falls below this floor is treated as "did not
/// extract" and the orchestrator continues to the next route; if every
/// route is sub-floor the result is a loud `PdfNothingExtractable` error
/// naming the OCR remedy — never empty text as success.
///
/// 10 is deliberately tiny: the shortest real content (a title line)
/// clears it easily, while the junk that empty routes emit (a stray page
/// number, form feeds, whitespace) stays under it. A false "sub-floor"
/// only costs trying the next route; a false "extracted" would violate
/// the presence-is-trustworthy invariant, so the bar stays low but
/// non-trivial. It is the same judgement ("no usable text") the OCR
/// probe applies, so both share this constant.
pub const MIN_EXTRACTED_CHARS: usize = 10;

/// True when `text` clears [`MIN_EXTRACTED_CHARS`], not counting
/// whitespace, `--- p.N ---` page anchors, or the Docling
/// formula/image placeholders.
fn meets_text_floor(text: &str) -> bool {
    let stripped = text
        .replace(FORMULA_NOT_DECODED_MARKER, " ")
        .replace(IMAGE_MARKER, " ");
    let n: usize = stripped
        .lines()
        .filter(|l| parse_page_anchor(l).is_none())
        .map(|l| l.chars().filter(|c| !c.is_whitespace()).count())
        .sum();
    n >= MIN_EXTRACTED_CHARS
}

/// Machine-readable completeness report attached to every extraction
/// result. Downstream may trust *presence* in the text; absence on a page
/// listed in a drop vector must be treated as "unknown", never as "not in
/// the document".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Completeness {
    /// True only when a layout route ran with formula enrichment and left
    /// zero undecoded formulas and zero untranscribed images.
    pub complete: bool,
    /// The engine that produced the text this report describes.
    pub engine: PdfTextSource,
    /// The document's true total page count, independent of the returned
    /// window. 0 when it could not be determined. A caller walks a large
    /// document by requesting windows until this many pages are covered.
    #[serde(default)]
    pub total_pages: u32,
    /// Number of pages seen in the page-anchored output (0 when the engine
    /// cannot report pages, i.e. the flat-text chain). Describes only the
    /// returned window, which may be a subset of `total_pages`.
    pub pages: u32,
    /// Character count per page (markers excluded), in page order.
    pub per_page_chars: Vec<usize>,
    /// One entry per undecoded-formula marker: the page it appears on.
    pub undecoded_formulas: Vec<u32>,
    /// One entry per untranscribed image/chart marker: the page it appears on.
    pub untranscribed_images: Vec<u32>,
    /// Pages recovered by the OCR pre-step.
    pub ocr_pages: Vec<u32>,
    /// Pages under `LOW_TEXT_CHAR_FLOOR` characters — possible drops.
    pub low_text_pages: Vec<u32>,
    /// Human-readable caveats (e.g. the flat-text warning).
    pub notes: Vec<String>,
}

impl Completeness {
    /// Report for the flat-text engines (`.zotero-ft-cache`, `pdf-extract`,
    /// `pdftotext`). They cannot detect structure, so they are never
    /// complete and their absence must never read as authoritative.
    pub fn flat_text(engine: PdfTextSource) -> Self {
        Self {
            complete: false,
            engine,
            total_pages: 0,
            pages: 0,
            per_page_chars: Vec::new(),
            undecoded_formulas: Vec::new(),
            untranscribed_images: Vec::new(),
            ocr_pages: Vec::new(),
            low_text_pages: Vec::new(),
            notes: vec!["flat-text engine cannot detect tables/formulas/images".into()],
        }
    }

    /// Derive the report from page-anchored markdown (`--- p.N ---` lines,
    /// as assembled from the Docling page-break sentinel). Pure — no I/O.
    ///
    /// Markdown without any anchors is treated as a single page.
    /// `complete` is true only when `engine` is a layout route
    /// (`Docling` / `OcrThenDocling`), `formula_enrichment` was on, and no
    /// undecoded-formula or untranscribed-image markers remain.
    pub fn from_page_anchored_markdown(
        markdown: &str,
        engine: PdfTextSource,
        formula_enrichment: bool,
        ocr_pages: Vec<u32>,
    ) -> Self {
        // Split into (page_number, content) in anchor order.
        let mut pages: Vec<(u32, String)> = Vec::new();
        for line in markdown.lines() {
            if let Some(n) = parse_page_anchor(line) {
                pages.push((n, String::new()));
            } else if let Some((_, content)) = pages.last_mut() {
                content.push_str(line);
                content.push('\n');
            }
        }
        if pages.is_empty() {
            pages.push((1, markdown.to_owned()));
        }

        let mut per_page_chars = Vec::with_capacity(pages.len());
        let mut undecoded_formulas = Vec::new();
        let mut untranscribed_images = Vec::new();
        let mut low_text_pages = Vec::new();

        for (page, content) in &pages {
            for _ in 0..content.matches(FORMULA_NOT_DECODED_MARKER).count() {
                undecoded_formulas.push(*page);
            }
            for _ in 0..content.matches(IMAGE_MARKER).count() {
                untranscribed_images.push(*page);
            }
            let stripped = content
                .replace(FORMULA_NOT_DECODED_MARKER, "")
                .replace(IMAGE_MARKER, "");
            let chars = stripped.trim().chars().count();
            if chars < LOW_TEXT_CHAR_FLOOR {
                low_text_pages.push(*page);
            }
            per_page_chars.push(chars);
        }

        let layout_route = matches!(
            engine,
            PdfTextSource::Docling | PdfTextSource::OcrThenDocling
        );
        let mut notes = Vec::new();
        if !layout_route {
            notes.push("flat-text engine cannot detect tables/formulas/images".into());
        }
        if !formula_enrichment {
            notes.push("formula enrichment was not enabled; formulas may be undecoded".into());
        }

        let complete = layout_route
            && formula_enrichment
            && undecoded_formulas.is_empty()
            && untranscribed_images.is_empty();

        Self {
            complete,
            engine,
            total_pages: 0,
            pages: pages.len() as u32,
            per_page_chars,
            undecoded_formulas,
            untranscribed_images,
            ocr_pages,
            low_text_pages,
            notes,
        }
    }
}

/// Parse a `--- p.N ---` page-anchor line; returns `N` on match.
fn parse_page_anchor(line: &str) -> Option<u32> {
    let rest = line.trim().strip_prefix("--- p.")?;
    let num = rest.strip_suffix(" ---")?;
    num.parse().ok()
}

/// Replace the Docling page-break sentinel with `--- p.N ---` anchors,
/// numbering pages from `start_page`. For a whole-document convert
/// `start_page` is 1, so output starts with `--- p.1 ---`; for a windowed
/// convert (the PDF was sliced to pages `start_page..`) the anchors carry
/// the document's *true* page numbers, e.g. a window beginning at page 5
/// starts with `--- p.5 ---`. Pure — no I/O.
fn assemble_page_anchors(markdown: &str, sentinel: &str, start_page: u32) -> String {
    let mut out = String::with_capacity(markdown.len() + 64);
    for (i, page) in markdown.split(sentinel).enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        out.push_str(&format!("--- p.{} ---\n\n", start_page + i as u32));
        out.push_str(page.trim());
    }
    out.push('\n');
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PdfTextResult {
    pub text: String,
    pub source: PdfTextSource,
    pub character_count: usize,
    pub format: PdfFormat,
    pub page_anchors: bool,
    pub completeness: Completeness,
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
    /// Layout-aware primary route; `None` when no Docling endpoint is
    /// configured (neither `DOCLING_URL` nor `docling_url` in config).
    docling: Option<Arc<DoclingEngine>>,
    /// `ocrmypdf` binary for the OCR pre-step on image-only (scanned)
    /// PDFs; `None` degrades gracefully (OCR skipped, gap recorded in the
    /// completeness report).
    ocrmypdf: Option<PathBuf>,
    primary: Arc<dyn PdfEngine>,
    fallback: FallbackState,
    /// Page ceiling for a whole-document (un-windowed) extraction; above it
    /// the orchestrator refuses with `PdfDocumentTooLarge` and directs the
    /// caller to page windows. From `cfg.pdf_whole_document_max_pages`.
    whole_document_max_pages: u32,
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
    pub fn docling(&self) -> Option<&Arc<DoclingEngine>> {
        self.docling.as_ref()
    }

    pub fn ocrmypdf(&self) -> Option<&PathBuf> {
        self.ocrmypdf.as_ref()
    }

    pub fn primary(&self) -> &Arc<dyn PdfEngine> {
        &self.primary
    }

    pub fn fallback(&self) -> &FallbackState {
        &self.fallback
    }

    pub fn whole_document_max_pages(&self) -> u32 {
        self.whole_document_max_pages
    }

    /// Replace the Docling route on an already-built bundle. Lets
    /// integration tests point the orchestrator at a specific endpoint
    /// (e.g. a dead port to exercise the flat-text fallback) regardless
    /// of `DOCLING_URL` / config; `None` disables the route.
    pub fn with_docling(mut self, docling: Option<Arc<DoclingEngine>>) -> Self {
        self.docling = docling;
        self
    }

    /// Build the engine bundle from configuration.
    ///
    /// Resolves the Docling endpoint (the layout-aware primary route):
    ///
    /// 1. The `DOCLING_URL` environment variable, when set and non-empty.
    /// 2. `cfg.docling_url` from config.
    /// 3. Otherwise, the Docling route is disabled.
    ///
    /// Resolves `ocrmypdf` (the OCR pre-step for scanned PDFs) the same
    /// way as `pdftotext` below: config override first, then PATH lookup;
    /// unresolvable means the pre-step is skipped gracefully.
    ///
    /// Resolves `pdftotext`:
    ///
    /// 1. `cfg.pdftotext_path` if set and the file exists.
    /// 2. `which::which("pdftotext")` on PATH.
    /// 3. Otherwise, the fallback is unavailable (`FallbackState::BinaryMissing`).
    ///
    /// Honors `cfg.pdftotext_fallback`: when false, the fallback is
    /// `Disabled` regardless of discovery.
    pub fn build(cfg: &crate::core::config::ZoteroConfig) -> Self {
        let docling_url = std::env::var("DOCLING_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| cfg.docling_url.clone());
        let docling = docling_url.map(|url| {
            tracing::info!(url = %url, "docling layout-aware extraction route enabled");
            Arc::new(DoclingEngine::new(
                url,
                Duration::from_secs(cfg.docling_convert_timeout_secs),
                Duration::from_secs(cfg.docling_health_timeout_secs),
            ))
        });

        let ocrmypdf = resolve_ocrmypdf(cfg.ocrmypdf_path.as_deref());
        match &ocrmypdf {
            Some(bin) => {
                tracing::info!(path = %bin.display(), "ocrmypdf OCR pre-step enabled");
            }
            None => {
                tracing::info!(
                    "ocrmypdf not on PATH; scanned (image-only) PDFs will not be OCR'd. \
                     Install ocrmypdf for the OCR pre-step."
                );
            }
        }

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

        Self {
            docling,
            ocrmypdf,
            primary,
            fallback,
            whole_document_max_pages: cfg.pdf_whole_document_max_pages,
        }
    }
}

fn resolve_ocrmypdf(override_path: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = override_path {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
        tracing::warn!(
            path = p,
            "configured ocrmypdf_path does not exist; falling back to PATH lookup"
        );
    }
    which::which("ocrmypdf").ok()
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

/// Layout-aware PDF-to-markdown extraction via a `docling-serve` instance.
///
/// The primary route: real tables, reading order, `--- p.N ---` page
/// anchors (assembled from the page-break sentinel), and formula
/// enrichment so equations decode to LaTeX instead of dropping. Not part
/// of the flat-text `PdfEngine` chain — the orchestrator health-checks it
/// and falls through to that chain on any failure.
pub struct DoclingEngine {
    base_url: String,
    client: reqwest::Client,
    convert_timeout: Duration,
    health_timeout: Duration,
}

/// A successful Docling conversion: page-anchored markdown plus whether
/// formula enrichment was applied (it is retried without enrichment when
/// the service cannot run the enrichment model).
pub struct DoclingExtraction {
    pub markdown: String,
    pub formula_enrichment: bool,
}

/// Shape of the docling-serve `/v1/convert/file` response (the fields we
/// consume).
#[derive(Deserialize)]
struct DoclingConvertResponse {
    #[serde(default)]
    document: Option<DoclingDocument>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    errors: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct DoclingDocument {
    #[serde(default)]
    md_content: Option<String>,
}

impl DoclingEngine {
    pub fn new(base_url: String, convert_timeout: Duration, health_timeout: Duration) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
            convert_timeout,
            health_timeout,
        }
    }

    /// Short probe of `GET {base_url}/health`. False on any error or
    /// non-2xx status; the orchestrator then skips the Docling route.
    pub async fn healthy(&self) -> bool {
        let url = format!("{}/health", self.base_url);
        match self
            .client
            .get(&url)
            .timeout(self.health_timeout)
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "docling health check failed");
                false
            }
        }
    }

    /// Convert the PDF at `path` to page-anchored markdown
    /// (`--- p.N ---` anchors).
    ///
    /// Tries with formula enrichment first (equations decode to LaTeX).
    /// Some docling-serve deployments cannot run the enrichment model —
    /// the convert then fails outright — so on failure the convert is
    /// retried once without enrichment and the returned
    /// `formula_enrichment: false` makes the completeness report declare
    /// the gap (`complete: false` + enrichment note) rather than lose the
    /// document entirely.
    /// `start_page` is the document page number the first converted page
    /// corresponds to (1 for a whole-document convert; the window start when
    /// `path` is a locally-sliced window), used to number the `--- p.N ---`
    /// anchors with true document page numbers.
    pub async fn extract_markdown(
        &self,
        path: &Path,
        start_page: u32,
    ) -> std::result::Result<DoclingExtraction, EngineError> {
        let bytes = tokio::fs::read(path).await.map_err(|e| {
            EngineError::Failed(format!("docling: failed to read {}: {}", path.display(), e))
        })?;
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "document.pdf".into());

        match self.convert(&bytes, &file_name, true, start_page).await {
            Ok(markdown) => Ok(DoclingExtraction {
                markdown,
                formula_enrichment: true,
            }),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "docling convert with formula enrichment failed; retrying without enrichment"
                );
                let markdown = self.convert(&bytes, &file_name, false, start_page).await?;
                Ok(DoclingExtraction {
                    markdown,
                    formula_enrichment: false,
                })
            }
        }
    }

    /// One `POST /v1/convert/file` round-trip. Treats a non-2xx response,
    /// `status != "success"`, non-empty `errors`, or empty markdown as
    /// failure.
    async fn convert(
        &self,
        bytes: &[u8],
        file_name: &str,
        formula_enrichment: bool,
        start_page: u32,
    ) -> std::result::Result<String, EngineError> {
        let part = reqwest::multipart::Part::bytes(bytes.to_vec())
            .file_name(file_name.to_string())
            .mime_str("application/pdf")
            .map_err(|e| EngineError::Failed(format!("docling: invalid mime type: {}", e)))?;
        let form = reqwest::multipart::Form::new()
            .text("to_formats", "md")
            .text(
                "do_formula_enrichment",
                if formula_enrichment { "true" } else { "false" },
            )
            .text("md_page_break_placeholder", DOCLING_PAGE_BREAK_SENTINEL)
            .part("files", part);

        let timeout_secs = self.convert_timeout.as_secs();
        let resp = self
            .client
            .post(format!("{}/v1/convert/file", self.base_url))
            .multipart(form)
            .timeout(self.convert_timeout)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    EngineError::Timeout(timeout_secs)
                } else {
                    EngineError::Failed(format!("docling convert request failed: {}", e))
                }
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let snippet: String = body.chars().take(300).collect();
            return Err(EngineError::Failed(format!(
                "docling convert returned HTTP {}: {}",
                status, snippet
            )));
        }

        let body: DoclingConvertResponse = resp.json().await.map_err(|e| {
            EngineError::Failed(format!("docling convert response not JSON: {}", e))
        })?;

        if body.status != "success" {
            return Err(EngineError::Failed(format!(
                "docling convert status was {:?}",
                body.status
            )));
        }
        if !body.errors.is_empty() {
            return Err(EngineError::Failed(format!(
                "docling convert reported errors: {:?}",
                body.errors
            )));
        }
        let md = body
            .document
            .and_then(|d| d.md_content)
            .ok_or_else(|| EngineError::Failed("docling convert returned no md_content".into()))?;
        if md.trim().is_empty() {
            return Err(EngineError::Failed(
                "docling convert returned empty markdown".into(),
            ));
        }

        Ok(assemble_page_anchors(
            &md,
            DOCLING_PAGE_BREAK_SENTINEL,
            start_page,
        ))
    }
}

/// Character floor for the text-layer probe: a PDF whose in-process
/// extraction yields fewer trimmed characters than this is treated as
/// having no usable text layer (i.e. a scan) and offered to the OCR
/// pre-step. Deliberately low — `ocrmypdf --skip-text` leaves any real
/// text alone, so a false "no text layer" only costs an OCR pass. Shares
/// the single minimum-text floor with the orchestrator.
pub const OCR_PROBE_MIN_CHARS: usize = MIN_EXTRACTED_CHARS;

/// Wall-clock ceiling for one `ocrmypdf` run.
const OCRMYPDF_TIMEOUT: Duration = Duration::from_secs(300);

/// Outcome of the cheap in-process text-layer probe.
enum TextLayer {
    /// Enough text read — a normal text PDF.
    Present,
    /// Near-nothing read — a scan.
    Absent,
    /// `pdf-extract` errored. Ambiguous: text-bearing PDFs defeat it, but
    /// so do scans it chokes on.
    Unknown,
}

/// Cheap text-layer probe. Distinguishes a definite scan (`Absent`) from a
/// probe that simply failed (`Unknown`) so the two OCR call sites can weigh
/// the ambiguous case differently: the pre-step stays conservative (only a
/// definite scan pre-empts Docling), while the last-ditch rescue is
/// aggressive (an ambiguous probe still earns an OCR attempt once every
/// other route has already produced nothing).
async fn probe_text_layer(pdf_path: &Path) -> TextLayer {
    match PdfExtractEngine.extract(pdf_path).await {
        Ok(text) if text.trim().chars().count() >= OCR_PROBE_MIN_CHARS => TextLayer::Present,
        Ok(_) => TextLayer::Absent,
        Err(_) => TextLayer::Unknown,
    }
}

/// A temp file removed on drop. Holds the OCR'd copy so the original PDF
/// is never mutated.
struct TempPdf(PathBuf);

impl TempPdf {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempPdf {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Run `ocrmypdf --skip-text` on `pdf_path` into a fresh temp file and
/// return a guard that deletes it on drop. The input is only ever read;
/// the OCR text layer exists solely in the temp copy.
async fn run_ocrmypdf(binary: &Path, pdf_path: &Path) -> std::result::Result<TempPdf, EngineError> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let out = std::env::temp_dir().join(format!(
        "zotero-mcp-ocr-{}-{}.pdf",
        std::process::id(),
        nanos
    ));
    let guard = TempPdf(out.clone());

    // --output-type pdf skips the Ghostscript PDF/A conversion; the copy
    // is transient input for Docling, not an archival artifact.
    let run = tokio::process::Command::new(binary)
        .arg("--skip-text")
        .arg("--output-type")
        .arg("pdf")
        .arg("--")
        .arg(pdf_path)
        .arg(&out)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();

    let output = match tokio::time::timeout(OCRMYPDF_TIMEOUT, run).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return Err(EngineError::Failed(format!(
                "failed to run ocrmypdf: {}",
                e
            )));
        }
        Err(_) => return Err(EngineError::Timeout(OCRMYPDF_TIMEOUT.as_secs())),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let trimmed = stderr.trim();
        // Keep the tail: ocrmypdf prints progress first, the error last.
        let tail: String = {
            let rev: Vec<char> = trimmed.chars().rev().take(400).collect();
            rev.into_iter().rev().collect()
        };
        return Err(EngineError::Failed(format!(
            "ocrmypdf exited {}: {}",
            output.status, tail
        )));
    }
    if !out.exists() {
        return Err(EngineError::Failed(
            "ocrmypdf reported success but produced no output file".into(),
        ));
    }

    Ok(guard)
}

/// A unique temp path in the system temp dir with the given extension.
fn temp_path(tag: &str, ext: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "zotero-mcp-{}-{}-{}-{}.{}",
        tag,
        std::process::id(),
        nanos,
        seq,
        ext
    ))
}

/// The document's true total page count, independent of any window and of
/// the extraction engines. Poppler `pdfinfo` first — it reads only the
/// structure and is near-instant even on a 400-page scan; `lopdf` (pure
/// Rust) is a slow fallback for hosts without Poppler; `0` when neither can,
/// which the orchestrator records as an unknown count rather than failing.
///
/// The order matters: `lopdf::Document::load` fully parses the file and takes
/// *minutes* on a large image-heavy PDF, so it must never be the primary path.
async fn total_page_count(path: &Path) -> u32 {
    // pdfinfo (Poppler): `Pages:            N`.
    if let Ok(bin) = which::which("pdfinfo") {
        if let Ok(out) = tokio::process::Command::new(bin)
            .arg("--")
            .arg(path)
            .stdin(std::process::Stdio::null())
            .output()
            .await
        {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                for line in text.lines() {
                    if let Some(rest) = line.strip_prefix("Pages:") {
                        if let Ok(n) = rest.trim().parse::<u32>() {
                            return n;
                        }
                    }
                }
            }
        }
    }
    // Pure-Rust fallback (slow on large files; only when Poppler is absent).
    let p = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        lopdf::Document::load(&p)
            .ok()
            .map(|doc| doc.get_pages().len() as u32)
            .filter(|n| *n > 0)
    })
    .await
    .ok()
    .flatten()
    .unwrap_or(0)
}

/// Slice pages `from..=to` (1-indexed, inclusive; assumed already clamped to
/// `1..=total`) of `pdf_path` into a fresh temp PDF, returning a guard that
/// deletes it on drop. The original file is only ever read.
///
/// Poppler `pdfseparate` + `pdfunite` is the primary path — lossless
/// structural surgery in ~1s even on a 400-page scan. `lopdf` is a pure-Rust
/// fallback for hosts without Poppler, but it parses the whole file and is
/// *minutes* slow on large PDFs, so it is a last resort, not the default.
async fn slice_pages(
    pdf_path: &Path,
    from: u32,
    to: u32,
    total: u32,
) -> std::result::Result<TempPdf, EngineError> {
    if let (Ok(sep), Ok(unite)) = (which::which("pdfseparate"), which::which("pdfunite")) {
        match slice_pages_poppler(&sep, &unite, pdf_path, from, to).await {
            Ok(temp) => return Ok(temp),
            Err(e) => tracing::warn!(
                error = %e,
                "pdfseparate/pdfunite slice failed; falling back to the slow lopdf slicer"
            ),
        }
    }
    slice_pages_lopdf(pdf_path, from, to, total).await
}

/// Poppler slice: `pdfseparate -f from -l to` writes one temp PDF per page,
/// then (for a multi-page window) `pdfunite` merges them into one. The
/// per-page files are cleaned up before returning; the merged window is the
/// returned guard.
async fn slice_pages_poppler(
    pdfseparate: &Path,
    pdfunite: &Path,
    pdf_path: &Path,
    from: u32,
    to: u32,
) -> std::result::Result<TempPdf, EngineError> {
    let pattern = temp_path("sep", "%d.pdf");
    let sep_status = tokio::process::Command::new(pdfseparate)
        .arg("-f")
        .arg(from.to_string())
        .arg("-l")
        .arg(to.to_string())
        .arg("--")
        .arg(pdf_path)
        .arg(&pattern)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| EngineError::Failed(format!("failed to run pdfseparate: {e}")))?;
    // The `%d` in the pattern expands to each page number.
    let pat = pattern.to_string_lossy();
    let per_page: Vec<PathBuf> = (from..=to)
        .map(|n| PathBuf::from(pat.replacen("%d", &n.to_string(), 1)))
        .collect();
    let _page_guards: Vec<TempPdf> = per_page.iter().cloned().map(TempPdf).collect();
    if !sep_status.status.success() || per_page.iter().any(|p| !p.exists()) {
        let stderr = String::from_utf8_lossy(&sep_status.stderr);
        return Err(EngineError::Failed(format!(
            "pdfseparate produced no output for pages {from}..={to}: {}",
            stderr.trim()
        )));
    }
    let out = temp_path("slice", "pdf");
    let guard = TempPdf(out.clone());
    if per_page.len() == 1 {
        // Single page: no merge needed, just move it into place.
        tokio::fs::rename(&per_page[0], &out)
            .await
            .map_err(|e| EngineError::Failed(format!("failed to place single-page slice: {e}")))?;
        return Ok(guard);
    }
    let unite = tokio::process::Command::new(pdfunite)
        .arg("--")
        .args(&per_page)
        .arg(&out)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| EngineError::Failed(format!("failed to run pdfunite: {e}")))?;
    if !unite.status.success() || !out.exists() {
        let stderr = String::from_utf8_lossy(&unite.stderr);
        return Err(EngineError::Failed(format!(
            "pdfunite failed to merge window {from}..={to}: {}",
            stderr.trim()
        )));
    }
    Ok(guard)
}

/// Pure-Rust `lopdf` slice: delete every page outside the window, prune, save.
/// Correct but slow on large files (`lopdf` fully parses the document), so it
/// is only reached when Poppler is unavailable. Runs off the async runtime.
async fn slice_pages_lopdf(
    pdf_path: &Path,
    from: u32,
    to: u32,
    total: u32,
) -> std::result::Result<TempPdf, EngineError> {
    let out = temp_path("slice", "pdf");
    let guard = TempPdf(out.clone());
    let src = pdf_path.to_path_buf();
    let dst = out.clone();
    let res = tokio::task::spawn_blocking(move || -> std::result::Result<(), String> {
        let mut doc = lopdf::Document::load(&src).map_err(|e| e.to_string())?;
        // delete_pages resolves page numbers against the original numbering,
        // so passing all out-of-window pages together in one call is correct.
        let to_delete: Vec<u32> = (1..from).chain((to + 1)..=total).collect();
        if !to_delete.is_empty() {
            doc.delete_pages(&to_delete);
        }
        doc.prune_objects();
        doc.save(&dst).map_err(|e| e.to_string())?;
        Ok(())
    })
    .await;
    match res {
        Ok(Ok(())) => Ok(guard),
        Ok(Err(e)) => Err(EngineError::Failed(format!("failed to slice pages: {}", e))),
        Err(je) => Err(EngineError::Failed(format!("slice task failed: {}", je))),
    }
}

pub async fn get_pdf_text(
    pool: &ReadOnlyPool,
    parent_key: &str,
    library_id: i64,
    storage_dir: &Path,
    engines: &PdfEngines,
    plain: bool,
    window: Option<(u32, u32)>,
) -> Result<PdfTextResult> {
    let pdf_path = resolve_path(pool, parent_key, library_id, storage_dir).await?;
    let storage_item_dir = pdf_path
        .parent()
        .ok_or_else(|| Error::AttachmentNotFound(parent_key.into()))?
        .to_path_buf();
    extract_windowed(&pdf_path, &storage_item_dir, engines, plain, window).await
}

pub async fn get_pdf_first_pages(
    pool: &ReadOnlyPool,
    parent_key: &str,
    library_id: i64,
    storage_dir: &Path,
    n_pages: usize,
    engines: &PdfEngines,
    plain: bool,
) -> Result<PdfTextResult> {
    // The first N pages ARE a page window `[1, n]`: extract only that window
    // so a large (or scanned) document is not fully processed just to return
    // its opening pages. The flat-text path (no page anchors) still cannot
    // page-slice its own output, so `truncate_to_first_pages` also caps it.
    let window = if n_pages == 0 {
        Some((1, 1))
    } else {
        Some((1, n_pages as u32))
    };
    let full = get_pdf_text(
        pool,
        parent_key,
        library_id,
        storage_dir,
        engines,
        plain,
        window,
    )
    .await?;
    Ok(truncate_to_first_pages(full, n_pages))
}

/// Character cap per requested page for output without page anchors,
/// where true page boundaries are unknown to the engine.
const PLAIN_CHARS_PER_PAGE: usize = 3500;

/// Truncate a full extraction to its first `n_pages` pages.
///
/// Page-anchored output is cut on page boundaries: the first `n_pages`
/// `--- p.N ---` sections are kept whole (tables and anchors are never
/// sliced mid-way) and the completeness report is adjusted to describe
/// only the retained pages — `pages`, `per_page_chars`, and the drop
/// vectors are filtered to the retained page numbers, with a note
/// declaring the truncation. Un-anchored (plain) output falls back to a
/// character cap, also declared in the notes. Pure — no I/O; public so
/// integration tests can drive it on a real extraction result.
pub fn truncate_to_first_pages(full: PdfTextResult, n_pages: usize) -> PdfTextResult {
    if full.page_anchors {
        let mut retained_pages: Vec<u32> = Vec::new();
        let mut kept_lines: Vec<&str> = Vec::new();
        let mut total_pages = 0usize;
        let mut keeping = true;
        for line in full.text.lines() {
            if let Some(n) = parse_page_anchor(line) {
                total_pages += 1;
                if total_pages > n_pages {
                    keeping = false;
                } else {
                    retained_pages.push(n);
                }
            }
            if keeping {
                kept_lines.push(line);
            }
        }
        if total_pages <= n_pages {
            return full;
        }

        let kept = kept_lines.join("\n");
        let text = format!(
            "{}\n\n[... truncated: first {} of {} pages ...]\n",
            kept.trim_end(),
            retained_pages.len(),
            total_pages
        );

        let retained: std::collections::HashSet<u32> = retained_pages.iter().copied().collect();
        let mut completeness = full.completeness;
        completeness.pages = retained_pages.len() as u32;
        completeness.per_page_chars.truncate(retained_pages.len());
        completeness
            .undecoded_formulas
            .retain(|p| retained.contains(p));
        completeness
            .untranscribed_images
            .retain(|p| retained.contains(p));
        completeness.ocr_pages.retain(|p| retained.contains(p));
        completeness.low_text_pages.retain(|p| retained.contains(p));
        completeness.notes.push(format!(
            "output truncated to the first {} of {} pages; this report describes \
             only the retained pages",
            retained_pages.len(),
            total_pages
        ));

        let n = text.chars().count();
        PdfTextResult {
            text,
            source: full.source,
            character_count: n,
            format: full.format,
            page_anchors: true,
            completeness,
        }
    } else {
        let cap = n_pages * PLAIN_CHARS_PER_PAGE;
        if full.text.chars().count() <= cap {
            return full;
        }
        let mut text: String = full.text.chars().take(cap).collect();
        text.push_str("\n[... truncated ...]");
        let mut completeness = full.completeness;
        completeness.notes.push(format!(
            "output truncated to the first {} characters; page boundaries are \
             unknown without page anchors",
            cap
        ));
        let n = text.chars().count();
        PdfTextResult {
            text,
            source: full.source,
            character_count: n,
            format: full.format,
            page_anchors: false,
            completeness,
        }
    }
}

/// Whole-document orchestrator — the historical entry point. Equivalent to
/// [`extract_windowed`] with no page window. Public so integration tests
/// (and callers already holding a resolved PDF path) can drive the full
/// route stack directly.
pub async fn extract(
    pdf_path: &Path,
    storage_item_dir: &Path,
    engines: &PdfEngines,
    plain: bool,
) -> Result<PdfTextResult> {
    extract_windowed(pdf_path, storage_item_dir, engines, plain, None).await
}

/// Windowed orchestrator.
///
/// `window: Some((from, to))` extracts only that inclusive 1-indexed page
/// range: the PDF is sliced to those pages locally (`lopdf`) and the slice —
/// not the whole file — is what every route (Docling, OCR, flat chain) sees,
/// so per-call work is bounded by the window, not the document size. Page
/// anchors carry the document's true page numbers. `window: None` extracts
/// the whole document, but a whole-document request on a PDF exceeding
/// `engines.whole_document_max_pages()` is refused with `PdfDocumentTooLarge`
/// so a large scan is never silently un-extractable — it is read via windows.
///
/// Every result reports the document's true `total_pages`, independent of the
/// window, so a caller can walk a large document window by window.
pub async fn extract_windowed(
    pdf_path: &Path,
    storage_item_dir: &Path,
    engines: &PdfEngines,
    plain: bool,
    window: Option<(u32, u32)>,
) -> Result<PdfTextResult> {
    // Total page count up front (engine-independent): reported on every
    // result and the gate for the large-document guard.
    let total = total_page_count(pdf_path).await;

    // A large whole-document request is refused, not silently attempted:
    // OCR + layout conversion over hundreds of pages would exceed the time
    // budget and the response size. Windowed requests are never refused.
    if window.is_none() && total > engines.whole_document_max_pages() {
        return Err(Error::PdfDocumentTooLarge {
            path: pdf_path.display().to_string(),
            pages: total,
            threshold: engines.whole_document_max_pages(),
        });
    }

    // Resolve the window to a working file. When a sub-range is requested we
    // slice those pages into a temp PDF (cheap structural surgery) and run
    // the whole pipeline over the slice; `start_page` offsets the page
    // anchors back to true document page numbers. `_slice_guard` keeps the
    // temp file alive for the duration of extraction.
    let mut start_page: u32 = 1;
    let mut window_note: Option<String> = None;
    let _slice_guard: Option<TempPdf>;
    let working_path: PathBuf = match window {
        Some((from, to)) if total > 0 => {
            let from = from.max(1).min(total);
            let to = to.max(from).min(total);
            start_page = from;
            window_note = Some(format!(
                "page window {from}..={to} of {total} total pages; this result and its \
                 completeness report describe only these pages"
            ));
            if from == 1 && to == total {
                _slice_guard = None;
                pdf_path.to_path_buf()
            } else {
                let temp = slice_pages(pdf_path, from, to, total).await.map_err(|e| {
                    Error::Pdf(format!("failed to slice page window {from}..={to}: {e}"))
                })?;
                let p = temp.path().to_path_buf();
                _slice_guard = Some(temp);
                p
            }
        }
        // A window was requested but the page count is unknown: process the
        // whole file (bounded only by the engine timeouts) rather than guess
        // a slice, and say so.
        Some((from, to)) => {
            start_page = from.max(1);
            window_note = Some(format!(
                "page window {from}..={to} requested but the document page count could \
                 not be determined; extracted whole-document instead"
            ));
            _slice_guard = None;
            pdf_path.to_path_buf()
        }
        None => {
            _slice_guard = None;
            pdf_path.to_path_buf()
        }
    };
    // The `.zotero-ft-cache` is the WHOLE-document Zotero extraction; it must
    // neither be read as, nor overwritten by, a single window.
    let use_cache = window.is_none();

    let mut result = extract_core(
        &working_path,
        storage_item_dir,
        engines,
        plain,
        start_page,
        use_cache,
    )
    .await?;
    result.completeness.total_pages = total;
    if let Some(note) = window_note {
        result.completeness.notes.push(note);
    }
    Ok(result)
}

/// The route stack over a single (already window-sliced) working file.
/// `start_page` is the true document page number of the working file's first
/// page (1 for whole-document); `use_cache` gates the `.zotero-ft-cache`
/// read/write to whole-document requests only.
async fn extract_core(
    pdf_path: &Path,
    storage_item_dir: &Path,
    engines: &PdfEngines,
    plain: bool,
    start_page: u32,
    use_cache: bool,
) -> Result<PdfTextResult> {
    // 0. Layout-aware primary route (Docling), when configured. Tried
    //    ahead of the whole flat-text chain — including the Zotero cache,
    //    which is itself a flat extraction — for arbiter quality. Any
    //    health/convert failure falls through to that chain, whose
    //    results report `complete: false`. Skipped when the caller asked
    //    for `plain` output.
    if let Some(docling) = engines.docling().filter(|_| !plain) {
        if docling.healthy().await {
            // OCR pre-step (not a route): when the PDF has no usable text
            // layer, run `ocrmypdf --skip-text` into a temp copy and send
            // *that* to Docling. The original file is never touched.
            // Missing or failing ocrmypdf degrades gracefully: the
            // original goes to Docling as-is and the gap is recorded in
            // the completeness notes.
            let mut ocr_temp: Option<TempPdf> = None;
            let mut ocr_note: Option<String> = None;
            // Conservative here: only a *definite* scan pre-empts Docling
            // with OCR. An ambiguous (`Unknown`) probe goes straight to
            // Docling, which does its own OCR anyway.
            if matches!(probe_text_layer(pdf_path).await, TextLayer::Absent) {
                match engines.ocrmypdf() {
                    Some(bin) => match run_ocrmypdf(bin, pdf_path).await {
                        Ok(temp) => {
                            tracing::info!(
                                path = %pdf_path.display(),
                                "no usable text layer; applied ocrmypdf OCR pre-step"
                            );
                            ocr_temp = Some(temp);
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                path = %pdf_path.display(),
                                "ocrmypdf failed; continuing without the OCR pre-step"
                            );
                            ocr_note = Some(format!(
                                "no usable text layer detected but the OCR pre-step failed \
                                 ({}); scanned content may be missing",
                                e
                            ));
                        }
                    },
                    None => {
                        tracing::warn!(
                            path = %pdf_path.display(),
                            "no usable text layer and ocrmypdf is not available; skipping OCR"
                        );
                        ocr_note = Some(
                            "no usable text layer detected and ocrmypdf is not available; \
                             scanned content may be missing (install ocrmypdf for the OCR \
                             pre-step)"
                                .into(),
                        );
                    }
                }
            }
            let (convert_path, source) = match &ocr_temp {
                Some(temp) => (temp.path(), PdfTextSource::OcrThenDocling),
                None => (pdf_path, PdfTextSource::Docling),
            };

            match docling.extract_markdown(convert_path, start_page).await {
                Ok(extraction) if !meets_text_floor(&extraction.markdown) => {
                    tracing::warn!(
                        path = %pdf_path.display(),
                        "docling markdown below the minimum text floor; \
                         falling back to flat-text chain"
                    );
                }
                Ok(extraction) => {
                    let mut completeness = Completeness::from_page_anchored_markdown(
                        &extraction.markdown,
                        source,
                        extraction.formula_enrichment,
                        Vec::new(),
                    );
                    if ocr_temp.is_some() {
                        // The pre-step ran because the working file lacked a
                        // text layer, so every page in the result was
                        // recovered by OCR — numbered from the window start.
                        completeness.ocr_pages =
                            (start_page..start_page + completeness.pages).collect();
                    }
                    if let Some(note) = ocr_note {
                        // A scan we could not OCR: never claim completeness.
                        completeness.complete = false;
                        completeness.notes.push(note);
                    }
                    let n = extraction.markdown.chars().count();
                    return Ok(PdfTextResult {
                        text: extraction.markdown,
                        source,
                        character_count: n,
                        format: PdfFormat::Markdown,
                        page_anchors: true,
                        completeness,
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        path = %pdf_path.display(),
                        "docling convert failed; falling back to flat-text chain"
                    );
                }
            }
        } else {
            tracing::warn!(
                path = %pdf_path.display(),
                "docling health check failed; falling back to flat-text chain"
            );
        }
    }

    // 1. Cache hit — only when the cached text clears the minimum floor.
    //    An empty/near-empty cache (e.g. Zotero's own failed extraction)
    //    must never be returned as success. Skipped for windowed requests:
    //    the cache holds the whole-document text, not this window.
    let cache = storage_item_dir.join(".zotero-ft-cache");
    if use_cache && cache.exists() {
        let text = tokio::fs::read_to_string(&cache).await?;
        if meets_text_floor(&text) {
            let n = text.chars().count();
            return Ok(PdfTextResult {
                text,
                source: PdfTextSource::ZoteroCache,
                character_count: n,
                format: PdfFormat::Plain,
                page_anchors: false,
                completeness: Completeness::flat_text(PdfTextSource::ZoteroCache),
            });
        }
        tracing::warn!(
            path = %cache.display(),
            "ignoring .zotero-ft-cache below the minimum text floor"
        );
    }

    // 2+3. Flat-text chain (pdf-extract → pdftotext) on the original file.
    let chain = flat_chain(pdf_path, engines).await;
    if let FlatChainOutcome::Extracted { text, source } = chain {
        if use_cache && source == PdfTextSource::PdftotextFallback {
            // Cache write (best-effort). Only for whole-document runs — a
            // window's partial text must never overwrite the full cache.
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
        }
        let n = text.chars().count();
        return Ok(PdfTextResult {
            text,
            source,
            character_count: n,
            format: PdfFormat::Plain,
            page_anchors: false,
            completeness: Completeness::flat_text(source),
        });
    }
    let FlatChainOutcome::Nothing {
        error,
        sub_floor_success,
    } = chain
    else {
        unreachable!("Extracted handled above");
    };

    // 4. OCR rescue: nothing extracted above the floor. When the PDF has
    //    no usable text layer (a scan) and ocrmypdf is available, OCR to a
    //    temp copy — the original is never touched — and run the flat-text
    //    chain over that copy. This honours the nothing-extractable
    //    invariant even when the Docling route is unreachable. Aggressive
    //    here: an ambiguous (`Unknown`) probe still earns an OCR attempt
    //    now that every other route has produced nothing. Skipped for
    //    `plain` callers, who asked for the flat path that never OCRs.
    let layer = if plain {
        TextLayer::Present // plain callers never OCR; treat as "has text layer".
    } else {
        probe_text_layer(pdf_path).await
    };
    if !matches!(layer, TextLayer::Present) {
        if let Some(bin) = engines.ocrmypdf() {
            match run_ocrmypdf(bin, pdf_path).await {
                Ok(temp) => {
                    if let FlatChainOutcome::Extracted { text, source } =
                        flat_chain(temp.path(), engines).await
                    {
                        tracing::info!(
                            path = %pdf_path.display(),
                            "OCR rescue recovered text via the flat-text chain"
                        );
                        let n = text.chars().count();
                        let mut completeness = Completeness::flat_text(source);
                        completeness.notes.push(
                            "text recovered by an ocrmypdf OCR pre-step (the Docling \
                             route was unavailable); a flat-text engine read the OCR'd \
                             copy, so page-level OCR attribution is unavailable"
                                .into(),
                        );
                        return Ok(PdfTextResult {
                            text,
                            source,
                            character_count: n,
                            format: PdfFormat::Plain,
                            page_anchors: false,
                            completeness,
                        });
                    }
                    tracing::warn!(
                        path = %pdf_path.display(),
                        "OCR rescue also produced no usable text"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        path = %pdf_path.display(),
                        "ocrmypdf failed during the OCR rescue"
                    );
                }
            }
        }
        // A *definite* scan nothing could read: loud error naming the
        // remedy — never empty text as success. An ambiguous (`Unknown`)
        // probe falls through to the concrete flat-chain error below,
        // which is more informative than a generic scan message.
        if matches!(layer, TextLayer::Absent) {
            return Err(Error::PdfNothingExtractable {
                path: pdf_path.display().to_string(),
                detail: match engines.ocrmypdf() {
                    Some(_) => "no usable text layer and the OCR pre-step could not recover text",
                    None => "no usable text layer and ocrmypdf is not available",
                }
                .into(),
            });
        }
    }

    // The PDF has (or may have) a text layer, yet no route extracted
    // above the floor. Sub-floor "successes" must still surface as the
    // loud nothing-extractable error; hard engine failures keep their
    // original error shapes.
    if sub_floor_success {
        return Err(Error::PdfNothingExtractable {
            path: pdf_path.display().to_string(),
            detail: "every route returned empty or near-empty text".into(),
        });
    }
    Err(error)
}

/// Outcome of one pass of the flat-text chain over a single file.
enum FlatChainOutcome {
    /// Above-floor text from an engine.
    Extracted { text: String, source: PdfTextSource },
    /// No above-floor text. `error` is what the orchestrator surfaces
    /// when no OCR rescue applies; `sub_floor_success` is true when an
    /// engine returned sub-floor text *as success* (the
    /// nothing-extractable shape, as opposed to hard engine failures).
    Nothing {
        error: Error,
        sub_floor_success: bool,
    },
}

/// Run the flat-text chain (primary `pdf-extract`, then the `pdftotext`
/// fallback) over `path`, applying the minimum-text floor to every
/// engine "success" so sub-floor output is treated as "did not extract".
async fn flat_chain(path: &Path, engines: &PdfEngines) -> FlatChainOutcome {
    let (primary_err, primary_sub_floor) = match engines.primary().extract(path).await {
        Ok(text) if meets_text_floor(&text) => {
            return FlatChainOutcome::Extracted {
                text,
                source: PdfTextSource::LiveExtract,
            };
        }
        Ok(_) => (
            "pdf-extract produced no usable text (below the minimum floor)".to_string(),
            true,
        ),
        Err(e) => (e.to_string(), false),
    };

    let fallback = match engines.fallback() {
        FallbackState::Ready(eng) => eng,
        FallbackState::Disabled => {
            return FlatChainOutcome::Nothing {
                error: Error::Pdf(primary_err),
                sub_floor_success: primary_sub_floor,
            };
        }
        FallbackState::BinaryMissing => {
            tracing::warn!(
                error = %primary_err,
                path = %path.display(),
                "pdf-extract failed and pdftotext fallback is unavailable"
            );
            return FlatChainOutcome::Nothing {
                error: Error::PdftotextMissing,
                sub_floor_success: primary_sub_floor,
            };
        }
    };

    tracing::warn!(
        error = %primary_err,
        path = %path.display(),
        "pdf-extract produced nothing usable; trying pdftotext fallback"
    );

    match fallback.extract(path).await {
        Ok(text) if meets_text_floor(&text) => FlatChainOutcome::Extracted {
            text,
            source: PdfTextSource::PdftotextFallback,
        },
        Ok(_) => FlatChainOutcome::Nothing {
            error: Error::PdfNothingExtractable {
                path: path.display().to_string(),
                detail: format!(
                    "{}; pdftotext produced no usable text (below the minimum floor)",
                    primary_err
                ),
            },
            sub_floor_success: true,
        },
        // A real timeout / hard failure is a distinct, often transient
        // condition — surface it verbatim rather than letting a sub-floor
        // primary mask it as the "install ocrmypdf" scan error.
        Err(EngineError::Timeout(secs)) => FlatChainOutcome::Nothing {
            error: Error::PdftotextTimeout(secs, path.display().to_string()),
            sub_floor_success: false,
        },
        Err(EngineError::Failed(msg)) => FlatChainOutcome::Nothing {
            error: Error::PdfAllEnginesFailed {
                pdf_extract: primary_err,
                pdftotext: msg,
            },
            sub_floor_success: false,
        },
    }
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
mod completeness_tests {
    use super::*;

    /// Page-anchored markdown with markers spread across pages:
    /// p.1 prose only, p.2 two undecoded formulas, p.3 a lone image,
    /// p.4 prose only.
    fn marked_markdown() -> String {
        format!(
            "--- p.1 ---\n\n\
             # A Title\n\n\
             This first page carries enough prose to sit comfortably above the \
             low-text character floor used by the completeness derivation.\n\n\
             --- p.2 ---\n\n\
             An equation follows, introduced by enough surrounding prose that \
             this page clears the low-text character floor on its own.\n\n\
             {formula}\n\n\
             Prose between the two formula regions on this page.\n\n\
             {formula}\n\n\
             --- p.3 ---\n\n\
             {image}\n\n\
             --- p.4 ---\n\n\
             The final page also carries enough prose to sit comfortably above \
             the low-text character floor used by the derivation.\n",
            formula = FORMULA_NOT_DECODED_MARKER,
            image = IMAGE_MARKER,
        )
    }

    #[test]
    fn derives_per_page_counts_and_locations_from_markers() {
        let c = Completeness::from_page_anchored_markdown(
            &marked_markdown(),
            PdfTextSource::Docling,
            true,
            Vec::new(),
        );

        assert_eq!(c.pages, 4);
        assert_eq!(c.per_page_chars.len(), 4);
        // One entry per marker occurrence: page 2 twice, page 3 once.
        assert_eq!(c.undecoded_formulas, vec![2, 2]);
        assert_eq!(c.untranscribed_images, vec![3]);
        // Page 3 is nothing but an image marker: low text after markers are
        // stripped; the prose pages are above the floor.
        assert_eq!(c.low_text_pages, vec![3]);
        assert!(c.per_page_chars[0] >= LOW_TEXT_CHAR_FLOOR);
        assert!(c.per_page_chars[2] < LOW_TEXT_CHAR_FLOOR);
        assert!(c.per_page_chars[3] >= LOW_TEXT_CHAR_FLOOR);
        // Unresolved drops => not complete.
        assert!(!c.complete);
        assert_eq!(c.engine, PdfTextSource::Docling);
        assert!(c.ocr_pages.is_empty());
    }

    #[test]
    fn clean_layout_extraction_is_complete() {
        let md = "--- p.1 ---\n\n\
                  Plenty of prose on the first page, well above the low-text \
                  floor, with no formula or image placeholders anywhere.\n\n\
                  --- p.2 ---\n\n\
                  A second page of equally unremarkable but sufficiently long \
                  prose so that nothing here reads as a possible drop.\n";
        let c =
            Completeness::from_page_anchored_markdown(md, PdfTextSource::Docling, true, Vec::new());

        assert!(c.complete);
        assert_eq!(c.pages, 2);
        assert!(c.undecoded_formulas.is_empty());
        assert!(c.untranscribed_images.is_empty());
        assert!(c.low_text_pages.is_empty());
        assert!(c.notes.is_empty());
    }

    #[test]
    fn ocr_route_with_no_drops_is_complete_and_records_ocr_pages() {
        let md = "--- p.1 ---\n\n\
                  Recovered prose from a scanned page, long enough to clear \
                  the low-text character floor after OCR has run over it.\n";
        let c = Completeness::from_page_anchored_markdown(
            md,
            PdfTextSource::OcrThenDocling,
            true,
            vec![1],
        );

        assert!(c.complete);
        assert_eq!(c.engine, PdfTextSource::OcrThenDocling);
        assert_eq!(c.ocr_pages, vec![1]);
    }

    #[test]
    fn enrichment_off_is_never_complete() {
        let md = "--- p.1 ---\n\n\
                  Prose without any markers at all, long enough to clear the \
                  low-text floor, yet enrichment was not enabled here.\n";
        let c = Completeness::from_page_anchored_markdown(
            md,
            PdfTextSource::Docling,
            false,
            Vec::new(),
        );

        assert!(!c.complete);
        assert!(c.notes.iter().any(|n| n.contains("formula enrichment")));
    }

    #[test]
    fn markdown_without_anchors_is_treated_as_a_single_page() {
        let md = format!(
            "Just one block of text with a formula. {}",
            FORMULA_NOT_DECODED_MARKER
        );
        let c = Completeness::from_page_anchored_markdown(
            &md,
            PdfTextSource::Docling,
            true,
            Vec::new(),
        );

        assert_eq!(c.pages, 1);
        assert_eq!(c.per_page_chars.len(), 1);
        assert_eq!(c.undecoded_formulas, vec![1]);
        assert!(!c.complete);
    }

    #[test]
    fn flat_text_report_is_incomplete_with_note() {
        let c = Completeness::flat_text(PdfTextSource::PdftotextFallback);

        assert!(!c.complete);
        assert_eq!(c.engine, PdfTextSource::PdftotextFallback);
        assert_eq!(c.pages, 0);
        assert!(c.per_page_chars.is_empty());
        assert!(c.undecoded_formulas.is_empty());
        assert!(c.untranscribed_images.is_empty());
        assert!(c.ocr_pages.is_empty());
        assert!(c.low_text_pages.is_empty());
        assert_eq!(
            c.notes,
            vec!["flat-text engine cannot detect tables/formulas/images".to_string()]
        );
    }
}

#[cfg(test)]
mod assembly_tests {
    use super::*;

    #[test]
    fn sentinel_split_becomes_numbered_anchors() {
        let md = format!(
            "Page one.{s}Page two.{s}Page three.",
            s = DOCLING_PAGE_BREAK_SENTINEL
        );
        let out = assemble_page_anchors(&md, DOCLING_PAGE_BREAK_SENTINEL, 1);
        assert_eq!(
            out,
            "--- p.1 ---\n\nPage one.\n\n--- p.2 ---\n\nPage two.\n\n--- p.3 ---\n\nPage three.\n"
        );
    }

    #[test]
    fn windowed_anchors_carry_true_document_page_numbers() {
        // A window sliced to start at page 5 must number its anchors 5,6,7 —
        // not 1,2,3 — so downstream page references stay correct.
        let md = format!(
            "Page five.{s}Page six.{s}Page seven.",
            s = DOCLING_PAGE_BREAK_SENTINEL
        );
        let out = assemble_page_anchors(&md, DOCLING_PAGE_BREAK_SENTINEL, 5);
        assert_eq!(
            out,
            "--- p.5 ---\n\nPage five.\n\n--- p.6 ---\n\nPage six.\n\n--- p.7 ---\n\nPage seven.\n"
        );
    }

    #[test]
    fn markdown_without_sentinel_gets_single_anchor() {
        let out = assemble_page_anchors("Only page.", DOCLING_PAGE_BREAK_SENTINEL, 1);
        assert_eq!(out, "--- p.1 ---\n\nOnly page.\n");
    }

    #[test]
    fn assembled_anchors_round_trip_into_completeness() {
        let md = format!(
            "First page prose.{}Second page with a formula. {}",
            DOCLING_PAGE_BREAK_SENTINEL, FORMULA_NOT_DECODED_MARKER
        );
        let out = assemble_page_anchors(&md, DOCLING_PAGE_BREAK_SENTINEL, 1);
        let c = Completeness::from_page_anchored_markdown(
            &out,
            PdfTextSource::Docling,
            true,
            Vec::new(),
        );
        assert_eq!(c.pages, 2);
        assert_eq!(c.undecoded_formulas, vec![2]);
    }
}

#[cfg(test)]
mod truncation_tests {
    use super::*;

    /// Three page-anchored pages: p.1 prose, p.2 a markdown table plus an
    /// undecoded formula, p.3 an image marker plus a formula. Completeness
    /// is derived from the markdown so the fixture is self-consistent.
    fn three_page_result() -> PdfTextResult {
        let text = format!(
            "--- p.1 ---\n\n\
             First page prose, comfortably long enough to clear the low-text \
             floor used by the completeness derivation for this fixture.\n\n\
             --- p.2 ---\n\n\
             | Region | Q1 | Q2 |\n\
             |---|---|---|\n\
             | North | 1214 | 1180 |\n\
             | South | 986 | 1002 |\n\n\
             Prose after the table so this page clears the low-text floor \
             with room to spare for the truncation assertions below.\n\n\
             {formula}\n\n\
             --- p.3 ---\n\n\
             Third page prose that must be dropped by the truncation.\n\n\
             {image}\n\n\
             {formula}\n",
            formula = FORMULA_NOT_DECODED_MARKER,
            image = IMAGE_MARKER,
        );
        let completeness = Completeness::from_page_anchored_markdown(
            &text,
            PdfTextSource::Docling,
            true,
            Vec::new(),
        );
        let n = text.chars().count();
        PdfTextResult {
            text,
            source: PdfTextSource::Docling,
            character_count: n,
            format: PdfFormat::Markdown,
            page_anchors: true,
            completeness,
        }
    }

    #[test]
    fn markdown_truncation_cuts_on_page_boundaries() {
        let full = three_page_result();
        let full_per_page = full.completeness.per_page_chars.clone();

        let r = truncate_to_first_pages(full, 2);

        // Retained pages are whole: anchors and the entire table survive.
        assert!(r.text.contains("--- p.1 ---"));
        assert!(r.text.contains("--- p.2 ---"));
        assert!(r.text.contains("| North | 1214 | 1180 |"));
        assert!(r.text.contains("| South | 986 | 1002 |"));
        // Dropped page is gone entirely — anchor and content.
        assert!(!r.text.contains("--- p.3 ---"));
        assert!(!r.text.contains("Third page prose"));
        assert!(r.text.contains("[... truncated: first 2 of 3 pages ...]"));

        // The report describes only the retained pages.
        assert_eq!(r.completeness.pages, 2);
        assert_eq!(r.completeness.per_page_chars, full_per_page[..2].to_vec());
        // p.2's formula survives; p.3's formula and image are filtered out.
        assert_eq!(r.completeness.undecoded_formulas, vec![2]);
        assert!(r.completeness.untranscribed_images.is_empty());
        assert!(r
            .completeness
            .notes
            .iter()
            .any(|n| n.contains("truncated to the first 2 of 3 pages")));
        assert_eq!(r.character_count, r.text.chars().count());
    }

    #[test]
    fn markdown_truncation_is_a_noop_when_enough_pages_requested() {
        let full = three_page_result();
        let expected_text = full.text.clone();
        let expected_completeness = full.completeness.clone();

        let r = truncate_to_first_pages(full, 3);

        assert_eq!(r.text, expected_text);
        assert_eq!(r.completeness, expected_completeness);
    }

    #[test]
    fn plain_truncation_caps_chars_and_declares_it() {
        let body = "plain body text ".repeat(1000); // 16k chars
        let n = body.chars().count();
        let full = PdfTextResult {
            text: body,
            source: PdfTextSource::LiveExtract,
            character_count: n,
            format: PdfFormat::Plain,
            page_anchors: false,
            completeness: Completeness::flat_text(PdfTextSource::LiveExtract),
        };

        let r = truncate_to_first_pages(full, 2);

        assert!(r.text.ends_with("[... truncated ...]"));
        assert_eq!(r.character_count, r.text.chars().count());
        assert!(
            r.character_count <= 2 * 3500 + "\n[... truncated ...]".len(),
            "char cap not applied: {}",
            r.character_count
        );
        assert!(r
            .completeness
            .notes
            .iter()
            .any(|n| n.contains("truncated to the first 7000 characters")));
    }

    #[test]
    fn plain_truncation_is_a_noop_for_short_text() {
        let full = PdfTextResult {
            text: "short plain body".into(),
            source: PdfTextSource::LiveExtract,
            character_count: 16,
            format: PdfFormat::Plain,
            page_anchors: false,
            completeness: Completeness::flat_text(PdfTextSource::LiveExtract),
        };
        let r = truncate_to_first_pages(full, 2);
        assert_eq!(r.text, "short plain body");
        assert!(!r.completeness.notes.iter().any(|n| n.contains("truncated")));
    }
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
        PdfEngines {
            docling: None,
            ocrmypdf: None,
            primary,
            fallback,
            whole_document_max_pages: 50,
        }
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
        let r = extract(&pdf, dir.path(), &engines, false).await.unwrap();

        assert_eq!(r.source, PdfTextSource::ZoteroCache);
        assert!(r.text.contains("cached body"));
        assert!(!r.completeness.complete);
    }

    #[tokio::test]
    async fn primary_success_does_not_write_cache() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let engines = engines_with(
            StubEngine::ok("primary text"),
            FallbackState::Ready(StubEngine::never()),
        );
        let r = extract(&pdf, dir.path(), &engines, false).await.unwrap();

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
        let r = extract(&pdf, dir.path(), &engines, false).await.unwrap();

        assert_eq!(r.source, PdfTextSource::PdftotextFallback);
        assert_eq!(r.text, "recovered text");
        assert!(!r.completeness.complete);
        assert!(r
            .completeness
            .notes
            .iter()
            .any(|n| n.contains("flat-text engine")));
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
        let err = extract(&pdf, dir.path(), &engines, false)
            .await
            .unwrap_err();

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
        let err = extract(&pdf, dir.path(), &engines, false)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::PdftotextMissing));
    }

    #[tokio::test]
    async fn fallback_disabled_returns_legacy_pdf_error() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let engines = engines_with(StubEngine::fail("primary"), FallbackState::Disabled);
        let err = extract(&pdf, dir.path(), &engines, false)
            .await
            .unwrap_err();
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
        let err = extract(&pdf, dir.path(), &engines, false)
            .await
            .unwrap_err();
        match err {
            Error::PdftotextTimeout(secs, _) => assert_eq!(secs, 42),
            other => panic!("expected PdftotextTimeout, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn sub_floor_primary_then_timeout_surfaces_timeout_not_scan_error() {
        // A whitespace-heavy sub-floor primary "success" must not let a
        // genuine (often transient) pdftotext timeout be masked as the
        // permanent "install ocrmypdf" scan error.
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let engines = engines_with(
            StubEngine::ok("a b c"), // 3 non-whitespace chars: below the floor
            FallbackState::Ready(StubEngine::timeout(42)),
        );
        let err = extract(&pdf, dir.path(), &engines, false)
            .await
            .unwrap_err();
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
            FallbackState::Ready(StubEngine::ok("rescued text from the stub fallback")),
        );
        let r = extract(&pdf, &nonexistent, &engines, false).await.unwrap();
        assert_eq!(r.source, PdfTextSource::PdftotextFallback);
        assert_eq!(r.text, "rescued text from the stub fallback");
        assert!(!nonexistent.join(".zotero-ft-cache").exists());
    }

    // --- minimum-text floor (F1): sub-floor text is never success ---

    #[tokio::test]
    async fn whitespace_only_cache_is_not_returned_as_success() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".zotero-ft-cache"), "   \n\n \t \n").unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let engines = engines_with(
            StubEngine::ok("primary text recovered by the stub engine"),
            FallbackState::Disabled,
        );
        let r = extract(&pdf, dir.path(), &engines, false).await.unwrap();

        assert_eq!(
            r.source,
            PdfTextSource::LiveExtract,
            "an empty/whitespace ft-cache must not be returned as success"
        );
        assert!(r.text.contains("primary text recovered"));
    }

    #[tokio::test]
    async fn sub_floor_primary_text_continues_to_fallback() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let engines = engines_with(
            StubEngine::ok("x"),
            FallbackState::Ready(StubEngine::ok("fallback recovered the real body text")),
        );
        let r = extract(&pdf, dir.path(), &engines, false).await.unwrap();

        assert_eq!(
            r.source,
            PdfTextSource::PdftotextFallback,
            "sub-floor primary output must be treated as 'did not extract'"
        );
        assert!(r.text.contains("fallback recovered"));
    }

    #[tokio::test]
    async fn image_only_pdf_with_no_docling_and_no_ocr_errors_loudly() {
        // scanned.pdf has no text layer; with no Docling route, no
        // ocrmypdf, and only the real pdf-extract engine (which "succeeds"
        // with near-empty text on scans), extraction must be a loud error
        // naming the OCR remedy — never empty text as success.
        let dir = TempDir::new().unwrap();
        let engines = PdfEngines {
            docling: None,
            ocrmypdf: None,
            primary: Arc::new(PdfExtractEngine),
            fallback: FallbackState::Disabled,
            whole_document_max_pages: 50,
        };
        let err = extract(&fixture("scanned.pdf"), dir.path(), &engines, false)
            .await
            .expect_err("near-empty extraction must not be success");
        let msg = err.to_string();
        assert!(
            msg.contains("ocrmypdf"),
            "loud error must name the OCR remedy, got: {msg}"
        );
    }

    // --- Docling route ordering (stubbed docling-serve via wiremock) ---

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn engines_with_docling(
        url: &str,
        primary: Arc<dyn PdfEngine>,
        fallback: FallbackState,
    ) -> PdfEngines {
        PdfEngines {
            docling: Some(Arc::new(DoclingEngine::new(
                url.to_string(),
                Duration::from_secs(10),
                Duration::from_secs(2),
            ))),
            ocrmypdf: None,
            primary,
            fallback,
            whole_document_max_pages: 50,
        }
    }

    async fn mount_health(server: &MockServer, status: u16) {
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(status))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn docling_success_preempts_cache_and_flat_engines() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".zotero-ft-cache"), "cached body\n").unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let server = MockServer::start().await;
        mount_health(&server, 200).await;
        let md = format!(
            "First page prose.{}Second page prose.",
            DOCLING_PAGE_BREAK_SENTINEL
        );
        Mock::given(method("POST"))
            .and(path("/v1/convert/file"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "document": { "md_content": md },
                "status": "success",
                "errors": []
            })))
            .mount(&server)
            .await;

        let engines = engines_with_docling(
            &server.uri(),
            StubEngine::never(),
            FallbackState::Ready(StubEngine::never()),
        );
        let r = extract(&pdf, dir.path(), &engines, false).await.unwrap();

        assert_eq!(r.source, PdfTextSource::Docling);
        assert_eq!(r.format, PdfFormat::Markdown);
        assert!(r.page_anchors);
        assert!(r.text.contains("--- p.1 ---"));
        assert!(r.text.contains("--- p.2 ---"));
        assert!(r.text.contains("First page prose."));
        assert_eq!(r.completeness.engine, PdfTextSource::Docling);
        assert_eq!(r.completeness.pages, 2);
        assert!(r.completeness.complete);
    }

    #[tokio::test]
    async fn docling_enrichment_failure_retries_without_and_declares_the_gap() {
        use wiremock::matchers::body_string_contains;

        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let server = MockServer::start().await;
        mount_health(&server, 200).await;
        // First convert (do_formula_enrichment=true) fails ...
        Mock::given(method("POST"))
            .and(path("/v1/convert/file"))
            .and(body_string_contains("do_formula_enrichment"))
            .and(body_string_contains("true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "document": null,
                "status": "failure",
                "errors": []
            })))
            .expect(1)
            .mount(&server)
            .await;
        // ... the retry without enrichment succeeds.
        Mock::given(method("POST"))
            .and(path("/v1/convert/file"))
            .and(body_string_contains("do_formula_enrichment"))
            .and(body_string_contains("false"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "document": { "md_content": "Recovered without enrichment." },
                "status": "success",
                "errors": []
            })))
            .expect(1)
            .mount(&server)
            .await;

        let engines = engines_with_docling(
            &server.uri(),
            StubEngine::never(),
            FallbackState::Ready(StubEngine::never()),
        );
        let r = extract(&pdf, dir.path(), &engines, false).await.unwrap();

        assert_eq!(r.source, PdfTextSource::Docling);
        assert_eq!(r.format, PdfFormat::Markdown);
        assert!(r.text.contains("Recovered without enrichment."));
        // The gap is declared: enrichment was off, so never complete.
        assert!(!r.completeness.complete);
        assert!(r
            .completeness
            .notes
            .iter()
            .any(|n| n.contains("formula enrichment")));
    }

    /// Spec scenario "Enrichment unavailable", end to end through the
    /// orchestrator: the service rejects `do_formula_enrichment=true`, the
    /// retry without enrichment returns a formula region as an explicit
    /// undecoded marker — the marker is preserved in the output text AND
    /// its page is recorded in `completeness.undecoded_formulas`.
    #[tokio::test]
    async fn enrichment_unavailable_preserves_undecoded_marker_and_records_page() {
        use wiremock::matchers::body_string_contains;

        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let server = MockServer::start().await;
        mount_health(&server, 200).await;
        // The enrichment convert fails server-side ...
        Mock::given(method("POST"))
            .and(path("/v1/convert/file"))
            .and(body_string_contains("do_formula_enrichment"))
            .and(body_string_contains("true"))
            .respond_with(ResponseTemplate::new(500).set_body_string("enrichment model missing"))
            .expect(1)
            .mount(&server)
            .await;
        // ... the retry without enrichment succeeds, with the formula
        // region preserved as an undecoded marker on page 2.
        let md = format!(
            "First page prose, long enough to clear the low-text floor used \
             by the completeness derivation in this end-to-end test.{}A second \
             page introducing an equation the service could not decode:\n\n{}\n\n\
             followed by enough prose to clear the low-text floor here too.",
            DOCLING_PAGE_BREAK_SENTINEL, FORMULA_NOT_DECODED_MARKER
        );
        Mock::given(method("POST"))
            .and(path("/v1/convert/file"))
            .and(body_string_contains("do_formula_enrichment"))
            .and(body_string_contains("false"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "document": { "md_content": md },
                "status": "success",
                "errors": []
            })))
            .expect(1)
            .mount(&server)
            .await;

        let engines = engines_with_docling(
            &server.uri(),
            StubEngine::never(),
            FallbackState::Ready(StubEngine::never()),
        );
        let r = extract(&pdf, dir.path(), &engines, false).await.unwrap();

        assert_eq!(r.source, PdfTextSource::Docling);
        // The formula region is preserved as an explicit undecoded marker,
        // never silently omitted ...
        assert!(
            r.text.contains(FORMULA_NOT_DECODED_MARKER),
            "undecoded marker missing from output: {:?}",
            r.text
        );
        // ... and its page is recorded in the completeness report.
        assert_eq!(r.completeness.undecoded_formulas, vec![2]);
        assert!(!r.completeness.complete);
        assert!(r
            .completeness
            .notes
            .iter()
            .any(|n| n.contains("formula enrichment")));
    }

    /// Spec scenario "Incomplete extraction is declared, not hidden":
    /// a figure the layout route could not transcribe leaves the report
    /// incomplete with the drop's page identified.
    #[tokio::test]
    async fn dropped_image_is_declared_with_its_page_not_hidden() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let server = MockServer::start().await;
        mount_health(&server, 200).await;
        let md = format!(
            "Opening page of prose, long enough to clear the low-text floor \
             for the purposes of this drop-declaration test fixture.{}Second \
             page carrying an untranscribed chart:\n\n{}\n\nplus enough \
             surrounding prose to clear the low-text floor on this page.",
            DOCLING_PAGE_BREAK_SENTINEL, IMAGE_MARKER
        );
        Mock::given(method("POST"))
            .and(path("/v1/convert/file"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "document": { "md_content": md },
                "status": "success",
                "errors": []
            })))
            .mount(&server)
            .await;

        let engines = engines_with_docling(
            &server.uri(),
            StubEngine::never(),
            FallbackState::Ready(StubEngine::never()),
        );
        let r = extract(&pdf, dir.path(), &engines, false).await.unwrap();

        assert_eq!(r.source, PdfTextSource::Docling);
        assert!(
            !r.completeness.complete,
            "a dropped figure must leave the report incomplete"
        );
        assert_eq!(
            r.completeness.untranscribed_images,
            vec![2],
            "the drop's page must be identified"
        );
        assert!(r.text.contains(IMAGE_MARKER));
    }

    #[tokio::test]
    async fn docling_unhealthy_falls_through_to_cache() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".zotero-ft-cache"), "cached body\n").unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let server = MockServer::start().await;
        mount_health(&server, 500).await;

        let engines = engines_with_docling(
            &server.uri(),
            StubEngine::never(),
            FallbackState::Ready(StubEngine::never()),
        );
        let r = extract(&pdf, dir.path(), &engines, false).await.unwrap();

        assert_eq!(r.source, PdfTextSource::ZoteroCache);
        assert_eq!(r.format, PdfFormat::Plain);
        assert!(!r.completeness.complete);
    }

    #[tokio::test]
    async fn docling_failure_status_falls_through_to_flat_chain() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let server = MockServer::start().await;
        mount_health(&server, 200).await;
        Mock::given(method("POST"))
            .and(path("/v1/convert/file"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "document": null,
                "status": "failure",
                "errors": []
            })))
            .mount(&server)
            .await;

        let engines = engines_with_docling(
            &server.uri(),
            StubEngine::ok("flat text from the stub primary engine"),
            FallbackState::Ready(StubEngine::never()),
        );
        let r = extract(&pdf, dir.path(), &engines, false).await.unwrap();

        assert_eq!(r.source, PdfTextSource::LiveExtract);
        assert_eq!(r.format, PdfFormat::Plain);
        assert!(!r.completeness.complete);
        assert!(r
            .completeness
            .notes
            .iter()
            .any(|n| n.contains("flat-text engine")));
    }

    // --- OCR pre-step (real fixtures; docling stubbed via wiremock) ---

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    // --- Total page count, slicing, windowing, and the large-doc guard ---

    #[tokio::test]
    async fn total_page_count_reports_document_length() {
        // multipage.pdf is a genuine three-page document.
        assert_eq!(total_page_count(&fixture("multipage.pdf")).await, 3);
    }

    #[tokio::test]
    async fn total_page_count_is_zero_for_unparseable_pdf() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("junk.pdf");
        std::fs::write(&p, b"not a pdf").unwrap();
        // lopdf and pdfinfo both fail: unknown count is reported as 0, not a panic.
        assert_eq!(total_page_count(&p).await, 0);
    }

    #[tokio::test]
    async fn slice_pages_keeps_only_the_requested_window() {
        // Slice the middle page of the three-page document; the slice is a
        // one-page PDF and the original is untouched.
        let src = fixture("multipage.pdf");
        let before = std::fs::read(&src).unwrap();
        let slice = slice_pages(&src, 2, 2, 3).await.expect("slice succeeds");
        assert_eq!(
            total_page_count(slice.path()).await,
            1,
            "a 2..=2 window of a 3-page doc must be a single page"
        );
        assert_eq!(
            std::fs::read(&src).unwrap(),
            before,
            "slicing must not mutate the original file"
        );
    }

    fn engines_with_threshold(
        primary: Arc<dyn PdfEngine>,
        fallback: FallbackState,
        whole_document_max_pages: u32,
    ) -> PdfEngines {
        PdfEngines {
            docling: None,
            ocrmypdf: None,
            primary,
            fallback,
            whole_document_max_pages,
        }
    }

    #[tokio::test]
    async fn large_whole_document_request_is_refused_with_the_windowing_remedy() {
        // A whole-document request over a doc past the page ceiling must be a
        // loud PdfDocumentTooLarge — never a silent timeout or empty success.
        let engines = engines_with_threshold(
            StubEngine::never(), // guard fires before any engine is touched
            FallbackState::Disabled,
            2,
        );
        let dir = TempDir::new().unwrap();
        let err = extract(&fixture("multipage.pdf"), dir.path(), &engines, false)
            .await
            .expect_err("3-page doc over a 2-page ceiling must be refused");
        match err {
            Error::PdfDocumentTooLarge {
                pages, threshold, ..
            } => {
                assert_eq!(pages, 3);
                assert_eq!(threshold, 2);
            }
            other => panic!("expected PdfDocumentTooLarge, got {other:?}"),
        }
        assert!(
            err_names_windows(&engines, &fixture("multipage.pdf"), dir.path()).await,
            "the error message must direct the caller to page windows"
        );
    }

    async fn err_names_windows(engines: &PdfEngines, pdf: &Path, dir: &Path) -> bool {
        extract(pdf, dir, engines, false)
            .await
            .err()
            .map(|e| {
                let m = e.to_string();
                m.contains("window") && m.contains("page range")
            })
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn windowed_request_bypasses_the_guard_and_reports_total_pages() {
        // The same over-ceiling document is readable via a window: the guard
        // is window-aware, and total_pages reflects the whole document.
        let engines = engines_with_threshold(
            StubEngine::ok("page one body text well over the floor"),
            FallbackState::Disabled,
            2,
        );
        let dir = TempDir::new().unwrap();
        let r = extract_windowed(
            &fixture("multipage.pdf"),
            dir.path(),
            &engines,
            false,
            Some((1, 1)),
        )
        .await
        .expect("a window of an over-ceiling doc must succeed");
        assert_eq!(r.source, PdfTextSource::LiveExtract);
        assert_eq!(
            r.completeness.total_pages, 3,
            "total_pages describes the whole document, not the window"
        );
        assert!(
            r.completeness
                .notes
                .iter()
                .any(|n| n.contains("page window")),
            "a windowed result must note that it describes only the window"
        );
    }

    #[tokio::test]
    async fn whole_document_result_reports_total_pages() {
        let engines = engines_with_threshold(
            StubEngine::ok("some real extracted body text over the floor"),
            FallbackState::Disabled,
            50,
        );
        let dir = TempDir::new().unwrap();
        let r = extract(&fixture("multipage.pdf"), dir.path(), &engines, false)
            .await
            .expect("whole-document extraction under the ceiling succeeds");
        assert_eq!(r.completeness.total_pages, 3);
    }

    #[tokio::test]
    async fn probe_reports_absent_for_scanned_fixture() {
        assert!(
            matches!(
                probe_text_layer(&fixture("scanned.pdf")).await,
                TextLayer::Absent
            ),
            "image-only scanned.pdf must probe as Absent (a definite scan)"
        );
    }

    #[tokio::test]
    async fn probe_reports_present_for_hello_fixture() {
        assert!(
            matches!(
                probe_text_layer(&fixture("hello.pdf")).await,
                TextLayer::Present
            ),
            "hello.pdf carries real text and must probe as Present"
        );
    }

    #[tokio::test]
    async fn probe_error_is_unknown_not_absent() {
        // An unparseable PDF is ambiguous: the pre-step must not pre-empt
        // Docling with OCR for it, but the last-ditch rescue may still try.
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("junk.pdf");
        std::fs::write(&p, b"this is not a pdf").unwrap();
        assert!(
            matches!(probe_text_layer(&p).await, TextLayer::Unknown),
            "an unparseable PDF must probe as Unknown"
        );
    }

    async fn mount_convert_md(server: &MockServer, md: &str) {
        Mock::given(method("POST"))
            .and(path("/v1/convert/file"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "document": { "md_content": md },
                "status": "success",
                "errors": []
            })))
            .mount(server)
            .await;
    }

    fn engines_with_docling_and_ocr(url: &str, ocrmypdf: Option<PathBuf>) -> PdfEngines {
        PdfEngines {
            docling: Some(Arc::new(DoclingEngine::new(
                url.to_string(),
                Duration::from_secs(30),
                Duration::from_secs(2),
            ))),
            ocrmypdf,
            primary: StubEngine::never(),
            fallback: FallbackState::Disabled,
            whole_document_max_pages: 50,
        }
    }

    #[tokio::test]
    async fn ocr_prestep_labels_result_and_populates_ocr_pages() {
        // Real ocrmypdf on the scanned fixture; Docling stubbed. Skips
        // loudly only on hosts without ocrmypdf.
        let Ok(bin) = which::which("ocrmypdf") else {
            eprintln!("ocrmypdf not on PATH; skipping OCR pre-step test");
            return;
        };
        let scanned = fixture("scanned.pdf");
        let original_bytes = std::fs::read(&scanned).unwrap();

        let dir = TempDir::new().unwrap();
        let server = MockServer::start().await;
        mount_health(&server, 200).await;
        mount_convert_md(&server, "Scanned quarterly report recovered by OCR.").await;

        let engines = engines_with_docling_and_ocr(&server.uri(), Some(bin));
        let r = extract(&scanned, dir.path(), &engines, false)
            .await
            .unwrap();

        assert_eq!(r.source, PdfTextSource::OcrThenDocling);
        assert_eq!(r.format, PdfFormat::Markdown);
        assert_eq!(r.completeness.engine, PdfTextSource::OcrThenDocling);
        assert_eq!(r.completeness.ocr_pages, vec![1]);
        assert!(r.text.contains("Scanned quarterly report"));
        // The original scan is never mutated.
        assert_eq!(
            std::fs::read(&scanned).unwrap(),
            original_bytes,
            "scanned.pdf must be byte-identical after extraction"
        );
    }

    #[tokio::test]
    async fn bogus_ocrmypdf_path_degrades_gracefully_with_note() {
        let dir = TempDir::new().unwrap();
        let server = MockServer::start().await;
        mount_health(&server, 200).await;
        // Docling still converts the un-OCR'd scan (its own server-side OCR).
        mount_convert_md(&server, "Server-side recovered text.").await;

        let engines = engines_with_docling_and_ocr(
            &server.uri(),
            Some(PathBuf::from("/nonexistent/ocrmypdf-bogus")),
        );
        let r = extract(&fixture("scanned.pdf"), dir.path(), &engines, false)
            .await
            .unwrap();

        assert_eq!(r.source, PdfTextSource::Docling);
        assert!(!r.completeness.complete);
        assert!(r.completeness.ocr_pages.is_empty());
        assert!(
            r.completeness
                .notes
                .iter()
                .any(|n| n.contains("OCR pre-step failed")),
            "notes must record the failed OCR pre-step: {:?}",
            r.completeness.notes
        );
    }

    #[tokio::test]
    async fn missing_ocrmypdf_degrades_gracefully_with_note() {
        let dir = TempDir::new().unwrap();
        let server = MockServer::start().await;
        mount_health(&server, 200).await;
        mount_convert_md(&server, "Server-side recovered text.").await;

        let engines = engines_with_docling_and_ocr(&server.uri(), None);
        let r = extract(&fixture("scanned.pdf"), dir.path(), &engines, false)
            .await
            .unwrap();

        assert_eq!(r.source, PdfTextSource::Docling);
        assert!(!r.completeness.complete);
        assert!(r.completeness.ocr_pages.is_empty());
        assert!(
            r.completeness
                .notes
                .iter()
                .any(|n| n.contains("ocrmypdf is not available")),
            "notes must record the skipped OCR pre-step: {:?}",
            r.completeness.notes
        );
    }

    // --- plain option (forces the flat path; Docling must not be touched) ---

    #[tokio::test]
    async fn plain_true_skips_docling_and_uses_flat_path() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let server = MockServer::start().await;
        // Neither the health check nor the convert endpoint may be hit;
        // the expectations are verified when the MockServer drops.
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/convert/file"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let engines = engines_with_docling(
            &server.uri(),
            StubEngine::ok("flat text from the stub primary engine"),
            FallbackState::Ready(StubEngine::never()),
        );
        let r = extract(&pdf, dir.path(), &engines, true).await.unwrap();

        assert_eq!(r.source, PdfTextSource::LiveExtract);
        assert_eq!(r.format, PdfFormat::Plain);
        assert!(!r.page_anchors);
        assert_eq!(r.text, "flat text from the stub primary engine");
        assert!(!r.completeness.complete);
        assert!(r
            .completeness
            .notes
            .iter()
            .any(|n| n.contains("flat-text engine")));
    }

    #[tokio::test]
    async fn plain_true_all_engines_failed_is_still_a_loud_error() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let server = MockServer::start().await;
        mount_health(&server, 200).await;

        let engines = engines_with_docling(
            &server.uri(),
            StubEngine::fail("primary boom"),
            FallbackState::Ready(StubEngine::fail("pdftotext boom")),
        );
        let err = extract(&pdf, dir.path(), &engines, true).await.unwrap_err();

        assert!(matches!(err, Error::PdfAllEnginesFailed { .. }));
    }

    #[tokio::test]
    async fn docling_reported_errors_fall_through_to_flat_chain() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let server = MockServer::start().await;
        mount_health(&server, 200).await;
        Mock::given(method("POST"))
            .and(path("/v1/convert/file"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "document": { "md_content": "ignored" },
                "status": "success",
                "errors": ["page 3 exploded"]
            })))
            .mount(&server)
            .await;

        let engines = engines_with_docling(
            &server.uri(),
            StubEngine::ok("flat text from the stub primary engine"),
            FallbackState::Ready(StubEngine::never()),
        );
        let r = extract(&pdf, dir.path(), &engines, false).await.unwrap();

        assert_eq!(r.source, PdfTextSource::LiveExtract);
        assert!(!r.completeness.complete);
    }
}
