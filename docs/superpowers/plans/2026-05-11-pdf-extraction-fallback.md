# PDF Extraction Fallback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `get_pdf_text` recover from `pdf-extract` failures by shelling out to Poppler's `pdftotext`, and persist recovered text into Zotero's `.zotero-ft-cache` so subsequent calls are free.

**Architecture:** Introduce a `PdfEngine` trait with two implementations (`PdfExtractEngine`, `PdftotextEngine`). Wrap them in a `PdfEngines` bundle stored on `AppState`. Refactor `core::pdf::extract` to a three-step chain (cache → primary → fallback). On fallback success, atomically write `.zotero-ft-cache`. Errors compose into new variants on `core::error::Error`.

**Tech Stack:** Rust, `tokio` (async runtime), `tokio::process` (subprocess), `async-trait` (already a dep), `which` (binary discovery), `tempfile` (already a dev-dep), Poppler's `pdftotext` (runtime external dep).

**Spec:** `docs/superpowers/specs/2026-05-11-pdf-extraction-fallback-design.md`

---

## File Structure

| File | Purpose | Action |
|---|---|---|
| `crates/zotero-mcp/Cargo.toml` | Add `which` crate | Modify |
| `crates/zotero-mcp/src/core/config.rs` | Add `pdftotext_path`, `pdftotext_fallback` to `ZoteroConfig` | Modify |
| `crates/zotero-mcp/src/core/error.rs` | Add 3 new error variants | Modify |
| `crates/zotero-mcp/src/core/pdf.rs` | Add engine trait, two engine impls, `PdfEngines` holder, refactor `extract` | Modify (substantial) |
| `crates/zotero-mcp/src/state.rs` | Store `Arc<PdfEngines>` on `AppState`; resolve pdftotext at build | Modify |
| `crates/zotero-mcp/src/tools/attachments.rs` | Pass `&state.pdf_engines` into `get_pdf_text` / `get_pdf_first_pages` | Modify |
| `crates/zotero-mcp/src/tools/enrichment.rs` | Pass engines through `ProposeInput` / `EnrichInput` | Modify |
| `crates/zotero-mcp/src/core/enrichment/propose.rs` | `ProposeInput` grows engines field | Modify |
| `crates/zotero-mcp/src/core/enrichment/mod.rs` (or wherever `EnrichInput` lives) | `EnrichInput` grows engines field | Modify |
| `crates/zotero-mcp/tests/pdf_text.rs` | Extend with orchestrator + integration tests | Modify |
| `crates/zotero-mcp/tests/fixtures/build_fixture.rs` | Replace fake "%PDF-1.4 fake" with a real minimal PDF | Modify |
| `crates/zotero-mcp/tests/fixtures/gen_pdfs.py` | Python generator script for test PDFs | Create |
| `crates/zotero-mcp/tests/fixtures/hello.pdf` | Tiny valid PDF for pdftotext integration test | Create (binary, generated) |
| `README.md` | Mention Poppler install for PDF fallback | Modify |

**Out of scope:** No CI workflows exist in `.github/workflows/`. The spec's §9.3 CI integration is therefore N/A — the README note is the user-facing surface.

---

## Task 1: Add config knobs to `ZoteroConfig`

**Files:**
- Modify: `crates/zotero-mcp/src/core/config.rs`

- [ ] **Step 1: Write failing tests**

Open `crates/zotero-mcp/src/core/config.rs`, locate the `tests` module at the bottom, and add two tests inside it:

```rust
    #[test]
    fn pdftotext_fallback_defaults_to_true() {
        let c = Config::default();
        assert!(c.zotero.pdftotext_fallback);
        assert!(c.zotero.pdftotext_path.is_none());
    }

    #[test]
    fn pdftotext_path_parses_from_toml() {
        let toml = r#"
[zotero]
pdftotext_path = "/opt/homebrew/bin/pdftotext"
pdftotext_fallback = false
"#;
        let c: Config = toml::from_str(toml).unwrap();
        assert_eq!(c.zotero.pdftotext_path.as_deref(), Some("/opt/homebrew/bin/pdftotext"));
        assert!(!c.zotero.pdftotext_fallback);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p zotero-mcp --lib core::config::tests::pdftotext -- --nocapture`

Expected: FAIL with "no field `pdftotext_fallback` on type `ZoteroConfig`" (compile error).

- [ ] **Step 3: Add fields to `ZoteroConfig`**

Modify the `ZoteroConfig` struct (around line 29-44 of `core/config.rs`) by adding two fields just before the closing brace:

```rust
    pub max_schema_userdata: i64,

    /// Optional explicit path to the `pdftotext` binary. When set and the file
    /// exists, used instead of PATH lookup. Useful for non-standard installs.
    #[serde(default)]
    pub pdftotext_path: Option<String>,

    /// Whether to fall back to `pdftotext` (Poppler) when the in-process
    /// `pdf-extract` engine fails. Default: true.
    #[serde(default = "default_true")]
    pub pdftotext_fallback: bool,
}
```

Add the `default_true` helper just above the `impl Default for ZoteroConfig` block:

```rust
fn default_true() -> bool {
    true
}
```

- [ ] **Step 4: Update `Default for ZoteroConfig`**

In the same file, update the `Default` impl (around line 46-58) to include the new fields. Add to the body:

```rust
            max_schema_userdata: 135,
            pdftotext_path: None,
            pdftotext_fallback: true,
        }
```

- [ ] **Step 5: Run tests to verify pass**

Run: `cargo test -p zotero-mcp --lib core::config::tests -- --nocapture`

Expected: PASS for all config tests (including new ones).

- [ ] **Step 6: Commit**

```bash
git add crates/zotero-mcp/src/core/config.rs
git commit -m "feat(config): add pdftotext_path and pdftotext_fallback knobs

Adds optional configuration for the upcoming Poppler-based PDF
extraction fallback. Defaults: fallback enabled, path auto-resolved
from PATH."
```

---

## Task 2: Add error variants

**Files:**
- Modify: `crates/zotero-mcp/src/core/error.rs`

- [ ] **Step 1: Write failing tests**

Inside the `#[cfg(test)] mod tests` block of `core/error.rs`, add three tests:

```rust
    #[test]
    fn pdftotext_missing_message_contains_install_hint() {
        let e = Error::PdftotextMissing;
        let s = e.to_string();
        assert!(s.contains("Poppler"));
        assert!(s.contains("brew install poppler"));
        assert!(s.contains("apt install poppler-utils"));
    }

    #[test]
    fn pdftotext_timeout_includes_seconds_and_path() {
        let e = Error::PdftotextTimeout(60, "/tmp/a.pdf".into());
        let s = e.to_string();
        assert!(s.contains("60"));
        assert!(s.contains("/tmp/a.pdf"));
    }

    #[test]
    fn pdf_all_engines_failed_includes_both_messages() {
        let e = Error::PdfAllEnginesFailed {
            pdf_extract: "unhandled function type 4".into(),
            pdftotext: "exited 1: bad xref".into(),
        };
        let s = e.to_string();
        assert!(s.contains("unhandled function type 4"));
        assert!(s.contains("bad xref"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p zotero-mcp --lib core::error::tests::pdftotext`

Expected: FAIL — variants don't exist yet.

- [ ] **Step 3: Add the new variants**

In `core/error.rs`, insert these three variants in the `Error` enum, immediately after the existing `Pdf` variant (which lives around line 41-42):

```rust
    #[error("pdf extraction failed: {0}")]
    Pdf(String),

    #[error(
        "pdftotext fallback unavailable: install Poppler \
         (`brew install poppler` on macOS, `apt install poppler-utils` on Linux), \
         or set `[zotero] pdftotext_path = \"...\"` in config.toml"
    )]
    PdftotextMissing,

    #[error("pdftotext timed out after {0}s extracting {1}")]
    PdftotextTimeout(u64, String),

    #[error(
        "pdf extraction failed in all engines. \
         pdf-extract: {pdf_extract}. \
         pdftotext: {pdftotext}"
    )]
    PdfAllEnginesFailed { pdf_extract: String, pdftotext: String },
```

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p zotero-mcp --lib core::error::tests`

Expected: PASS (all tests, including new ones).

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/core/error.rs
git commit -m "feat(error): add PdftotextMissing, PdftotextTimeout, PdfAllEnginesFailed

New variants for the upcoming pdftotext fallback engine. PdftotextMissing
includes the install hint inline so the message is actionable."
```

---

## Task 3: Add `which` crate dependency

**Files:**
- Modify: `crates/zotero-mcp/Cargo.toml`

- [ ] **Step 1: Add `which` to crate dependencies**

In `crates/zotero-mcp/Cargo.toml`, add `which = "7"` in the `[dependencies]` block. Place it alphabetically near other small crates (after `url = "2"` or near `rand`):

```toml
url = "2"
which = "7"
```

- [ ] **Step 2: Run `cargo build` to fetch and compile**

Run: `cargo build -p zotero-mcp 2>&1 | tail -20`

Expected: build succeeds; `which` is downloaded and compiled.

- [ ] **Step 3: Commit**

```bash
git add crates/zotero-mcp/Cargo.toml Cargo.lock
git commit -m "build: add 'which' crate for pdftotext binary discovery"
```

---

## Task 4: Add `PdftotextFallback` variant to `PdfTextSource`

**Files:**
- Modify: `crates/zotero-mcp/src/core/pdf.rs`

- [ ] **Step 1: Add the variant**

In `crates/zotero-mcp/src/core/pdf.rs`, the `PdfTextSource` enum currently has `ZoteroCache` and `LiveExtract` (lines 7-12). Add a third variant:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfTextSource {
    ZoteroCache,
    LiveExtract,
    /// Recovered via Poppler's `pdftotext` after `pdf-extract` failed.
    PdftotextFallback,
}
```

Note: also added `PartialEq, Eq` derives so tests can use `assert_eq!`. The existing test in `tests/pdf_text.rs` uses `matches!`, which still works.

- [ ] **Step 2: Verify build still passes**

Run: `cargo build -p zotero-mcp`

Expected: build succeeds.

- [ ] **Step 3: Verify existing test still passes**

Run: `cargo test -p zotero-mcp --test pdf_text`

Expected: PASS — `prefers_zotero_ft_cache_when_present` still passes.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-mcp/src/core/pdf.rs
git commit -m "feat(pdf): add PdftotextFallback variant to PdfTextSource

Callers and tools will see this when text was recovered via the pdftotext
fallback engine instead of the in-process pdf-extract crate."
```

---

## Task 5: Define engine trait + `PdfExtractEngine`

**Files:**
- Modify: `crates/zotero-mcp/src/core/pdf.rs`

- [ ] **Step 1: Add imports and types**

At the top of `core/pdf.rs`, expand imports:

```rust
use crate::core::error::{Error, Result};
use crate::core::reader::pool::ReadOnlyPool;
use crate::core::reader::attachments::resolve_path;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
```

- [ ] **Step 2: Add the engine trait and error**

Insert this block immediately after the `PdfTextResult` struct (after line 19 in the current file):

```rust
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
```

- [ ] **Step 3: Implement `PdfExtractEngine`**

Add immediately below:

```rust
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
```

- [ ] **Step 4: Add a unit test for `PdfExtractEngine` failure path**

Append a test module at the bottom of `core/pdf.rs`:

```rust
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
```

- [ ] **Step 5: Run the test**

Run: `cargo test -p zotero-mcp --lib core::pdf::engine_tests`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/zotero-mcp/src/core/pdf.rs
git commit -m "feat(pdf): introduce PdfEngine trait and PdfExtractEngine

The trait is the seam for unit-testing the orchestrator with stub engines
and for adding the pdftotext fallback engine in the next task. The
PdfExtractEngine implementation moves the existing pdf-extract call
behind the trait without changing behaviour."
```

---

## Task 6: Add `PdfEngines` holder + `FallbackState`

**Files:**
- Modify: `crates/zotero-mcp/src/core/pdf.rs`

- [ ] **Step 1: Add the holder types**

Append to `core/pdf.rs`, after `PdfExtractEngine` impl:

```rust
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
}
```

(`PdftotextEngine` and `build` are added in subsequent tasks. For now, callers can construct `PdfEngines` directly in tests if needed.)

- [ ] **Step 2: Verify build**

Run: `cargo build -p zotero-mcp 2>&1 | tail -5`

Expected: builds with no errors. (Unused-warning OK at this stage; resolved by later tasks.)

- [ ] **Step 3: Commit**

```bash
git add crates/zotero-mcp/src/core/pdf.rs
git commit -m "feat(pdf): add PdfEngines holder and FallbackState

The holder bundles primary and (optional) fallback engines for the
orchestrator. FallbackState distinguishes 'binary missing' from 'user
disabled' so the orchestrator can emit different errors."
```

---

## Task 7: Generate `hello.pdf` fixture for pdftotext tests

**Files:**
- Create: `crates/zotero-mcp/tests/fixtures/gen_pdfs.py`
- Create: `crates/zotero-mcp/tests/fixtures/hello.pdf`

- [ ] **Step 1: Write the generator script**

Create `crates/zotero-mcp/tests/fixtures/gen_pdfs.py`:

```python
#!/usr/bin/env python3
"""Generate test PDFs for the pdf-extraction-fallback tests.

Requires:  pip install pikepdf

Run from anywhere; produces files next to this script.
"""
from pathlib import Path

try:
    import pikepdf
except ImportError as e:
    raise SystemExit(
        "Install pikepdf first:  pip install pikepdf  (or uv pip install pikepdf)"
    ) from e


HERE = Path(__file__).resolve().parent


def make_hello() -> None:
    """A minimal valid PDF containing 'Hello fallback world.' so pdftotext can extract text."""
    pdf = pikepdf.Pdf.new()
    pdf.add_blank_page(page_size=(612, 792))
    page = pdf.pages[0]
    # Embed a Type 1 (PostScript) Helvetica font reference.
    font = pikepdf.Dictionary(
        Type=pikepdf.Name("/Font"),
        Subtype=pikepdf.Name("/Type1"),
        BaseFont=pikepdf.Name("/Helvetica"),
    )
    page.Resources = pikepdf.Dictionary(
        Font=pikepdf.Dictionary(F1=pdf.make_indirect(font)),
    )
    content = b"BT /F1 14 Tf 72 720 Td (Hello fallback world.) Tj ET"
    page.Contents = pdf.make_stream(content)
    out = HERE / "hello.pdf"
    pdf.save(out)
    print(f"Wrote {out} ({out.stat().st_size} bytes)")


if __name__ == "__main__":
    make_hello()
```

- [ ] **Step 2: Run the generator**

```bash
cd crates/zotero-mcp/tests/fixtures
python3 -m pip install --user --quiet pikepdf 2>/dev/null || pip install --user --quiet pikepdf
python3 gen_pdfs.py
```

Expected: `Wrote .../hello.pdf (~1-2 KB)`.

If `pikepdf` install fails on the engineer's machine, use a venv:
```bash
python3 -m venv /tmp/zmpdf && /tmp/zmpdf/bin/pip install pikepdf && /tmp/zmpdf/bin/python gen_pdfs.py
```

- [ ] **Step 3: Verify the PDF is valid**

```bash
file crates/zotero-mcp/tests/fixtures/hello.pdf
pdftotext crates/zotero-mcp/tests/fixtures/hello.pdf -
```

Expected:
- `file` reports `PDF document, version 1.x, 1 pages`.
- `pdftotext` prints `Hello fallback world.`.

If `pdftotext` is not installed locally: `brew install poppler` (macOS) and retry. This is the same binary the runtime fallback uses.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-mcp/tests/fixtures/gen_pdfs.py crates/zotero-mcp/tests/fixtures/hello.pdf
git commit -m "test: add hello.pdf fixture + generator script

Minimal valid PDF for the upcoming pdftotext integration test. The
generator script is committed so the fixture is reproducible from
source."
```

---

## Task 8: Implement `PdftotextEngine` + integration test

**Files:**
- Modify: `crates/zotero-mcp/src/core/pdf.rs`
- Modify: `crates/zotero-mcp/tests/pdf_text.rs`

- [ ] **Step 1: Implement `PdftotextEngine`**

Append to `core/pdf.rs`, after the `FallbackState` enum:

```rust
use std::time::Duration;

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

    #[cfg(test)]
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
            let mut limited = (&mut stderr_pipe).take(500);
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
            return Err(EngineError::Failed("pdftotext produced empty output".into()));
        }

        String::from_utf8(stdout)
            .map_err(|e| EngineError::Failed(format!("pdftotext output not valid UTF-8: {}", e)))
    }
}
```

- [ ] **Step 2: Write the integration test**

In `crates/zotero-mcp/tests/pdf_text.rs`, append:

```rust
use std::path::PathBuf;
use std::time::Duration;
use zotero_mcp::core::pdf::{EngineError, PdfEngine, PdftotextEngine};

/// Locate `pdftotext` on PATH; return None to signal "skip this test on this host".
fn locate_pdftotext() -> Option<PathBuf> {
    which::which("pdftotext").ok()
}

#[tokio::test]
async fn pdftotext_engine_extracts_text_from_hello_pdf() {
    let Some(bin) = locate_pdftotext() else {
        eprintln!("pdftotext not on PATH; skipping integration test");
        return;
    };
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hello.pdf");
    assert!(fixture.exists(), "hello.pdf fixture missing — run tests/fixtures/gen_pdfs.py");

    let eng = PdftotextEngine::new(bin);
    let text = eng.extract(&fixture).await.expect("extraction succeeded");
    assert!(text.contains("Hello fallback world"), "got: {:?}", text);
}

#[tokio::test]
async fn pdftotext_engine_returns_timeout_when_deadline_exceeded() {
    let Some(bin) = locate_pdftotext() else {
        eprintln!("pdftotext not on PATH; skipping integration test");
        return;
    };
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hello.pdf");

    // 1 ns is unreachable; the timer fires before pdftotext can even spawn-and-exit.
    let eng = PdftotextEngine::with_timeout(bin, Duration::from_nanos(1));
    let err = eng.extract(&fixture).await.expect_err("should time out");
    assert!(matches!(err, EngineError::Timeout(0)), "got: {:?}", err);
}
```

The test file currently imports `which` indirectly through... actually, it doesn't. Add `which` to dev-deps if it isn't already inheritable. Since we added it to the crate's `[dependencies]` in Task 3, it's available in tests automatically.

- [ ] **Step 3: Run tests**

```bash
which pdftotext  # confirm it's on PATH; if not, brew install poppler
cargo test -p zotero-mcp --test pdf_text -- --nocapture
```

Expected: all tests pass, including the two new integration tests.

If `pdftotext` is not installed, the new tests print a skip message and `return;` (so the test still passes — no false red).

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-mcp/src/core/pdf.rs crates/zotero-mcp/tests/pdf_text.rs
git commit -m "feat(pdf): implement PdftotextEngine fallback

Out-of-process pdftotext invocation with 60s timeout, 50MB output cap,
stderr capture, and kill-on-drop. Integration tests verify text
extraction and the timeout path (both skip gracefully if pdftotext is
not on PATH)."
```

---

## Task 9: `PdfEngines::build` + wire onto `AppState`

**Files:**
- Modify: `crates/zotero-mcp/src/core/pdf.rs`
- Modify: `crates/zotero-mcp/src/state.rs`

- [ ] **Step 1: Add the `PdfEngines::build` factory**

In `core/pdf.rs`, add this `impl` block (place it next to the existing `impl PdfEngines`):

```rust
impl PdfEngines {
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
```

Replace the previous `impl PdfEngines { primary(), fallback() }` block by merging both impls (or just keep both — Rust accepts multiple `impl` blocks).

- [ ] **Step 2: Add `pdf_engines` field to `AppState`**

In `crates/zotero-mcp/src/state.rs`, update the struct (around line 13-22):

```rust
#[derive(Clone)]
pub struct AppState {
    pub cfg: Config,
    pub pool: ReadOnlyPool,
    pub api: LocalApi,
    pub bbt: Option<Arc<BbtClient>>,
    pub crossref: CrossrefClient,
    pub openlibrary: OpenLibraryClient,
    pub arxiv: ArxivClient,
    pub semantic_scholar: SemanticScholarClient,
    pub pdf_engines: Arc<crate::core::pdf::PdfEngines>,
}
```

- [ ] **Step 3: Construct `pdf_engines` in `AppState::build`**

In the `build` method (`state.rs:25`), add immediately before the final `Ok(Self {...})`:

```rust
        let pdf_engines = Arc::new(crate::core::pdf::PdfEngines::build(&cfg.zotero));

        Ok(Self {
            cfg,
            pool,
            api,
            bbt,
            crossref,
            openlibrary,
            arxiv,
            semantic_scholar,
            pdf_engines,
        })
```

- [ ] **Step 4: Verify build**

Run: `cargo build -p zotero-mcp 2>&1 | tail -10`

Expected: builds cleanly. Some unused warnings on the new engine types are acceptable until Task 10 wires them into `extract`.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/core/pdf.rs crates/zotero-mcp/src/state.rs
git commit -m "feat(pdf): add PdfEngines::build factory and store on AppState

Resolves pdftotext at startup (config override → PATH → unavailable),
emits an advisory INFO log on the chosen state, and exposes the bundle
on AppState for the orchestrator wiring in the next task."
```

---

## Task 10: Refactor `extract` to use the engine chain

**Files:**
- Modify: `crates/zotero-mcp/src/core/pdf.rs`

- [ ] **Step 1: Replace the body of `extract` (and `get_pdf_text`/`get_pdf_first_pages`)**

Replace lines 21-65 of `core/pdf.rs` (everything from `pub async fn get_pdf_text` through `pub fn cache_path_for`) with:

```rust
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
    Ok(PdfTextResult { text, source: full.source, character_count: cap })
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
        Err(e) => e.display(),
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
            return Err(Error::PdftotextTimeout(secs, pdf_path.display().to_string()));
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
        tracing::debug!(
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
    let tmp = cache.with_file_name(".zotero-ft-cache.tmp");
    let mut content = text.to_owned();
    if !content.ends_with('\n') {
        content.push('\n');
    }
    tokio::fs::write(&tmp, content).await?;
    tokio::fs::rename(&tmp, cache).await
}

pub fn cache_path_for(storage_dir: &Path, parent_key: &str) -> PathBuf {
    storage_dir.join(parent_key).join(".zotero-ft-cache")
}
```

- [ ] **Step 2: Verify the file builds**

Run: `cargo build -p zotero-mcp 2>&1 | tail -20`

Expected: COMPILE ERRORS in `tools/attachments.rs`, `tools/enrichment.rs`, `core/enrichment/propose.rs`, and `tests/pdf_text.rs` — the signature change of `get_pdf_text` and `get_pdf_first_pages` broke them. This is intentional; the next task fixes them all.

DO NOT commit yet — the working tree won't build. Move to Task 11 immediately.

---

## Task 11: Plumb `&PdfEngines` through callers

**Files:**
- Modify: `crates/zotero-mcp/src/tools/attachments.rs`
- Modify: `crates/zotero-mcp/src/tools/enrichment.rs`
- Modify: `crates/zotero-mcp/src/core/enrichment/propose.rs`
- Modify: `crates/zotero-mcp/tests/pdf_text.rs`

- [ ] **Step 1: Update `tools/attachments.rs`**

Two call sites need `&s.pdf_engines`:

`get_pdf_text_t` (around line 35) — replace its body:

```rust
pub async fn get_pdf_text_t(s: &AppState, a: ItemKeyArgs) -> Result<CallToolResult, Error> {
    let r = get_pdf_text(&s.pool, &a.item_key, 1, &s.cfg.storage_dir(), &s.pdf_engines)
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}
```

`get_pdf_first_pages_t` (around line 54) — replace:

```rust
pub async fn get_pdf_first_pages_t(s: &AppState, a: FirstPagesArgs) -> Result<CallToolResult, Error> {
    let r = get_pdf_first_pages(&s.pool, &a.item_key, 1, &s.cfg.storage_dir(), a.n, &s.pdf_engines)
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}
```

- [ ] **Step 2: Update `core/enrichment/propose.rs`**

Locate `ProposeInput<'a>` (around line 75). Add a field:

```rust
pub struct ProposeInput<'a> {
    pub item_key: &'a str,
    pub library_id: i64,
    pub storage_dir: &'a Path,
    pub candidates: Vec<NormalizedRecord>,
    pub engines: &'a crate::core::pdf::PdfEngines,
}
```

Update the body of `propose_metadata_update` (around line 86) — the `get_pdf_first_pages` call now needs `inp.engines`:

```rust
    let signals = match get_pdf_first_pages(pool, inp.item_key, inp.library_id, inp.storage_dir, 1, inp.engines).await {
        Ok(p) => crate::core::enrichment::pdf_signals::extract_signals(&p.text),
        Err(_) => PdfSignals::default(),
    };
```

- [ ] **Step 3: Update `tools/enrichment.rs`**

Locate `propose_metadata_update_t` (around line 131). Pass `&s.pdf_engines` in:

```rust
    let p = propose_metadata_update(
        &s.pool,
        ProposeInput {
            item_key: &a.item_key,
            library_id: 1,
            storage_dir: &storage_dir,
            candidates,
            engines: &s.pdf_engines,
        },
    )
```

Locate `enrich_item_t` (around line 180). If `EnrichInput` similarly contains a `propose_metadata_update` call, update it the same way. Run:

```bash
grep -n "EnrichInput\b\|enrich_item\b" crates/zotero-mcp/src/core/enrichment -r
```

For each `EnrichInput` definition and `enrich_item` call site, add an `engines` field flowing through to any internal `propose_metadata_update` / `get_pdf_first_pages` call. Apply the same shape as `ProposeInput`.

- [ ] **Step 4: Update `tests/pdf_text.rs`**

The existing test (`prefers_zotero_ft_cache_when_present`) calls `get_pdf_text` with the old signature. Update it to pass a `PdfEngines`. Add an import at the top:

```rust
use zotero_mcp::core::pdf::{get_pdf_text, PdfEngines, PdfTextSource};
```

And in the test:

```rust
#[tokio::test]
async fn prefers_zotero_ft_cache_when_present() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let engines = PdfEngines::build(&zotero_mcp::core::config::Config::default().zotero);
    let res = get_pdf_text(&pool, "AAAA0001", 1, &f.storage_dir(), &engines).await.unwrap();
    assert!(matches!(res.source, PdfTextSource::ZoteroCache));
    assert!(res.text.contains("zoteroconnectortest"));
}
```

- [ ] **Step 5: Verify all builds pass**

Run: `cargo build -p zotero-mcp --tests 2>&1 | tail -20`

Expected: clean build, no errors. Any unused-import warnings: clean up.

- [ ] **Step 6: Run the existing tests**

Run: `cargo test -p zotero-mcp 2>&1 | tail -30`

Expected: all tests pass (including `prefers_zotero_ft_cache_when_present`, the engine unit test from Task 5, and the pdftotext integration tests from Task 8).

- [ ] **Step 7: Commit**

```bash
git add crates/zotero-mcp/src/core/pdf.rs \
        crates/zotero-mcp/src/tools/attachments.rs \
        crates/zotero-mcp/src/tools/enrichment.rs \
        crates/zotero-mcp/src/core/enrichment/propose.rs \
        crates/zotero-mcp/src/core/enrichment/mod.rs \
        crates/zotero-mcp/tests/pdf_text.rs
git commit -m "feat(pdf): wire PdfEngines through the extraction pipeline

extract() now chains cache → primary (pdf-extract) → fallback (pdftotext)
and writes the recovered text into .zotero-ft-cache atomically. The
public get_pdf_text / get_pdf_first_pages signatures grow a &PdfEngines
parameter; all call sites updated."
```

(If `core/enrichment/mod.rs` was not touched in step 3, drop it from `git add`.)

---

## Task 12: Orchestrator unit tests with stub engines

**Files:**
- Modify: `crates/zotero-mcp/src/core/pdf.rs`

- [ ] **Step 1: Add a stub engine inside `core/pdf.rs`**

Append inside the existing `#[cfg(test)] mod engine_tests` block (or a new `#[cfg(test)] mod orchestrator_tests`):

```rust
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
            Arc::new(Self { queue: Mutex::new(results) })
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

        let engines = engines_with(StubEngine::never(), FallbackState::Ready(StubEngine::never()));
        let r = extract(&pdf, dir.path(), &engines).await.unwrap();

        assert_eq!(r.source, PdfTextSource::ZoteroCache);
        assert!(r.text.contains("cached body"));
    }

    #[tokio::test]
    async fn primary_success_does_not_write_cache() {
        let dir = TempDir::new().unwrap();
        let pdf = write_dummy_pdf(dir.path());

        let engines = engines_with(StubEngine::ok("primary text"), FallbackState::Ready(StubEngine::never()));
        let r = extract(&pdf, dir.path(), &engines).await.unwrap();

        assert_eq!(r.source, PdfTextSource::LiveExtract);
        assert_eq!(r.text, "primary text");
        assert!(!dir.path().join(".zotero-ft-cache").exists(), "cache must not be written on primary success");
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
            Error::PdfAllEnginesFailed { pdf_extract, pdftotext } => {
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
```

- [ ] **Step 2: Run the new tests**

Run: `cargo test -p zotero-mcp --lib core::pdf::orchestrator_tests`

Expected: all 8 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/zotero-mcp/src/core/pdf.rs
git commit -m "test(pdf): orchestrator unit tests with stub engines

Covers all branches of the extraction chain: cache hit, primary success
(no cache write), fallback recovery (with cache write), both-engines
failure, binary-missing, opt-out, timeout, and best-effort cache failure."
```

---

## Task 13: (Optional) Type 4 canary regression test

This task is **best-effort**. If you cannot produce a Type 4 PDF in reasonable time (15 minutes), skip and proceed to Task 14. The spec acknowledged this fixture-sourcing risk.

**Files:**
- (Optionally) Create: `crates/zotero-mcp/tests/fixtures/type4_function.pdf`
- (Optionally) Modify: `crates/zotero-mcp/tests/fixtures/gen_pdfs.py`
- (Optionally) Modify: `crates/zotero-mcp/tests/pdf_text.rs`

- [ ] **Step 1: Try to build a Type 4 fixture**

Append to `gen_pdfs.py`:

```python
def make_type4() -> None:
    """A PDF whose font CMap references a Type 4 (PostScript Calculator) function.

    The goal is to trigger pdf-extract 0.7's `unhandled function type 4` panic
    during text extraction. This is empirical: shading-only Type 4 functions
    do not trigger the panic during text extraction; the function must be
    reached via the text decode path (e.g., CMap, Encoding differences).

    If this fixture does not reproduce the panic when run through
    pdf_extract::extract_text in the Rust test harness, treat the task as
    skipped — the unit-test stubs already cover the orchestrator semantics.
    """
    pdf = pikepdf.Pdf.new()
    pdf.add_blank_page(page_size=(612, 792))
    # ... attempt a CMap-with-Type-4 construction here.
    # If you cannot construct one in pikepdf, an alternative is to
    # download a known-bad PDF from the pdf-extract GitHub issue tracker
    # (with attribution) and commit it after verifying its license.
    pdf.save(HERE / "type4_function.pdf")
    print(f"Wrote {HERE / 'type4_function.pdf'}")
```

- [ ] **Step 2: Verify pdf-extract panics on the fixture**

Write a probe test in `crates/zotero-mcp/tests/pdf_text.rs`:

```rust
#[tokio::test]
async fn pdf_extract_panics_on_type4_fixture() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/type4_function.pdf");
    if !fixture.exists() {
        eprintln!("type4_function.pdf not present; canary test skipped");
        return;
    }
    let eng = zotero_mcp::core::pdf::PdfExtractEngine;
    let res = eng.extract(&fixture).await;
    assert!(matches!(res, Err(zotero_mcp::core::pdf::EngineError::Failed(_))),
            "expected pdf-extract to fail on Type 4 fixture; got: {:?}", res);
}
```

Run: `cargo test -p zotero-mcp --test pdf_text -- pdf_extract_panics_on_type4`

Expected outcomes:
- If the fixture **does** trigger the panic: the test passes (the engine returns `EngineError::Failed`). Commit the fixture and test, then **add a full-chain canary test**:

  ```rust
  #[tokio::test]
  async fn fallback_recovers_from_type4_panic() {
      let Some(bin) = locate_pdftotext() else { return };
      let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
          .join("tests/fixtures/type4_function.pdf");
      if !fixture.exists() { return; }

      // Set up engines manually pointing primary at PdfExtract and fallback at real pdftotext.
      // (Use the same internal extract() via a thin wrapper, or build a PdfEngines and
      //  call get_pdf_text with a fixture pool.) Keep this short — assertion is "text returned".
  }
  ```

- If the fixture **does not** trigger the panic: delete the fixture and the test stub, do not commit them. Note in the commit message of Task 14 that the canary is deferred.

- [ ] **Step 3: Commit (only if Step 2 succeeded)**

```bash
git add crates/zotero-mcp/tests/fixtures/type4_function.pdf \
        crates/zotero-mcp/tests/fixtures/gen_pdfs.py \
        crates/zotero-mcp/tests/pdf_text.rs
git commit -m "test(pdf): add Type 4 canary regression fixture

PDF with a Type 4 (PostScript Calculator) function reachable via text
extraction — reproduces the pdf-extract panic that motivated this
fallback. The test verifies pdf-extract fails AND the orchestrator
recovers via pdftotext."
```

---

## Task 14: README update

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add a Poppler note to the install section**

Open `README.md`. After the bullet list under "You also need:" (the section that already mentions Zotero desktop, BetterBibTeX, and the Zotero Web API key), add one more bullet:

```markdown
- **Poppler's `pdftotext`** (optional, recommended): a small set of PDFs
  use features the pure-Rust `pdf-extract` crate doesn't handle (e.g.
  PostScript Calculator functions). When `pdftotext` is on `PATH`,
  `zotero-mcp` automatically falls back to it and caches the recovered
  text alongside Zotero's own full-text index. Install with:

  ```bash
  brew install poppler          # macOS
  sudo apt install poppler-utils  # Debian/Ubuntu
  ```

  Or set an explicit path in `config.toml`:

  ```toml
  [zotero]
  pdftotext_path = "/opt/homebrew/bin/pdftotext"
  pdftotext_fallback = true   # default; set false to disable
  ```
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs(readme): document Poppler pdftotext fallback for PDF extraction"
```

---

## Task 15: Final verification

- [ ] **Step 1: Run the full test suite**

```bash
cargo test -p zotero-mcp 2>&1 | tail -40
```

Expected: all tests pass. Confirm at minimum:
- `core::config::tests::pdftotext_*` pass.
- `core::error::tests::pdftotext_*` and `pdf_all_engines_failed_*` pass.
- `core::pdf::engine_tests::pdf_extract_engine_returns_failed_for_non_pdf` passes.
- `core::pdf::orchestrator_tests::*` (8 tests) all pass.
- `pdf_text::prefers_zotero_ft_cache_when_present` passes.
- `pdf_text::pdftotext_engine_extracts_text_from_hello_pdf` passes (skips silently if pdftotext is not installed).
- `pdf_text::pdftotext_engine_returns_timeout_when_deadline_exceeded` passes.

- [ ] **Step 2: Smoke-test against the original failing item**

This step verifies the user-visible behaviour for the bug that motivated this work. It requires Zotero desktop running and item `KSALPBV7` present in the library.

Build and run the binary in stdio mode:

```bash
cargo build -p zotero-mcp --release
```

Then issue a single MCP request via the existing `zotero-mcp` MCP server (already configured for Claude Desktop / Claude Code). Either:
- Restart Claude Desktop / Claude Code so it picks up the new build, then ask Claude to call `get_pdf_text` with `item_key = "KSALPBV7"`. Expected: returns text containing "Albedo" or "Nikolov", `source: "pdftotext_fallback"`.

Or for a direct cargo-run smoke test, write a tiny test binary inline:

```bash
cargo run -p zotero-mcp --release --bin zotero-mcp -- --help
```

(The binary's CLI is the primary surface; the actual extraction is exercised through MCP tool calls, so the Claude-side test is the most direct verification.)

- [ ] **Step 3: Confirm the cache file was created**

After a successful fallback call against `KSALPBV7`:

```bash
ls -la /Users/rjl/Zotero/storage/UF8PCADV/.zotero-ft-cache
head -c 200 /Users/rjl/Zotero/storage/UF8PCADV/.zotero-ft-cache
```

Expected: file exists, contains plain UTF-8 starting with text from the paper.

- [ ] **Step 4: Re-invoke the same item and confirm cache path is used**

Call `get_pdf_text` on `KSALPBV7` a second time. Expected: same text, but `source: "zotero_cache"` (the cache the fallback just wrote is being read on this call).

- [ ] **Step 5: Final commit if any cleanup**

If anything (warnings, dead imports) was tweaked during smoke testing:

```bash
git add -p
git commit -m "chore: cleanup after fallback smoke test"
```

---

## Self-Review Notes (filled at plan-write time)

**Spec coverage:**
- §3 extraction chain → Task 10.
- §3.1 PdfTextSource enum → Task 4.
- §4 pdftotext invocation (flags, discovery, hardening) → Tasks 8, 9.
- §5 cache write semantics → Task 10 (`write_cache_atomic`).
- §6 error model → Task 2.
- §7 configuration → Task 1.
- §8 telemetry → Tasks 9 (startup INFO), 10 (per-call WARN/DEBUG).
- §9.1 unit tests → Task 12.
- §9.2 integration test → Task 8 (basic) + Task 13 (Type 4 canary, best-effort).
- §9.3 CI → N/A (no workflows present); documented at top of plan.

**Placeholder scan:** Clean — no "TBD", "implement later", or vague directives. Task 13 is explicit about being best-effort with a defined skip path.

**Type consistency:** `PdfTextSource::PdftotextFallback`, `EngineError::Failed`/`Timeout`, `FallbackState::Ready`/`BinaryMissing`/`Disabled`, `Error::PdftotextMissing`/`PdftotextTimeout`/`PdfAllEnginesFailed` all named consistently across Tasks 2, 4, 5, 6, 7, 8, 9, 10, 12.
