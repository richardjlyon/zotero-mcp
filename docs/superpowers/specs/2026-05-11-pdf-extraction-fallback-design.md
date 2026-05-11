# PDF Extraction Fallback — Design

**Date:** 2026-05-11
**Status:** Approved (design phase)
**Author:** rjl
**Component:** `zotero-mcp::core::pdf`

## 1. Overview

The MCP server's PDF text extraction currently panics on PDFs that use uncommon-but-legal features. Concretely: item `KSALPBV7` ("Roles of Earth's Albedo Variations …", Nikolov & Zeller 2024, a valid 10-page PDF 1.7) triggers

```
pdf extraction failed: task 216 panicked with message "unhandled function type 4"
```

from the `pdf-extract` crate. PDF *Function Type 4* is a PostScript Calculator Function used for colour/shading transforms — the file is well-formed; the crate is missing the feature.

This design adds a Poppler `pdftotext` fallback engine that engages only when `pdf-extract` fails, and caches the recovered text into Zotero's own `.zotero-ft-cache` so subsequent calls are free.

### 1.1 Goals

- Recover gracefully from `pdf-extract` errors and panics for any PDF that Poppler can read (which is essentially all PDFs in the wild).
- Preserve the current fast path: cache hit and `pdf-extract` success behave exactly as today.
- Be honest with callers: surface *which* engine succeeded via `PdfTextSource`.
- Make the fallback's external dependency (Poppler) opt-out-able and discoverable.

### 1.2 Non-goals

- Replacing `pdf-extract` outright. It still handles the common case, ships as a Rust dep, and works for ~all PDFs we've tested.
- OCR. Image-only / scanned PDFs are out of scope; they would be a separate engine added later if needed.
- Repairing actually corrupt PDFs. We will *not* lie about what we extracted from a damaged file.

## 2. Verified Environment

Confirmed during design on 2026-05-11:

- Repro item: `1_KSALPBV7`, attachment `UF8PCADV`, PDF version 1.7, 5.5 MB, 10 pages, zip-deflate encoded.
- `pdf-extract 0.7` panics inside its parser with `unhandled function type 4`.
- The existing panic catch in `core::pdf::extract` (`tokio::task::spawn_blocking` + outer JoinError mapping at `crates/zotero-mcp/src/core/pdf.rs:39-42`) cleanly converts the panic into `Error::Pdf(...)`. No process abort.
- `/Users/rjl/Zotero/storage/UF8PCADV/.zotero-ft-cache` is absent — Zotero's own indexer (which internally uses Poppler-derived tooling) has not produced one for this file.
- System `pdftotext` (Poppler 26.01.0) extracts ~128 KB of clean text from the same file in <1 s.
- Existing `.zotero-ft-cache` files on disk are plain UTF-8 text, no header, trailing newline. Safe to write into.

## 3. Extraction Chain

`core::pdf::extract` becomes a three-step chain.

| Step | Source on success | Trigger |
|---|---|---|
| 1. `.zotero-ft-cache` hit | `ZoteroCache` | File exists |
| 2. `pdf-extract` live | `LiveExtract` | Cache miss |
| 3. `pdftotext` fallback | `PdftotextFallback` (new) | Step 2 returned `Err`; fallback enabled; binary available |
| 4. All failed | — | Returns `Error::PdfAllEnginesFailed { pdf_extract, pdftotext }` |

If the fallback is disabled by config, step 3 is skipped and step 2's `Err` is returned verbatim (preserves today's surface).

### 3.1 `PdfTextSource` enum

Add a third variant:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PdfTextSource {
    ZoteroCache,
    LiveExtract,
    PdftotextFallback,  // new
}
```

Callers that pattern-match on this enum need to be reviewed for exhaustiveness. Search current usage during planning.

## 4. `pdftotext` Invocation

### 4.1 Binary discovery

Resolved at first-fallback time, then cached on `AppState`:

1. `[zotero] pdftotext_path = "..."` in `config.toml`, if set.
2. `which pdftotext` via `which::which` or equivalent.
3. If missing: `Error::PdftotextMissing` with install hint.

On startup, log `INFO` once if `pdftotext` is not on PATH (so the user knows the fallback won't be available before they hit a broken PDF). Startup detection is *advisory only*; the real resolution happens at use time.

### 4.2 Command line

```
pdftotext -enc UTF-8 -q -- <pdf_path> -
```

- `-enc UTF-8`: force UTF-8 (Poppler defaults to `auto`, which is occasionally mojibake).
- `-q`: quiet; suppress non-fatal warnings.
- `--`: stop argument parsing before the path, since filenames can start with `-`.
- `-` (final arg): write to stdout, no temp file.

### 4.3 Process hardening

- Use `tokio::process::Command` for native async execution.
- 60-second `tokio::time::timeout`; on expiry, kill the child and return `Error::PdftotextTimeout(secs, path)`.
- Cap stdout buffer at 50 MB; truncate + log if exceeded. Bounds memory under a runaway PDF.
- Capture stderr separately; include the first 500 bytes in the error if the process exited non-zero.
- Treat exit code 0 + empty stdout as the fallback `Err` (something silently went wrong; prefer to surface than to cache emptiness). The fallback branch returns `Err("empty output")` internally, which composes into `Error::PdfAllEnginesFailed` if the primary also failed.

## 5. Cache Write

On successful fallback:

- Write the recovered text to `<storage_item_dir>/.zotero-ft-cache`.
- Atomic write: `.zotero-ft-cache.tmp` in the same dir, then `rename`.
- UTF-8, no BOM, trailing newline.
- Best-effort: a write failure (permission, disk full) is logged at `WARN` and **does not propagate** — the text is still returned.

Do **not** touch `.zotero-ft-info` (Zotero's indexer state file). If we wrote a fake info file, we'd be lying to Zotero about indexer state and could mask future bugs. The acceptable trade-off: Zotero may later re-index, fail again, and overwrite our cache with nothing — at which point the next call falls back again and rewrites.

Do not write a cache when `pdf-extract` succeeds. Today's code doesn't; we'd be duplicating Zotero's own indexing work.

### 5.1 Alternative considered: sibling-file cache

Write to `.zotero-mcp-ft-cache` instead; have the read path check `.zotero-ft-cache` → `.zotero-mcp-ft-cache` → live extract. Cleaner isolation from Zotero, no risk of clobber. **Rejected** because (a) we don't have an observed clobber problem to solve, (b) cooperating with `.zotero-ft-cache` means Zotero's own UI search picks up the text — a small but real user-visible bonus, (c) more code.

## 6. Error Model

New variants on `core::error::Error`:

```rust
#[error("pdftotext fallback unavailable: install Poppler (`brew install poppler` on macOS, \
         `apt install poppler-utils` on Linux), or set `[zotero] pdftotext_path = \"...\"` \
         in config.toml")]
PdftotextMissing,

#[error("pdftotext timed out after {0}s extracting {1}")]
PdftotextTimeout(u64, String),

#[error("pdf extraction failed in all engines. pdf-extract: {pdf_extract}. pdftotext: {pdftotext}")]
PdfAllEnginesFailed { pdf_extract: String, pdftotext: String },
```

The existing `Error::Pdf(String)` is retained for the "fallback disabled by config AND pdf-extract failed" path — keeps today's surface intact for opt-out users.

## 7. Configuration

Additions to the `[zotero]` section of `config.toml`:

```toml
# Optional: explicit path to pdftotext if not on PATH or not at the auto-resolved location.
pdftotext_path = "/opt/homebrew/bin/pdftotext"

# Optional: disable the fallback entirely. Default: true.
pdftotext_fallback = true
```

Both are optional. Defaults preserve "just works" behaviour where Poppler is installed.

## 8. Telemetry

Using the existing `tracing` setup:

- `INFO` once at startup if `pdftotext` is not discoverable (advisory).
- `WARN` when `pdf-extract` fails and the fallback engages; includes the original error string and the PDF path.
- `DEBUG` for cache write success / failure.
- `WARN` when the cache write fails (permission, disk).

## 9. Testing Strategy

### 9.1 Unit tests

Refactor `core::pdf::extract` so the two engine calls are taken as injected dependencies (trait or function pointers). Production wires the real `pdf-extract` and `pdftotext` runner; tests pass stubs.

Coverage:

- Cache hit short-circuits.
- Primary `Ok` → `LiveExtract`, no cache write occurs.
- Primary `Err` + fallback `Ok` → `PdftotextFallback`, cache file written with the recovered text.
- Primary `Err` + fallback `Err` → `Error::PdfAllEnginesFailed { pdf_extract, pdftotext }`, both strings included.
- Primary `Err` + fallback binary missing → `Error::PdftotextMissing`.
- Cache write fails (read-only dir simulated) → text still returned, warning logged.
- `pdftotext_fallback = false` skips fallback entirely; primary `Err` returns `Error::Pdf` unchanged.
- Timeout path: stub fallback that sleeps past the timeout → `Error::PdftotextTimeout`.

### 9.2 Integration test (gated)

One hand-authored fixture PDF in `tests/fixtures/` containing a Type 4 function. ~1-2 KB target. Test assertions:

- `pdf-extract` panics on it (regression canary — if upstream ever fixes this, the test fails loudly so we can revisit whether the fallback is still needed).
- The full extraction chain returns non-empty text via the fallback.
- A subsequent call to `extract` reads from `.zotero-ft-cache` (`source: ZoteroCache`).
- Skipped at runtime if `pdftotext` is not on PATH so CI without Poppler still passes.

**Fixture sourcing risk.** Hand-authoring a minimal Type 4 PDF is fiddly. Acceptable fallbacks at implementation time:
- Author with a tiny Python script using `reportlab` or `pikepdf`, commit script + generated PDF.
- Use a permissively-licensed sample from a public PDF test corpus (e.g., `pdf-extract`'s own bug-reproducer attachments on GitHub) provided licensing is clear.

### 9.3 CI

If `.github/workflows/` runs tests, add `apt-get install poppler-utils` (Linux) and `brew install poppler` (macOS) before `cargo test` so integration tests aren't silently skipped on every CI run. Audit during planning.

## 10. Files Touched (estimated)

- `crates/zotero-mcp/src/core/pdf.rs` — chain refactor; engine trait/injection.
- `crates/zotero-mcp/src/core/error.rs` — new variants.
- `crates/zotero-mcp/src/core/config.rs` (or wherever `[zotero]` lives) — `pdftotext_path`, `pdftotext_fallback`.
- `crates/zotero-mcp/src/server.rs` or `AppState` — discovered `pdftotext` path cache.
- `crates/zotero-mcp/Cargo.toml` — possibly add `which` if not already present.
- `crates/zotero-mcp/tests/pdf_text.rs` — extended.
- `crates/zotero-mcp/tests/fixtures/` — new Type 4 fixture + (optional) generator script.
- `README.md` — short note on the Poppler dependency for users hitting unusual PDFs.
- `.github/workflows/*.yml` — poppler install step if CI runs tests.

## 11. Out-of-Scope / Future Work

- OCR engine for scanned PDFs (`tesseract` shell-out, or `tesseract-rs`).
- Migrating fully off `pdf-extract` in favour of `pdfium-render` or `mupdf-rs` — only if `pdf-extract` shows more bug coverage gaps over time.
- Detecting "successfully extracted garbage" (e.g., funky CMap-only PDFs that produce text but it's all empty glyphs). Out of scope here; would require heuristic quality scoring.
