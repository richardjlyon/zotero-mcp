//! Durable store for layout-faithful PDF text derivatives.
//!
//! Extraction is expensive (a Docling convert over a tailnet, sometimes an
//! OCR pre-step first) and its output used to live only in the calling
//! model's context: every reader re-extracted, and a figure quoted in one
//! session could not be checked against an artefact later. This module keeps
//! the markdown once, keyed to the *content* of the source PDF and to the
//! extraction profile that produced it, and serves it to every later reader.
//!
//! **Why the store is not in the Zotero library.** Writes go to
//! `api.zotero.org` while reads come from the local SQLite database, so a
//! derivative written as a Zotero child attachment would be invisible to this
//! server's own reads until Zotero desktop synced it back down — an interval
//! we neither control nor observe. A sidecar under `~/Zotero/storage` fails
//! for a different reason: on the VM that directory is a read-only Resilio
//! mirror. So the store is server-owned and lives outside the Zotero tree,
//! which costs Zotero-UI visibility and cross-host sharing and buys working
//! identically on every host.
//!
//! **Only layout-faithful output is ever stored** (enforced by the caller in
//! [`crate::core::pdf`]): a flat-text run cannot express tables, so storing
//! one would make a lossy artefact permanent and let a cold-service accident
//! outlive itself.

use crate::core::error::{Error, Result};
use crate::core::pdf::{Completeness, PdfFormat, PdfTextSource};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;

/// Identifier for the extraction behaviour that produced a derivative.
///
/// Part of the store key, so bumping it invalidates every derivative built
/// under the previous behaviour. **Bump this by hand whenever a change to
/// extraction would alter the bytes of the output** — route order, OCR
/// policy, formula enrichment, the page-anchor format. It lives here rather
/// than being derived from the remote service because `DoclingEngine` exposes
/// only a liveness probe: the model version behind the endpoint is not
/// observable, and pretending otherwise would make staleness untestable.
pub const EXTRACTION_PROFILE: &str = "docling-md-anchors-v1";

/// Bytes read per hashing chunk. PDFs run to hundreds of megabytes; the file
/// is streamed rather than loaded so hashing a large scan costs no more
/// memory than hashing a small paper.
const HASH_CHUNK: usize = 64 * 1024;

/// What the store holds for a given (attachment, content, profile) triple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DerivativeStatus {
    /// A complete derivative is present and current.
    Present,
    /// A build was attempted and failed; the reason is recorded so a caller
    /// is told "extraction failed" rather than "not extracted yet".
    Failed(String),
    /// Nothing recorded for this triple.
    Absent,
}

/// Sidecar recorded next to every stored derivative.
///
/// Carries the provenance of the *stored bytes* — the engine and profile that
/// actually produced them, not the engine that would run now. A derivative
/// served today may have been built days ago; reporting the current route
/// would be a lie about the past.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DerivativeMeta {
    /// Zotero attachment key the derivative belongs to.
    pub attachment_key: String,
    /// Hex SHA-256 of the source PDF's bytes at build time.
    pub source_hash: String,
    /// [`EXTRACTION_PROFILE`] as it stood when the derivative was built.
    pub profile: String,
    /// The engine that produced the stored bytes.
    pub engine: PdfTextSource,
    pub format: PdfFormat,
    pub page_anchors: bool,
    pub character_count: usize,
    /// Completeness report describing the stored bytes.
    pub completeness: Completeness,
    /// The page windows walked to build it, in page order. A single-window
    /// entry means the document extracted whole.
    pub windows: Vec<(u32, u32)>,
    /// RFC 3339 build timestamp, best-effort (empty when the clock is
    /// unavailable). Informational only — never part of the staleness key.
    pub built_at: String,
}

/// A stored derivative: its bytes and their provenance.
#[derive(Debug, Clone)]
pub struct StoredDerivative {
    pub text: String,
    pub meta: DerivativeMeta,
}

/// Record written when a build fails, so a later caller can distinguish
/// "extraction failed on pages 41–60" from "nobody has tried yet".
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FailureRecord {
    reason: String,
    /// Page window that could not be extracted, when the failure was localised
    /// to one window of a walk.
    failed_window: Option<(u32, u32)>,
    at: String,
}

/// Filesystem-backed derivative store rooted at a server-owned directory.
#[derive(Debug, Clone)]
pub struct DerivativeStore {
    root: PathBuf,
}

impl DerivativeStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Stable store key for a source PDF: attachment key, the first 16 hex
    /// digits of the content hash, and the profile. Any change to the PDF's
    /// bytes or to the profile yields a different key, which *is* the
    /// staleness rule — there is no separate invalidation step, and a stale
    /// derivative is simply never looked up.
    fn key(attachment_key: &str, source_hash: &str) -> String {
        let short = &source_hash[..source_hash.len().min(16)];
        format!("{attachment_key}-{short}-{EXTRACTION_PROFILE}")
    }

    fn md_path(&self, attachment_key: &str, source_hash: &str) -> PathBuf {
        self.root
            .join(format!("{}.md", Self::key(attachment_key, source_hash)))
    }

    fn meta_path(&self, attachment_key: &str, source_hash: &str) -> PathBuf {
        self.root
            .join(format!("{}.json", Self::key(attachment_key, source_hash)))
    }

    fn failure_path(&self, attachment_key: &str, source_hash: &str) -> PathBuf {
        self.root
            .join(format!("{}.failed", Self::key(attachment_key, source_hash)))
    }

    /// Hex SHA-256 of a file's bytes, streamed.
    ///
    /// Content, not mtime: Resilio rewrites timestamps on sync, so mtime
    /// would invalidate derivatives at random on the mirror host.
    pub async fn content_hash(path: &Path) -> Result<String> {
        let mut f = tokio::fs::File::open(path).await.map_err(|e| {
            Error::DerivativeStore(format!("hash: cannot open {}: {e}", path.display()))
        })?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; HASH_CHUNK];
        loop {
            let n = f.read(&mut buf).await.map_err(|e| {
                Error::DerivativeStore(format!("hash: read {}: {e}", path.display()))
            })?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hex(&hasher.finalize()))
    }

    /// Fetch a current derivative, or `None` when the triple is not stored.
    /// A missing or unreadable sidecar means the entry is not usable, so it
    /// reads as absent rather than as a derivative of unknown provenance.
    pub async fn get(&self, attachment_key: &str, source_hash: &str) -> Option<StoredDerivative> {
        let text = tokio::fs::read_to_string(self.md_path(attachment_key, source_hash))
            .await
            .ok()?;
        let raw = tokio::fs::read_to_string(self.meta_path(attachment_key, source_hash))
            .await
            .ok()?;
        let meta: DerivativeMeta = serde_json::from_str(&raw).ok()?;
        Some(StoredDerivative { text, meta })
    }

    pub async fn status(&self, attachment_key: &str, source_hash: &str) -> DerivativeStatus {
        if self.get(attachment_key, source_hash).await.is_some() {
            return DerivativeStatus::Present;
        }
        if let Ok(raw) =
            tokio::fs::read_to_string(self.failure_path(attachment_key, source_hash)).await
        {
            if let Ok(rec) = serde_json::from_str::<FailureRecord>(&raw) {
                return DerivativeStatus::Failed(rec.reason);
            }
        }
        DerivativeStatus::Absent
    }

    /// Path a caller can hand to another tool or a person. Returned whether
    /// or not the file exists yet, so the caller can pair it with
    /// [`Self::status`]; it is always a path on *this* machine.
    pub fn path_for(&self, attachment_key: &str, source_hash: &str) -> PathBuf {
        self.md_path(attachment_key, source_hash)
    }

    /// Store a derivative and its sidecar.
    ///
    /// Both files are written to temporaries and renamed into place, so a
    /// process killed mid-write leaves no half-file that a later reader would
    /// serve as a whole document. The markdown lands *before* the sidecar and
    /// [`Self::get`] requires both, so the entry becomes visible only once it
    /// is complete.
    pub async fn put(
        &self,
        attachment_key: &str,
        source_hash: &str,
        text: &str,
        meta: &DerivativeMeta,
    ) -> Result<PathBuf> {
        tokio::fs::create_dir_all(&self.root).await.map_err(|e| {
            Error::DerivativeStore(format!("cannot create {}: {e}", self.root.display()))
        })?;
        let md = self.md_path(attachment_key, source_hash);
        write_atomic(&md, text.as_bytes()).await?;
        let json = serde_json::to_vec_pretty(meta)
            .map_err(|e| Error::DerivativeStore(format!("encode sidecar: {e}")))?;
        write_atomic(&self.meta_path(attachment_key, source_hash), &json).await?;
        // A successful build supersedes any recorded failure for the same
        // triple; leaving it would report a stale "extraction failed".
        let _ = tokio::fs::remove_file(self.failure_path(attachment_key, source_hash)).await;
        Ok(md)
    }

    /// Record that a build failed, naming the window that could not be
    /// extracted. Best-effort: a store that cannot be written must not turn
    /// an extraction failure into a second, more confusing failure.
    pub async fn record_failure(
        &self,
        attachment_key: &str,
        source_hash: &str,
        reason: &str,
        failed_window: Option<(u32, u32)>,
    ) {
        if tokio::fs::create_dir_all(&self.root).await.is_err() {
            return;
        }
        let rec = FailureRecord {
            reason: reason.to_string(),
            failed_window,
            at: now_rfc3339(),
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(&rec) {
            let _ = write_atomic(self.failure_path(attachment_key, source_hash), &bytes).await;
        }
    }
}

/// Write via temp file + rename so an interrupted write is never observable
/// as a short file. Mirrors `write_cache_atomic` in `core::pdf`.
async fn write_atomic(path: impl AsRef<Path>, bytes: &[u8]) -> Result<()> {
    let path = path.as_ref();
    let dir = path.parent().unwrap_or(Path::new("."));
    tokio::fs::create_dir_all(dir).await.map_err(|e| {
        Error::DerivativeStore(format!("cannot create {}: {e}", dir.display()))
    })?;
    let tmp = dir.join(format!(
        ".{}.tmp.{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|e| Error::DerivativeStore(format!("write {}: {e}", tmp.display())))?;
    tokio::fs::rename(&tmp, path).await.map_err(|e| {
        Error::DerivativeStore(format!("rename into {}: {e}", path.display()))
    })?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Best-effort RFC 3339 timestamp without pulling in a date library: seconds
/// since the epoch is enough for a human reading a sidecar, and nothing
/// depends on it.
fn now_rfc3339() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => format!("{}s-since-epoch", d.as_secs()),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(hash: &str) -> DerivativeMeta {
        DerivativeMeta {
            attachment_key: "AAAA0001".into(),
            source_hash: hash.into(),
            profile: EXTRACTION_PROFILE.into(),
            engine: PdfTextSource::Docling,
            format: PdfFormat::Markdown,
            page_anchors: true,
            character_count: 12,
            completeness: Completeness::flat_text(PdfTextSource::Docling),
            windows: vec![(1, 3)],
            built_at: now_rfc3339(),
        }
    }

    #[tokio::test]
    async fn content_hash_is_stable_and_content_sensitive() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.pdf");
        let b = dir.path().join("b.pdf");
        tokio::fs::write(&a, b"same bytes").await.unwrap();
        tokio::fs::write(&b, b"same bytes").await.unwrap();
        let ha = DerivativeStore::content_hash(&a).await.unwrap();
        let hb = DerivativeStore::content_hash(&b).await.unwrap();
        assert_eq!(ha, hb, "identical bytes must hash equal");
        assert_eq!(ha.len(), 64, "sha-256 renders as 64 hex digits");

        tokio::fs::write(&b, b"same bytee").await.unwrap();
        let hb2 = DerivativeStore::content_hash(&b).await.unwrap();
        assert_ne!(ha, hb2, "a changed byte must change the hash");
    }

    #[tokio::test]
    async fn content_hash_streams_larger_than_one_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.pdf");
        let bytes = vec![7u8; HASH_CHUNK * 3 + 11];
        tokio::fs::write(&big, &bytes).await.unwrap();
        let h = DerivativeStore::content_hash(&big).await.unwrap();
        // Same content written in one go must hash the same as the streamed
        // read of it — proves the chunk loop feeds the hasher in order.
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        assert_eq!(h, hex(&hasher.finalize()));
    }

    #[tokio::test]
    async fn put_then_get_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = DerivativeStore::new(dir.path().join("derivatives"));
        let m = meta("deadbeefdeadbeefcafe");
        store
            .put("AAAA0001", &m.source_hash, "# Title\n\n| a | b |\n", &m)
            .await
            .unwrap();

        let got = store.get("AAAA0001", &m.source_hash).await.unwrap();
        assert_eq!(got.text, "# Title\n\n| a | b |\n");
        assert_eq!(got.meta, m);
        assert_eq!(
            store.status("AAAA0001", &m.source_hash).await,
            DerivativeStatus::Present
        );
    }

    #[tokio::test]
    async fn a_different_content_hash_is_a_different_entry() {
        let dir = tempfile::tempdir().unwrap();
        let store = DerivativeStore::new(dir.path());
        let m = meta("1111111111111111");
        store.put("AAAA0001", &m.source_hash, "old", &m).await.unwrap();

        // The PDF was replaced: same attachment key, different content hash.
        assert!(store.get("AAAA0001", "2222222222222222").await.is_none());
        assert_eq!(
            store.status("AAAA0001", "2222222222222222").await,
            DerivativeStatus::Absent,
            "a replaced PDF must read as absent, never as the old derivative"
        );
    }

    #[tokio::test]
    async fn missing_sidecar_reads_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let store = DerivativeStore::new(dir.path());
        let m = meta("3333333333333333");
        store.put("AAAA0001", &m.source_hash, "text", &m).await.unwrap();
        tokio::fs::remove_file(store.meta_path("AAAA0001", &m.source_hash))
            .await
            .unwrap();
        assert!(
            store.get("AAAA0001", &m.source_hash).await.is_none(),
            "bytes without provenance must not be served"
        );
    }

    #[tokio::test]
    async fn failure_is_distinguishable_from_never_tried() {
        let dir = tempfile::tempdir().unwrap();
        let store = DerivativeStore::new(dir.path());
        assert_eq!(store.status("BBBB0002", "abc").await, DerivativeStatus::Absent);

        store
            .record_failure("BBBB0002", "abc", "window 41..=60 failed: docling timeout", Some((41, 60)))
            .await;
        match store.status("BBBB0002", "abc").await {
            DerivativeStatus::Failed(r) => assert!(r.contains("41..=60")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_successful_build_clears_a_recorded_failure() {
        let dir = tempfile::tempdir().unwrap();
        let store = DerivativeStore::new(dir.path());
        let m = meta("4444444444444444");
        store
            .record_failure("AAAA0001", &m.source_hash, "transient", None)
            .await;
        store.put("AAAA0001", &m.source_hash, "text", &m).await.unwrap();
        assert_eq!(
            store.status("AAAA0001", &m.source_hash).await,
            DerivativeStatus::Present
        );
    }

    #[tokio::test]
    async fn store_works_when_zotero_dir_is_read_only() {
        // The VM reads a read-only Resilio mirror of ~/Zotero. The store must
        // not be under it; this asserts the store only ever touches its own
        // root, by pointing the root somewhere writable while the "Zotero
        // dir" beside it is not writable at all.
        let dir = tempfile::tempdir().unwrap();
        let zotero = dir.path().join("zotero");
        std::fs::create_dir_all(&zotero).unwrap();
        let mut perms = std::fs::metadata(&zotero).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&zotero, perms).unwrap();

        let store = DerivativeStore::new(dir.path().join("state/derivatives"));
        let m = meta("5555555555555555");
        store
            .put("AAAA0001", &m.source_hash, "built anyway", &m)
            .await
            .expect("store root is outside the read-only tree");
        assert!(store.get("AAAA0001", &m.source_hash).await.is_some());

        let mut perms = std::fs::metadata(&zotero).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&zotero, perms).unwrap();
    }

    #[test]
    fn key_includes_profile_so_a_bump_invalidates() {
        let k = DerivativeStore::key("AAAA0001", "abcdef0123456789ffff");
        assert!(k.starts_with("AAAA0001-abcdef0123456789-"));
        assert!(k.ends_with(EXTRACTION_PROFILE));
    }
}
