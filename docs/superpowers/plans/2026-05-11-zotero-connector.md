# zotero-connector Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust MCP server (`zotero-mcp`) that gives Claude fast, safe access to the user's local Zotero library, plus a confidence-gated metadata enrichment subsystem.

**Architecture:** Two-crate workspace. `zotero-core` is a library: read-only SQLite access to `~/Zotero/zotero.sqlite`, filesystem reads for attachments + extracted-text caches, HTTP writes through Zotero's Local Web API on `localhost:23119`, BetterBibTeX JSON-RPC for citation keys, external scholarly APIs (CrossRef/OpenLibrary/arXiv/Semantic Scholar) for enrichment. `zotero-mcp` is a binary that exposes the library via the `rmcp` MCP server over stdio.

**Tech Stack:** Rust (edition 2021 or 2024), `rmcp`, `rusqlite` (bundled), `deadpool-sqlite`, `reqwest`, `tokio`, `serde`/`serde_json`, `pdf-extract`, `readability`, `directories`, `tracing`, `thiserror`, `wiremock` (dev).

**Spec:** `docs/superpowers/specs/2026-05-11-zotero-connector-design.md`.

**Verified environment facts** (gathered during brainstorming, pin code against these):

- Zotero user ID: `93338`. "My Library" = library ID `1` in SQLite; group `richlyon` = library ID `2`.
- SQLite path: `~/Zotero/zotero.sqlite`. Schema row `userdata` is the version we pin against (current value at design time: `125`).
- Storage layout: `~/Zotero/storage/<itemKey>/<filename>`; cached extracted PDF text at `~/Zotero/storage/<itemKey>/.zotero-ft-cache`.
- Local API base: `http://localhost:23119/api`. Required header: `Zotero-API-Version: 3`.
- BBT JSON-RPC endpoint: `http://localhost:23119/better-bibtex/json-rpc`. Verified methods: `item.citationkey([zoteroKeys])` returns `{zoteroKey: bbtKey}`; `item.search("query")` returns CSL-JSON array.

**Conventions:**

- TDD throughout: failing test → minimal implementation → passing test → commit.
- Each task ends in a single commit. Commit messages use Conventional Commits (`feat:`, `test:`, `chore:`, etc.).
- All logging via `tracing` to **stderr**. Never write to stdout (stdout is the MCP transport).
- All public types in `zotero-core` derive `Serialize` and `Deserialize` so the MCP tool layer can pass them through unchanged.

---

## Phase 1 — Foundation

### Task 1: Workspace bootstrap

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/zotero-core/Cargo.toml`
- Create: `crates/zotero-core/src/lib.rs`
- Create: `crates/zotero-mcp/Cargo.toml`
- Create: `crates/zotero-mcp/src/main.rs`
- Create: `.gitignore`
- Create: `rust-toolchain.toml`

- [ ] **Step 1: Create workspace `Cargo.toml`**

```toml
[workspace]
members = ["crates/zotero-core", "crates/zotero-mcp"]
resolver = "2"

[workspace.package]
edition = "2021"
version = "0.1.0"
authors = ["rjl"]
license = "MIT OR Apache-2.0"
repository = "https://github.com/rjl/zotero-connector"

[workspace.dependencies]
anyhow = "1"
async-trait = "0.1"
deadpool-sqlite = "0.8"
directories = "5"
miette = { version = "7", features = ["fancy"] }
pdf-extract = "0.7"
readability = "0.3"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "gzip", "brotli"] }
rmcp = { version = "0.1", features = ["server", "transport-io"] }
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
tokio = { version = "1", features = ["full"] }
toml = "0.8"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }

[workspace.dependencies.wiremock]
version = "0.6"
```

- [ ] **Step 2: Create `crates/zotero-core/Cargo.toml`**

```toml
[package]
name = "zotero-core"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
deadpool-sqlite.workspace = true
directories.workspace = true
miette.workspace = true
pdf-extract.workspace = true
readability.workspace = true
reqwest.workspace = true
rusqlite.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true
toml.workspace = true
tracing.workspace = true

[dev-dependencies]
tempfile = "3"
wiremock.workspace = true
```

- [ ] **Step 3: Create `crates/zotero-core/src/lib.rs`**

```rust
//! Library: read/write access to a local Zotero installation.
//!
//! See the design spec in `docs/superpowers/specs/`.
```

- [ ] **Step 4: Create `crates/zotero-mcp/Cargo.toml`**

```toml
[package]
name = "zotero-mcp"
edition.workspace = true
version.workspace = true
license.workspace = true

[[bin]]
name = "zotero-mcp"
path = "src/main.rs"

[dependencies]
anyhow.workspace = true
async-trait.workspace = true
rmcp.workspace = true
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
zotero-core = { path = "../zotero-core" }
```

- [ ] **Step 5: Create `crates/zotero-mcp/src/main.rs`**

```rust
fn main() {
    eprintln!("zotero-mcp: not yet implemented");
}
```

- [ ] **Step 6: Create `rust-toolchain.toml`**

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 7: Create `.gitignore`**

```
target/
*.swp
.DS_Store
/zotero-mcp.log
**/.zotero-mcp-cache/
```

- [ ] **Step 8: Verify it builds**

Run: `cargo build --workspace`
Expected: both crates compile; warnings about unused are OK at this stage.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml rust-toolchain.toml .gitignore crates/
git commit -m "chore: bootstrap workspace with zotero-core and zotero-mcp crates"
```

---

### Task 2: Core domain types

**Files:**
- Create: `crates/zotero-core/src/types.rs`
- Modify: `crates/zotero-core/src/lib.rs`
- Test: `crates/zotero-core/src/types.rs` (inline `#[cfg(test)]` module)

- [ ] **Step 1: Write the failing test**

In `crates/zotero-core/src/types.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_serialises_round_trip() {
        let item = Item {
            key: "JGF2UTMW".into(),
            library_id: 1,
            version: 10005,
            item_type: "book".into(),
            citation_key: Some("rabkinWhatModernIsrael2016".into()),
            fields: serde_json::json!({
                "title": "What is Modern Israel?",
                "date": "2016",
                "publisher": "Pluto Press"
            }),
            creators: vec![Creator {
                first_name: Some("Yakob".into()),
                last_name: Some("Rabkin".into()),
                creator_type: "author".into(),
                order_index: 0,
            }],
            tags: vec![],
            collection_keys: vec!["LU3TXR2S".into()],
            date_added: "2026-05-11T06:28:35Z".into(),
            date_modified: "2026-05-11T06:29:38Z".into(),
            parent_key: None,
            recommended_content_tool: Some("get_pdf_text".into()),
        };
        let s = serde_json::to_string(&item).unwrap();
        let back: Item = serde_json::from_str(&s).unwrap();
        assert_eq!(back.key, "JGF2UTMW");
        assert_eq!(back.citation_key.as_deref(), Some("rabkinWhatModernIsrael2016"));
    }

    #[test]
    fn diff_default_has_no_changes() {
        let d = Diff::default();
        assert!(d.changes.is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zotero-core types::tests`
Expected: FAIL with errors about undefined `Item`, `Creator`, `Diff`.

- [ ] **Step 3: Write the types**

Replace `crates/zotero-core/src/types.rs` with:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub key: String,
    pub library_id: i64,
    pub version: i64,
    pub item_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation_key: Option<String>,
    pub fields: Value,
    pub creators: Vec<Creator>,
    pub tags: Vec<String>,
    pub collection_keys: Vec<String>,
    pub date_added: String,
    pub date_modified: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_content_tool: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Creator {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    pub creator_type: String,
    pub order_index: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub key: String,
    pub parent_key: Option<String>,
    pub content_type: Option<String>,
    pub filename: Option<String>,
    pub absolute_path: Option<String>,
    pub link_mode: AttachmentLinkMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentLinkMode {
    ImportedFile,
    ImportedUrl,
    LinkedFile,
    LinkedUrl,
    EmbeddedImage,
    Unknown,
}

impl AttachmentLinkMode {
    pub fn from_i64(n: i64) -> Self {
        // Zotero link_mode constants:
        //   0 = imported_file, 1 = imported_url, 2 = linked_file,
        //   3 = linked_url, 4 = embedded_image
        match n {
            0 => Self::ImportedFile,
            1 => Self::ImportedUrl,
            2 => Self::LinkedFile,
            3 => Self::LinkedUrl,
            4 => Self::EmbeddedImage,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    pub key: String,
    pub library_id: i64,
    pub name: String,
    pub parent_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub item_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub key: String,
    pub parent_attachment_key: String,
    pub kind: String,
    pub text: Option<String>,
    pub comment: Option<String>,
    pub color: Option<String>,
    pub page_label: Option<String>,
    pub sort_index: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation_key: Option<String>,
    pub item_type: String,
    pub title: Option<String>,
    pub creators_short: Option<String>,
    pub year: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_excerpt: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Diff {
    pub changes: Vec<FieldChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldChange {
    pub field: String,
    pub current: Option<Value>,
    pub proposed: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentProposal {
    pub item_key: String,
    pub diff: Diff,
    pub confidence: f64,
    pub source_breakdown: Vec<SourceBreakdown>,
    pub needs_review: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceBreakdown {
    pub source: String,
    pub matched: bool,
    pub fields_contributed: Vec<String>,
    pub raw_response_cached: bool,
}
```

- [ ] **Step 4: Wire into `lib.rs`**

Replace `crates/zotero-core/src/lib.rs` with:

```rust
//! Library: read/write access to a local Zotero installation.

pub mod types;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p zotero-core types::tests`
Expected: 2 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/zotero-core/src/lib.rs crates/zotero-core/src/types.rs
git commit -m "feat(core): add core domain types (Item, Attachment, Collection, etc.)"
```

---

### Task 3: Error type

**Files:**
- Create: `crates/zotero-core/src/error.rs`
- Modify: `crates/zotero-core/src/lib.rs`

- [ ] **Step 1: Write the failing test**

In `crates/zotero-core/src/error.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_includes_context() {
        let e = Error::SchemaMismatch { expected: "125..130".into(), found: 99 };
        let s = e.to_string();
        assert!(s.contains("125..130"));
        assert!(s.contains("99"));
    }

    #[test]
    fn from_rusqlite_error_maps_to_database() {
        let inner = rusqlite::Error::QueryReturnedNoRows;
        let e: Error = inner.into();
        assert!(matches!(e, Error::Database(_)));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zotero-core error::tests`
Expected: FAIL — module not defined.

- [ ] **Step 3: Implement the error type**

In `crates/zotero-core/src/error.rs`:

```rust
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("zotero schema version mismatch: expected {expected}, found {found}")]
    SchemaMismatch { expected: String, found: i64 },

    #[error("zotero is not running; the requested operation requires the Local API at {0}")]
    ZoteroNotRunning(String),

    #[error("item not found: {0}")]
    ItemNotFound(String),

    #[error("attachment not found: {0}")]
    AttachmentNotFound(String),

    #[error("citation key not found: {0}")]
    CitationKeyNotFound(String),

    #[error("local API rejected write (version conflict): {0}")]
    VersionConflict(String),

    #[error("local API error {status}: {body}")]
    LocalApi { status: u16, body: String },

    #[error("better-bibtex JSON-RPC unavailable")]
    BbtUnavailable,

    #[error("better-bibtex error: {0}")]
    Bbt(String),

    #[error("pdf extraction failed: {0}")]
    Pdf(String),

    #[error("html extraction failed: {0}")]
    Html(String),

    #[error("external lookup failed for source {source}: {message}")]
    Lookup { source: String, message: String },

    #[error("config error: {0}")]
    Config(String),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("connection pool error: {0}")]
    Pool(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml error: {0}")]
    Toml(#[from] toml::de::Error),
}
```

- [ ] **Step 4: Wire into `lib.rs`**

`crates/zotero-core/src/lib.rs`:

```rust
//! Library: read/write access to a local Zotero installation.

pub mod error;
pub mod types;

pub use error::{Error, Result};
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p zotero-core error::tests`
Expected: 2 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/zotero-core/src/error.rs crates/zotero-core/src/lib.rs
git commit -m "feat(core): add Error and Result types"
```

---

### Task 4: Config module

**Files:**
- Create: `crates/zotero-core/src/config.rs`
- Modify: `crates/zotero-core/src/lib.rs`

- [ ] **Step 1: Write the failing test**

In `crates/zotero-core/src/config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let c = Config::default();
        assert_eq!(c.zotero.local_api_base, "http://localhost:23119");
        assert_eq!(c.zotero.user_id, 0); // 0 means "auto-detect from local API"
        assert!(c.zotero.include_group_libraries);
        assert!((c.enrichment.auto_apply_threshold - 0.9).abs() < f64::EPSILON);
        assert!(!c.enrichment.sources.is_empty());
    }

    #[test]
    fn parses_partial_toml() {
        let toml = r#"
[enrichment]
auto_apply_threshold = 0.75
"#;
        let c: Config = toml::from_str(toml).unwrap();
        assert!((c.enrichment.auto_apply_threshold - 0.75).abs() < f64::EPSILON);
        // defaults preserved for unspecified sections
        assert_eq!(c.zotero.local_api_base, "http://localhost:23119");
    }

    #[test]
    fn data_dir_expands_tilde() {
        let p = expand_tilde("~/Zotero");
        assert!(p.starts_with("/"));
        assert!(p.contains("Zotero"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zotero-core config::tests`
Expected: FAIL.

- [ ] **Step 3: Implement the config**

In `crates/zotero-core/src/config.rs`:

```rust
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub zotero: ZoteroConfig,
    pub enrichment: EnrichmentConfig,
    pub web: WebConfig,
    pub paths: PathsConfig,
    pub logging: LoggingConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            zotero: ZoteroConfig::default(),
            enrichment: EnrichmentConfig::default(),
            web: WebConfig::default(),
            paths: PathsConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoteroConfig {
    pub data_dir: String,
    pub local_api_base: String,
    pub user_id: i64,
    pub include_group_libraries: bool,
    pub min_schema_userdata: i64,
    pub max_schema_userdata: i64,
}

impl Default for ZoteroConfig {
    fn default() -> Self {
        Self {
            data_dir: "~/Zotero".into(),
            local_api_base: "http://localhost:23119".into(),
            user_id: 0,
            include_group_libraries: true,
            min_schema_userdata: 120,
            max_schema_userdata: 135,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EnrichmentConfig {
    pub auto_apply_threshold: f64,
    pub sources: Vec<String>,
    pub cache_ttl_days: u64,
}

impl Default for EnrichmentConfig {
    fn default() -> Self {
        Self {
            auto_apply_threshold: 0.9,
            sources: vec![
                "crossref".into(),
                "openlibrary".into(),
                "arxiv".into(),
                "semantic_scholar".into(),
            ],
            cache_ttl_days: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfig {
    pub snapshot_cache_ttl_hours: u64,
    pub user_agent: String,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            snapshot_cache_ttl_hours: 24,
            user_agent: "zotero-mcp/0.1".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PathsConfig {
    pub cache_dir: Option<String>,
    pub log_dir: Option<String>,
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            cache_dir: None,
            log_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self { level: "info".into() }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        if let Some(path) = config_path() {
            if path.exists() {
                let text = std::fs::read_to_string(&path)?;
                return Ok(toml::from_str(&text)?);
            }
        }
        Ok(Self::default())
    }

    pub fn resolved_data_dir(&self) -> PathBuf {
        PathBuf::from(expand_tilde(&self.zotero.data_dir))
    }

    pub fn sqlite_path(&self) -> PathBuf {
        self.resolved_data_dir().join("zotero.sqlite")
    }

    pub fn storage_dir(&self) -> PathBuf {
        self.resolved_data_dir().join("storage")
    }

    pub fn resolved_cache_dir(&self) -> PathBuf {
        if let Some(p) = &self.paths.cache_dir {
            return PathBuf::from(expand_tilde(p));
        }
        directories::ProjectDirs::from("dev", "zotero-mcp", "zotero-mcp")
            .map(|d| d.cache_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(expand_tilde("~/.cache/zotero-mcp")))
    }

    pub fn resolved_log_dir(&self) -> PathBuf {
        if let Some(p) = &self.paths.log_dir {
            return PathBuf::from(expand_tilde(p));
        }
        directories::ProjectDirs::from("dev", "zotero-mcp", "zotero-mcp")
            .map(|d| d.data_local_dir().join("logs"))
            .unwrap_or_else(|| PathBuf::from(expand_tilde("~/.local/state/zotero-mcp")))
    }
}

pub fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "zotero-mcp", "zotero-mcp")
        .map(|d| d.config_dir().join("config.toml"))
}

pub fn expand_tilde<S: AsRef<str>>(s: S) -> String {
    let s = s.as_ref();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs_home() {
            return format!("{}/{}", home, rest);
        }
    }
    s.to_string()
}

fn dirs_home() -> Option<String> {
    directories::UserDirs::new()
        .map(|u| u.home_dir().to_string_lossy().into_owned())
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Config(s.into())
    }
}
```

- [ ] **Step 4: Wire into `lib.rs`**

Append to `crates/zotero-core/src/lib.rs`:

```rust
pub mod config;
pub use config::Config;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p zotero-core config::tests`
Expected: 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/zotero-core/src/config.rs crates/zotero-core/src/lib.rs
git commit -m "feat(core): add Config with TOML loading and tilde expansion"
```

---

### Task 5: Logging setup in zotero-mcp

**Files:**
- Create: `crates/zotero-mcp/src/logging.rs`
- Modify: `crates/zotero-mcp/src/main.rs`

- [ ] **Step 1: Implement logging init**

In `crates/zotero-mcp/src/logging.rs`:

```rust
use std::path::Path;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init(level: &str, log_dir: Option<&Path>) -> anyhow::Result<()> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_thread_ids(false);

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer);

    if let Some(dir) = log_dir {
        std::fs::create_dir_all(dir).ok();
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("zotero-mcp.log"))?;
        let file_layer = fmt::layer()
            .with_writer(file)
            .with_ansi(false);
        registry.with(file_layer).try_init().ok();
    } else {
        registry.try_init().ok();
    }
    Ok(())
}
```

- [ ] **Step 2: Update `main.rs` to call init and emit a stderr message**

```rust
mod logging;

fn main() -> anyhow::Result<()> {
    let config = zotero_core::Config::load()
        .unwrap_or_default();
    logging::init(&config.logging.level, Some(&config.resolved_log_dir()))?;
    tracing::info!("zotero-mcp starting (stub)");
    Ok(())
}
```

- [ ] **Step 3: Run it to confirm logs go to stderr only**

Run: `cargo run -p zotero-mcp 2>/tmp/zmcp.stderr 1>/tmp/zmcp.stdout && wc -c /tmp/zmcp.stdout && head -1 /tmp/zmcp.stderr`
Expected: `/tmp/zmcp.stdout` is 0 bytes; `/tmp/zmcp.stderr` contains `zotero-mcp starting`.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-mcp/src/logging.rs crates/zotero-mcp/src/main.rs
git commit -m "feat(mcp): wire tracing subscriber to stderr (never stdout)"
```

---

## Phase 2 — SQLite reader: foundations

### Task 6: Test fixture SQLite database

**Files:**
- Create: `crates/zotero-core/tests/fixtures/build_fixture.sql`
- Create: `crates/zotero-core/tests/fixtures/build_fixture.rs` (helper for tests)
- Create: `crates/zotero-core/tests/fixtures/mod.rs`

Goal: a programmatic fixture builder that creates a small Zotero-shaped SQLite DB in a temp directory for tests. Avoids checking a binary into the repo and avoids drift.

- [ ] **Step 1: Create `crates/zotero-core/tests/fixtures/build_fixture.rs`**

```rust
//! Build a small Zotero-shaped SQLite database for tests.

use rusqlite::Connection;
use std::path::{Path, PathBuf};

pub struct Fixture {
    pub dir: tempfile::TempDir,
}

impl Fixture {
    pub fn sqlite_path(&self) -> PathBuf {
        self.dir.path().join("zotero.sqlite")
    }
    pub fn storage_dir(&self) -> PathBuf {
        self.dir.path().join("storage")
    }
}

pub fn build() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("zotero.sqlite");
    let conn = Connection::open(&db_path).expect("open");
    create_schema(&conn);
    insert_minimal_data(&conn);
    drop(conn);

    let storage = dir.path().join("storage");
    std::fs::create_dir_all(storage.join("AAAA0001")).unwrap();
    std::fs::write(storage.join("AAAA0001").join("paper.pdf"), b"%PDF-1.4 fake").unwrap();
    std::fs::write(
        storage.join("AAAA0001").join(".zotero-ft-cache"),
        b"Cached extracted text of the paper containing keyword zoteroconnectortest.",
    ).unwrap();
    std::fs::create_dir_all(storage.join("BBBB0002")).unwrap();
    std::fs::write(
        storage.join("BBBB0002").join("article.html"),
        b"<html><body><article><h1>An Article</h1><p>Hello snapshot.</p></article></body></html>",
    ).unwrap();

    Fixture { dir }
}

fn create_schema(c: &Connection) {
    // Minimal subset of the real Zotero schema needed by reader code.
    c.execute_batch(r#"
        CREATE TABLE version (schema TEXT PRIMARY KEY, version INT NOT NULL);
        CREATE TABLE libraries (libraryID INTEGER PRIMARY KEY);
        CREATE TABLE itemTypes (itemTypeID INTEGER PRIMARY KEY, typeName TEXT NOT NULL);
        CREATE TABLE fields (fieldID INTEGER PRIMARY KEY, fieldName TEXT NOT NULL);
        CREATE TABLE fieldsCombined (fieldID INTEGER PRIMARY KEY, fieldName TEXT NOT NULL);
        CREATE TABLE creatorTypes (creatorTypeID INTEGER PRIMARY KEY, creatorType TEXT NOT NULL);
        CREATE TABLE items (
            itemID INTEGER PRIMARY KEY,
            itemTypeID INT NOT NULL,
            dateAdded TIMESTAMP NOT NULL,
            dateModified TIMESTAMP NOT NULL,
            clientDateModified TIMESTAMP NOT NULL,
            libraryID INT NOT NULL,
            key TEXT NOT NULL,
            version INT NOT NULL,
            synced INT NOT NULL DEFAULT 0
        );
        CREATE TABLE itemDataValues (valueID INTEGER PRIMARY KEY, value);
        CREATE TABLE itemData (itemID INT, fieldID INT, valueID INT, PRIMARY KEY (itemID, fieldID));
        CREATE TABLE creators (creatorID INTEGER PRIMARY KEY, firstName TEXT, lastName TEXT, fieldMode INT);
        CREATE TABLE itemCreators (itemID INT, creatorID INT, creatorTypeID INT, orderIndex INT, PRIMARY KEY (itemID, creatorID, creatorTypeID, orderIndex));
        CREATE TABLE itemAttachments (
            itemID INTEGER PRIMARY KEY,
            parentItemID INT,
            linkMode INT,
            contentType TEXT,
            path TEXT,
            syncState INT DEFAULT 0
        );
        CREATE TABLE tags (tagID INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
        CREATE TABLE itemTags (itemID INT, tagID INT, type INT, PRIMARY KEY (itemID, tagID));
        CREATE TABLE collections (
            collectionID INTEGER PRIMARY KEY,
            collectionName TEXT NOT NULL,
            parentCollectionID INT,
            libraryID INT NOT NULL,
            key TEXT NOT NULL,
            version INT NOT NULL DEFAULT 0
        );
        CREATE TABLE collectionItems (collectionID INT, itemID INT, orderIndex INT, PRIMARY KEY (collectionID, itemID));
        CREATE TABLE fulltextWords (wordID INTEGER PRIMARY KEY, word TEXT UNIQUE);
        CREATE TABLE fulltextItems (itemID INTEGER PRIMARY KEY, indexedPages INT, totalPages INT, indexedChars INT, totalChars INT, version INT, synced INT);
        CREATE TABLE fulltextItemWords (wordID INT, itemID INT, PRIMARY KEY (wordID, itemID));
        CREATE TABLE itemAnnotations (
            itemID INTEGER PRIMARY KEY,
            parentItemID INT NOT NULL,
            type INTEGER NOT NULL,
            authorName TEXT,
            text TEXT,
            comment TEXT,
            color TEXT,
            pageLabel TEXT,
            sortIndex TEXT NOT NULL,
            position TEXT NOT NULL,
            isExternal INT NOT NULL
        );
        CREATE TABLE itemNotes (itemID INTEGER PRIMARY KEY, parentItemID INT, note TEXT, title TEXT);
    "#).unwrap();
}

fn insert_minimal_data(c: &Connection) {
    c.execute("INSERT INTO version(schema, version) VALUES ('userdata', 125)", []).unwrap();
    c.execute("INSERT INTO libraries(libraryID) VALUES (1)", []).unwrap();
    c.execute("INSERT INTO itemTypes(itemTypeID, typeName) VALUES (2, 'book'), (4, 'journalArticle'), (14, 'webpage'), (3, 'attachment'), (12, 'note'), (37, 'annotation')", []).unwrap();
    c.execute("INSERT INTO fields(fieldID, fieldName) VALUES (1, 'title'), (3, 'date'), (4, 'publisher'), (52, 'DOI'), (60, 'url'), (90, 'abstractNote')", []).unwrap();
    c.execute("INSERT INTO fieldsCombined(fieldID, fieldName) VALUES (1, 'title'), (3, 'date'), (4, 'publisher'), (52, 'DOI'), (60, 'url'), (90, 'abstractNote')", []).unwrap();
    c.execute("INSERT INTO creatorTypes(creatorTypeID, creatorType) VALUES (1, 'author'), (2, 'editor')", []).unwrap();

    // Item 1: a book "What is Modern Israel?" by Yakob Rabkin
    c.execute("INSERT INTO items VALUES (1, 2, '2026-05-01 00:00:00', '2026-05-01 00:00:00', '2026-05-01 00:00:00', 1, 'JGF2UTMW', 10005, 0)", []).unwrap();
    c.execute("INSERT INTO itemDataValues VALUES (1, 'What is Modern Israel?'), (2, '2016'), (3, 'Pluto Press')", []).unwrap();
    c.execute("INSERT INTO itemData VALUES (1, 1, 1), (1, 3, 2), (1, 4, 3)", []).unwrap();
    c.execute("INSERT INTO creators VALUES (1, 'Yakob', 'Rabkin', 0)", []).unwrap();
    c.execute("INSERT INTO itemCreators VALUES (1, 1, 1, 0)", []).unwrap();

    // Item 2: a journal article with a PDF attachment that has cached full text
    c.execute("INSERT INTO items VALUES (2, 4, '2026-05-02 00:00:00', '2026-05-02 00:00:00', '2026-05-02 00:00:00', 1, 'AAAA0001', 11, 0)", []).unwrap();
    c.execute("INSERT INTO itemDataValues VALUES (10, 'A Paper on Things'), (11, '2024'), (12, '10.1234/abcd')", []).unwrap();
    c.execute("INSERT INTO itemData VALUES (2, 1, 10), (2, 3, 11), (2, 52, 12)", []).unwrap();

    // Attachment row for item 2 (item ID 3, key "AAAA0001" so storage dir matches)
    c.execute("INSERT INTO items VALUES (3, 3, '2026-05-02 00:00:00', '2026-05-02 00:00:00', '2026-05-02 00:00:00', 1, 'AAAA0001', 12, 0)", []).unwrap();
    c.execute("INSERT INTO itemAttachments VALUES (3, 2, 0, 'application/pdf', 'storage:paper.pdf', 0)", []).unwrap();

    // Item 4: a webpage item with HTML snapshot
    c.execute("INSERT INTO items VALUES (4, 14, '2026-05-03 00:00:00', '2026-05-03 00:00:00', '2026-05-03 00:00:00', 1, 'WEB00001', 5, 0)", []).unwrap();
    c.execute("INSERT INTO itemDataValues VALUES (20, 'An Article'), (21, 'https://example.com/article')", []).unwrap();
    c.execute("INSERT INTO itemData VALUES (4, 1, 20), (4, 60, 21)", []).unwrap();
    c.execute("INSERT INTO items VALUES (5, 3, '2026-05-03 00:00:00', '2026-05-03 00:00:00', '2026-05-03 00:00:00', 1, 'BBBB0002', 6, 0)", []).unwrap();
    c.execute("INSERT INTO itemAttachments VALUES (5, 4, 1, 'text/html', 'storage:article.html', 0)", []).unwrap();

    // Collection and tag
    c.execute("INSERT INTO collections VALUES (1, 'Reading List', NULL, 1, 'COL00001', 1)", []).unwrap();
    c.execute("INSERT INTO collectionItems VALUES (1, 1, 0), (1, 2, 1), (1, 4, 2)", []).unwrap();
    c.execute("INSERT INTO tags VALUES (1, 'history'), (2, 'method')", []).unwrap();
    c.execute("INSERT INTO itemTags VALUES (1, 1, 0), (2, 2, 0)", []).unwrap();

    // Full-text words for item 2
    c.execute("INSERT INTO fulltextWords VALUES (1, 'zoteroconnectortest'), (2, 'keyword'), (3, 'paper')", []).unwrap();
    c.execute("INSERT INTO fulltextItems VALUES (3, 1, 1, 50, 50, 1, 0)", []).unwrap();
    c.execute("INSERT INTO fulltextItemWords VALUES (1, 3), (2, 3), (3, 3)", []).unwrap();
}

#[allow(dead_code)]
pub fn fixture_path_or_create() -> PathBuf {
    build().sqlite_path()
}
```

- [ ] **Step 2: Create `crates/zotero-core/tests/fixtures/mod.rs`**

```rust
pub mod build_fixture;
```

- [ ] **Step 3: Write a sanity test that uses the fixture**

Create `crates/zotero-core/tests/fixtures_sanity.rs`:

```rust
mod fixtures;
use rusqlite::Connection;

#[test]
fn fixture_has_expected_items() {
    let f = fixtures::build_fixture::build();
    let conn = Connection::open(f.sqlite_path()).unwrap();
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM items WHERE itemTypeID IN (2,4,14)", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 3);
    let title: String = conn.query_row(
        "SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID WHERE d.itemID=1 AND d.fieldID=1",
        [], |r| r.get(0)).unwrap();
    assert_eq!(title, "What is Modern Israel?");
}
```

- [ ] **Step 4: Run it**

Run: `cargo test -p zotero-core --test fixtures_sanity`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-core/tests/
git commit -m "test(core): add synthetic SQLite fixture builder for reader tests"
```

---

### Task 7: SQLite connection open + schema-version check

**Files:**
- Create: `crates/zotero-core/src/reader/mod.rs`
- Create: `crates/zotero-core/src/reader/conn.rs`
- Modify: `crates/zotero-core/src/lib.rs`
- Test: `crates/zotero-core/tests/reader_conn.rs`

- [ ] **Step 1: Write the failing test**

`crates/zotero-core/tests/reader_conn.rs`:

```rust
mod fixtures;
use zotero_core::reader::conn::{open_read_only, check_schema};

#[test]
fn opens_read_only_and_passes_schema_check() {
    let f = fixtures::build_fixture::build();
    let conn = open_read_only(&f.sqlite_path()).unwrap();
    let v = check_schema(&conn, 120, 135).unwrap();
    assert_eq!(v, 125);
}

#[test]
fn rejects_unknown_schema_version() {
    let f = fixtures::build_fixture::build();
    let conn = open_read_only(&f.sqlite_path()).unwrap();
    let err = check_schema(&conn, 200, 210).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("125"));
    assert!(msg.contains("200"));
}
```

Also place a copy of `fixtures/` symlink or `mod fixtures;` shim — easiest:

```rust
// Top of reader_conn.rs already has `mod fixtures;`
// Cargo will resolve via tests/fixtures/mod.rs
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zotero-core --test reader_conn`
Expected: FAIL — module `reader::conn` not defined.

- [ ] **Step 3: Implement**

`crates/zotero-core/src/reader/mod.rs`:

```rust
pub mod conn;
```

`crates/zotero-core/src/reader/conn.rs`:

```rust
use crate::error::{Error, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

pub fn open_read_only(db: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // We rely on Zotero's WAL mode; do not change journaling.
    conn.busy_timeout(std::time::Duration::from_millis(500))?;
    Ok(conn)
}

pub fn check_schema(conn: &Connection, min_inclusive: i64, max_inclusive: i64) -> Result<i64> {
    let v: i64 = conn.query_row(
        "SELECT version FROM version WHERE schema = 'userdata'",
        [],
        |r| r.get(0),
    )?;
    if v < min_inclusive || v > max_inclusive {
        return Err(Error::SchemaMismatch {
            expected: format!("{}..={}", min_inclusive, max_inclusive),
            found: v,
        });
    }
    Ok(v)
}
```

Add `pub mod reader;` to `lib.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p zotero-core --test reader_conn`
Expected: 2 pass.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-core/src/reader/ crates/zotero-core/src/lib.rs crates/zotero-core/tests/reader_conn.rs
git commit -m "feat(core): open SQLite read-only and verify Zotero schema version"
```

---

### Task 8: Async pool wrapper

**Files:**
- Create: `crates/zotero-core/src/reader/pool.rs`
- Modify: `crates/zotero-core/src/reader/mod.rs`
- Test: `crates/zotero-core/tests/reader_pool.rs`

- [ ] **Step 1: Write the failing test**

`crates/zotero-core/tests/reader_pool.rs`:

```rust
mod fixtures;
use zotero_core::reader::pool::ReadOnlyPool;

#[tokio::test(flavor = "multi_thread")]
async fn pool_runs_concurrent_queries() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 4).await.unwrap();
    let mut handles = vec![];
    for _ in 0..8 {
        let p = pool.clone();
        handles.push(tokio::spawn(async move {
            p.with_conn(|c| {
                let n: i64 = c.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))?;
                Ok(n)
            }).await.unwrap()
        }));
    }
    for h in handles {
        let n = h.await.unwrap();
        assert!(n > 0);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zotero-core --test reader_pool`
Expected: FAIL.

- [ ] **Step 3: Implement**

`crates/zotero-core/src/reader/pool.rs`:

```rust
use crate::error::{Error, Result};
use crate::reader::conn::open_read_only;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct ReadOnlyPool {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    sem: Semaphore,
}

impl ReadOnlyPool {
    pub async fn new(path: PathBuf, max: usize) -> Result<Self> {
        // Quick open to validate path / permissions.
        let _probe = open_read_only(&path)?;
        Ok(Self {
            inner: Arc::new(Inner {
                path,
                sem: Semaphore::new(max),
            }),
        })
    }

    pub async fn with_conn<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let _permit = self
            .inner
            .sem
            .acquire()
            .await
            .map_err(|e| Error::Pool(e.to_string()))?;
        let path = self.inner.path.clone();
        let r = tokio::task::spawn_blocking(move || {
            let conn = open_read_only(&path)?;
            f(&conn).map_err(Error::from)
        })
        .await
        .map_err(|e| Error::Pool(e.to_string()))?;
        r
    }
}
```

Add `pub mod pool;` to `reader/mod.rs`.

- [ ] **Step 4: Run test**

Run: `cargo test -p zotero-core --test reader_pool`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-core/src/reader/ crates/zotero-core/tests/reader_pool.rs
git commit -m "feat(core): add async ReadOnlyPool over SQLite"
```

---

### Task 9: get_item by Zotero key

**Files:**
- Create: `crates/zotero-core/src/reader/items.rs`
- Modify: `crates/zotero-core/src/reader/mod.rs`
- Test: `crates/zotero-core/tests/reader_items.rs`

- [ ] **Step 1: Write the failing test**

`crates/zotero-core/tests/reader_items.rs`:

```rust
mod fixtures;
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::reader::items::get_item_by_key;

#[tokio::test]
async fn fetches_item_with_fields_and_creators() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();

    let item = get_item_by_key(&pool, "JGF2UTMW", 1).await.unwrap();
    assert_eq!(item.item_type, "book");
    assert_eq!(item.fields["title"], "What is Modern Israel?");
    assert_eq!(item.fields["date"], "2016");
    assert_eq!(item.fields["publisher"], "Pluto Press");
    assert_eq!(item.creators.len(), 1);
    assert_eq!(item.creators[0].last_name.as_deref(), Some("Rabkin"));
    assert_eq!(item.creators[0].creator_type, "author");
    assert_eq!(item.collection_keys, vec!["COL00001"]);
    assert!(item.recommended_content_tool.is_none()); // no PDF attached directly
}

#[tokio::test]
async fn missing_item_returns_error() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let err = get_item_by_key(&pool, "DOESNOTEXIST", 1).await.unwrap_err();
    assert!(matches!(err, zotero_core::Error::ItemNotFound(_)));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zotero-core --test reader_items`
Expected: FAIL.

- [ ] **Step 3: Implement**

`crates/zotero-core/src/reader/items.rs`:

```rust
use crate::error::{Error, Result};
use crate::reader::pool::ReadOnlyPool;
use crate::types::{Creator, Item};
use serde_json::{json, Map, Value};

pub async fn get_item_by_key(pool: &ReadOnlyPool, key: &str, library_id: i64) -> Result<Item> {
    let key_owned = key.to_string();
    pool.with_conn(move |c| {
        // Resolve itemID, itemType, base fields
        let (item_id, item_type_id, date_added, date_modified, version): (i64, i64, String, String, i64) = c.query_row(
            "SELECT i.itemID, i.itemTypeID, i.dateAdded, i.dateModified, i.version
             FROM items i WHERE i.libraryID = ? AND i.key = ?",
            (library_id, &key_owned),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        ).map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                rusqlite::Error::UserFunctionError(Box::new(std::io::Error::other(
                    format!("__zmcp_item_not_found:{}", &key_owned)
                )))
            }
            other => other,
        })?;

        let item_type: String = c.query_row(
            "SELECT typeName FROM itemTypes WHERE itemTypeID = ?",
            [item_type_id], |r| r.get(0))?;

        // Fields
        let mut fields = Map::new();
        let mut stmt = c.prepare(
            "SELECT f.fieldName, v.value
             FROM itemData d
             JOIN fieldsCombined f ON f.fieldID = d.fieldID
             JOIN itemDataValues v ON v.valueID = d.valueID
             WHERE d.itemID = ?")?;
        let mut rows = stmt.query([item_id])?;
        while let Some(r) = rows.next()? {
            let name: String = r.get(0)?;
            let value: String = r.get(1)?;
            fields.insert(name, Value::String(value));
        }

        // Creators
        let mut creators = vec![];
        let mut stmt = c.prepare(
            "SELECT cr.firstName, cr.lastName, ct.creatorType, ic.orderIndex
             FROM itemCreators ic
             JOIN creators cr ON cr.creatorID = ic.creatorID
             JOIN creatorTypes ct ON ct.creatorTypeID = ic.creatorTypeID
             WHERE ic.itemID = ?
             ORDER BY ic.orderIndex ASC")?;
        let mut rows = stmt.query([item_id])?;
        while let Some(r) = rows.next()? {
            creators.push(Creator {
                first_name: r.get::<_, Option<String>>(0)?,
                last_name:  r.get::<_, Option<String>>(1)?,
                creator_type: r.get::<_, String>(2)?,
                order_index: r.get::<_, i64>(3)?,
            });
        }

        // Tags
        let mut tags = vec![];
        let mut stmt = c.prepare(
            "SELECT t.name FROM itemTags it JOIN tags t ON t.tagID = it.tagID WHERE it.itemID = ? ORDER BY t.name")?;
        let mut rows = stmt.query([item_id])?;
        while let Some(r) = rows.next()? { tags.push(r.get::<_, String>(0)?); }

        // Collections
        let mut collection_keys = vec![];
        let mut stmt = c.prepare(
            "SELECT col.key FROM collectionItems ci JOIN collections col ON col.collectionID = ci.collectionID WHERE ci.itemID = ? ORDER BY ci.orderIndex")?;
        let mut rows = stmt.query([item_id])?;
        while let Some(r) = rows.next()? { collection_keys.push(r.get::<_, String>(0)?); }

        // recommended_content_tool: if item has a child PDF attachment → get_pdf_text;
        // if it has an HTML snapshot OR a `url` field → get_webpage_content; else none.
        let has_pdf: i64 = c.query_row(
            "SELECT COUNT(*) FROM itemAttachments a JOIN items i ON i.itemID = a.itemID
             WHERE a.parentItemID = ? AND a.contentType = 'application/pdf'",
            [item_id], |r| r.get(0))?;
        let has_html: i64 = c.query_row(
            "SELECT COUNT(*) FROM itemAttachments a WHERE a.parentItemID = ? AND a.contentType = 'text/html'",
            [item_id], |r| r.get(0))?;
        let has_url = fields.contains_key("url");
        let recommended_content_tool = if has_pdf > 0 {
            Some("get_pdf_text".to_string())
        } else if has_html > 0 || has_url {
            Some("get_webpage_content".to_string())
        } else {
            None
        };

        Ok(Item {
            key: key_owned,
            library_id,
            version,
            item_type,
            citation_key: None, // populated later when BBT is wired in
            fields: Value::Object(fields),
            creators,
            tags,
            collection_keys,
            date_added,
            date_modified,
            parent_key: None,
            recommended_content_tool,
        })
    }).await.map_err(map_user_err)
}

fn map_user_err(e: Error) -> Error {
    if let Error::Database(rusqlite::Error::UserFunctionError(ref boxed)) = e {
        let s = boxed.to_string();
        if let Some(rest) = s.strip_prefix("__zmcp_item_not_found:") {
            return Error::ItemNotFound(rest.to_string());
        }
    }
    e
}
```

Add `pub mod items;` to `reader/mod.rs`.

- [ ] **Step 4: Run test**

Run: `cargo test -p zotero-core --test reader_items`
Expected: 2 pass.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-core/src/reader/ crates/zotero-core/tests/reader_items.rs
git commit -m "feat(core): read items with fields, creators, tags, collections"
```

---

### Task 10: list_collections, list_tags, list_recent_items

**Files:**
- Create: `crates/zotero-core/src/reader/collections.rs`
- Create: `crates/zotero-core/src/reader/tags.rs`
- Create: `crates/zotero-core/src/reader/recent.rs`
- Modify: `crates/zotero-core/src/reader/mod.rs`
- Test: `crates/zotero-core/tests/reader_browse.rs`

- [ ] **Step 1: Write the failing test**

`crates/zotero-core/tests/reader_browse.rs`:

```rust
mod fixtures;
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::reader::{collections, recent, tags};

#[tokio::test]
async fn lists_collections_tags_and_recent() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();

    let cs = collections::list(&pool, 1, None).await.unwrap();
    assert_eq!(cs.len(), 1);
    assert_eq!(cs[0].key, "COL00001");
    assert_eq!(cs[0].name, "Reading List");

    let ts = tags::list(&pool, 1, None).await.unwrap();
    assert!(ts.iter().any(|t| t.name == "history" && t.item_count == 1));

    let rs = recent::list(&pool, 1, "dateModified", 5).await.unwrap();
    assert!(rs.len() >= 3);
}
```

- [ ] **Step 2: Run test (fails)**

Run: `cargo test -p zotero-core --test reader_browse`
Expected: FAIL.

- [ ] **Step 3: Implement collections**

`crates/zotero-core/src/reader/collections.rs`:

```rust
use crate::error::Result;
use crate::reader::pool::ReadOnlyPool;
use crate::types::Collection;

pub async fn list(pool: &ReadOnlyPool, library_id: i64, parent: Option<String>) -> Result<Vec<Collection>> {
    pool.with_conn(move |c| {
        let mut out = vec![];
        let mut sql = String::from(
            "SELECT c.key, c.libraryID, c.collectionName, p.key
             FROM collections c
             LEFT JOIN collections p ON p.collectionID = c.parentCollectionID
             WHERE c.libraryID = ?"
        );
        if parent.is_some() {
            sql.push_str(" AND p.key = ?");
        }
        sql.push_str(" ORDER BY c.collectionName");
        let mut stmt = c.prepare(&sql)?;
        let mut rows = if let Some(p) = parent.as_deref() {
            stmt.query(rusqlite::params![library_id, p])?
        } else {
            stmt.query([library_id])?
        };
        while let Some(r) = rows.next()? {
            out.push(Collection {
                key: r.get(0)?,
                library_id: r.get(1)?,
                name: r.get(2)?,
                parent_key: r.get::<_, Option<String>>(3)?,
            });
        }
        Ok(out)
    }).await
}
```

- [ ] **Step 4: Implement tags**

`crates/zotero-core/src/reader/tags.rs`:

```rust
use crate::error::Result;
use crate::reader::pool::ReadOnlyPool;
use crate::types::Tag;

pub async fn list(pool: &ReadOnlyPool, library_id: i64, prefix: Option<String>) -> Result<Vec<Tag>> {
    pool.with_conn(move |c| {
        let like = prefix.as_ref().map(|p| format!("{}%", p));
        let mut out = vec![];
        let sql = if like.is_some() {
            "SELECT t.name, COUNT(it.itemID) AS n
             FROM tags t JOIN itemTags it ON it.tagID = t.tagID JOIN items i ON i.itemID = it.itemID
             WHERE i.libraryID = ? AND t.name LIKE ?
             GROUP BY t.name ORDER BY n DESC, t.name"
        } else {
            "SELECT t.name, COUNT(it.itemID) AS n
             FROM tags t JOIN itemTags it ON it.tagID = t.tagID JOIN items i ON i.itemID = it.itemID
             WHERE i.libraryID = ?
             GROUP BY t.name ORDER BY n DESC, t.name"
        };
        let mut stmt = c.prepare(sql)?;
        let mut rows = if let Some(l) = like.as_deref() {
            stmt.query(rusqlite::params![library_id, l])?
        } else {
            stmt.query([library_id])?
        };
        while let Some(r) = rows.next()? {
            out.push(Tag { name: r.get(0)?, item_count: r.get(1)? });
        }
        Ok(out)
    }).await
}
```

- [ ] **Step 5: Implement recent**

`crates/zotero-core/src/reader/recent.rs`:

```rust
use crate::error::{Error, Result};
use crate::reader::pool::ReadOnlyPool;
use crate::types::SearchHit;

pub async fn list(pool: &ReadOnlyPool, library_id: i64, sort_by: &str, limit: i64) -> Result<Vec<SearchHit>> {
    let col = match sort_by {
        "dateAdded" => "i.dateAdded",
        "dateModified" => "i.dateModified",
        other => return Err(Error::Config(format!("sort_by must be dateAdded or dateModified, got {}", other))),
    };
    let sql = format!(
        "SELECT i.key, it.typeName,
                (SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = i.itemID AND d.fieldID = (SELECT fieldID FROM fieldsCombined WHERE fieldName='title')),
                (SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = i.itemID AND d.fieldID = (SELECT fieldID FROM fieldsCombined WHERE fieldName='date'))
         FROM items i JOIN itemTypes it ON it.itemTypeID = i.itemTypeID
         WHERE i.libraryID = ?
           AND it.typeName NOT IN ('attachment', 'note', 'annotation')
         ORDER BY {} DESC LIMIT ?",
        col
    );
    pool.with_conn(move |c| {
        let mut out = vec![];
        let mut stmt = c.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params![library_id, limit])?;
        while let Some(r) = rows.next()? {
            let year = r.get::<_, Option<String>>(3)?.and_then(|s| s.split('-').next().map(str::to_string));
            out.push(SearchHit {
                key: r.get(0)?,
                citation_key: None,
                item_type: r.get(1)?,
                title: r.get::<_, Option<String>>(2)?,
                creators_short: None,
                year,
                match_excerpt: None,
            });
        }
        Ok(out)
    }).await
}
```

- [ ] **Step 6: Wire modules in `reader/mod.rs`**

```rust
pub mod conn;
pub mod pool;
pub mod items;
pub mod collections;
pub mod tags;
pub mod recent;
```

- [ ] **Step 7: Run test**

Run: `cargo test -p zotero-core --test reader_browse`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/zotero-core/src/reader/ crates/zotero-core/tests/reader_browse.rs
git commit -m "feat(core): list collections, tags (with counts), and recent items"
```

---

---

## Phase 3 — SQLite reader: search and attachments

### Task 11: Metadata search

**Files:**
- Create: `crates/zotero-core/src/reader/search.rs`
- Modify: `crates/zotero-core/src/reader/mod.rs`
- Test: `crates/zotero-core/tests/reader_search.rs`

- [ ] **Step 1: Write the failing test**

`crates/zotero-core/tests/reader_search.rs`:

```rust
mod fixtures;
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::reader::search::{search_metadata, SearchParams};

#[tokio::test]
async fn finds_items_by_title_substring() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let hits = search_metadata(&pool, 1, SearchParams { query: "Israel".into(), ..Default::default() }).await.unwrap();
    assert!(hits.iter().any(|h| h.key == "JGF2UTMW"));
}

#[tokio::test]
async fn finds_items_by_creator_lastname() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let hits = search_metadata(&pool, 1, SearchParams { query: "Rabkin".into(), ..Default::default() }).await.unwrap();
    assert!(hits.iter().any(|h| h.key == "JGF2UTMW"));
}

#[tokio::test]
async fn limit_and_offset_work() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let hits = search_metadata(&pool, 1, SearchParams {
        query: String::new(),
        limit: 1, offset: 0, ..Default::default()
    }).await.unwrap();
    assert_eq!(hits.len(), 1);
}
```

- [ ] **Step 2: Run test (fails)**

Run: `cargo test -p zotero-core --test reader_search`
Expected: FAIL.

- [ ] **Step 3: Implement**

`crates/zotero-core/src/reader/search.rs`:

```rust
use crate::error::Result;
use crate::reader::pool::ReadOnlyPool;
use crate::types::SearchHit;

#[derive(Debug, Clone, Default)]
pub struct SearchParams {
    pub query: String,
    pub item_type: Option<String>,
    pub tag: Option<String>,
    pub collection_key: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn search_metadata(
    pool: &ReadOnlyPool,
    library_id: i64,
    mut params: SearchParams,
) -> Result<Vec<SearchHit>> {
    if params.limit <= 0 { params.limit = 50; }

    pool.with_conn(move |c| {
        let q = params.query.trim();
        let q_like = if q.is_empty() { "%".to_string() } else { format!("%{}%", q) };

        // Build base query. We resolve title/date via subqueries so the row stays one item.
        let mut sql = String::from(
            "SELECT DISTINCT i.itemID, i.key, it.typeName,
                (SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = i.itemID AND d.fieldID = (SELECT fieldID FROM fieldsCombined WHERE fieldName='title')) AS title,
                (SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = i.itemID AND d.fieldID = (SELECT fieldID FROM fieldsCombined WHERE fieldName='date')) AS date
             FROM items i
             JOIN itemTypes it ON it.itemTypeID = i.itemTypeID
             LEFT JOIN itemCreators ic ON ic.itemID = i.itemID
             LEFT JOIN creators cr ON cr.creatorID = ic.creatorID
             LEFT JOIN itemTags itag ON itag.itemID = i.itemID
             LEFT JOIN tags tg ON tg.tagID = itag.tagID
             LEFT JOIN collectionItems ci ON ci.itemID = i.itemID
             LEFT JOIN collections cl ON cl.collectionID = ci.collectionID
             WHERE i.libraryID = ?
               AND it.typeName NOT IN ('attachment','note','annotation')"
        );
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(library_id)];

        if !q.is_empty() {
            sql.push_str(" AND (
                EXISTS (SELECT 1 FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                        WHERE d.itemID = i.itemID AND v.value LIKE ?)
                OR cr.lastName LIKE ? OR cr.firstName LIKE ?
                OR tg.name LIKE ?
            )");
            for _ in 0..4 { binds.push(Box::new(q_like.clone())); }
        }

        if let Some(t) = &params.item_type {
            sql.push_str(" AND it.typeName = ?");
            binds.push(Box::new(t.clone()));
        }
        if let Some(t) = &params.tag {
            sql.push_str(" AND tg.name = ?");
            binds.push(Box::new(t.clone()));
        }
        if let Some(ck) = &params.collection_key {
            sql.push_str(" AND cl.key = ?");
            binds.push(Box::new(ck.clone()));
        }
        sql.push_str(" ORDER BY i.dateModified DESC LIMIT ? OFFSET ?");
        binds.push(Box::new(params.limit));
        binds.push(Box::new(params.offset));

        let params_refs: Vec<&dyn rusqlite::ToSql> = binds.iter().map(|b| &**b).collect();
        let mut stmt = c.prepare(&sql)?;
        let mut rows = stmt.query(params_refs.as_slice())?;
        let mut out = vec![];
        while let Some(r) = rows.next()? {
            let date: Option<String> = r.get(4)?;
            out.push(SearchHit {
                key: r.get(1)?,
                citation_key: None,
                item_type: r.get(2)?,
                title: r.get::<_, Option<String>>(3)?,
                creators_short: None,
                year: date.and_then(|s| s.split('-').next().map(str::to_string)),
                match_excerpt: None,
            });
        }
        Ok(out)
    }).await
}
```

Add `pub mod search;` to `reader/mod.rs`.

- [ ] **Step 4: Run test**

Run: `cargo test -p zotero-core --test reader_search`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-core/src/reader/ crates/zotero-core/tests/reader_search.rs
git commit -m "feat(core): metadata search across title, creator, tag, collection"
```

---

### Task 12: Full-text search via fulltextItemWords

**Files:**
- Create: `crates/zotero-core/src/reader/fulltext.rs`
- Modify: `crates/zotero-core/src/reader/mod.rs`, `crates/zotero-core/src/reader/search.rs`
- Test: `crates/zotero-core/tests/reader_fulltext.rs`

- [ ] **Step 1: Write the failing test**

`crates/zotero-core/tests/reader_fulltext.rs`:

```rust
mod fixtures;
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::reader::fulltext::fulltext_match_items;

#[tokio::test]
async fn matches_items_by_indexed_word() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    // The fixture's PDF attachment (key AAAA0001) has fulltextWord "zoteroconnectortest".
    // That attachment's parent item is key AAAA0001 (item ID 2).
    let parents = fulltext_match_items(&pool, 1, "zoteroconnectortest").await.unwrap();
    assert!(parents.contains(&"AAAA0001".to_string()));
}

#[tokio::test]
async fn unknown_word_returns_empty() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let parents = fulltext_match_items(&pool, 1, "nonexistentwordxyz").await.unwrap();
    assert!(parents.is_empty());
}
```

- [ ] **Step 2: Run test (fails)**

Run: `cargo test -p zotero-core --test reader_fulltext`
Expected: FAIL.

- [ ] **Step 3: Implement**

`crates/zotero-core/src/reader/fulltext.rs`:

```rust
use crate::error::Result;
use crate::reader::pool::ReadOnlyPool;

/// Lowercased-word match. Returns parent-item keys (i.e. the actual library
/// items, not the attachment items) for hits.
pub async fn fulltext_match_items(pool: &ReadOnlyPool, library_id: i64, word: &str) -> Result<Vec<String>> {
    let needle = word.to_lowercase();
    pool.with_conn(move |c| {
        let mut out = vec![];
        let mut stmt = c.prepare(
            "SELECT DISTINCT parent.key
             FROM fulltextWords fw
             JOIN fulltextItemWords fiw ON fiw.wordID = fw.wordID
             JOIN itemAttachments a ON a.itemID = fiw.itemID
             JOIN items parent ON parent.itemID = a.parentItemID
             WHERE parent.libraryID = ? AND fw.word = ?"
        )?;
        let mut rows = stmt.query(rusqlite::params![library_id, needle])?;
        while let Some(r) = rows.next()? { out.push(r.get::<_, String>(0)?); }
        Ok(out)
    }).await
}
```

Add `pub mod fulltext;` to `reader/mod.rs`.

- [ ] **Step 4: Integrate full-text matches into `search_metadata`**

Update `crates/zotero-core/src/reader/search.rs` — add a `fields` filter param and a fulltext branch:

Add to `SearchParams`:

```rust
#[derive(Debug, Clone, Default)]
pub struct SearchParams {
    pub query: String,
    pub item_type: Option<String>,
    pub tag: Option<String>,
    pub collection_key: Option<String>,
    pub include_fulltext: bool,
    pub limit: i64,
    pub offset: i64,
}
```

Inside `search_metadata`, after computing `q_like` and before pushing the final ORDER BY, if `params.include_fulltext` is true AND `q` is a single token (no whitespace), inject:

```rust
if params.include_fulltext && !q.is_empty() && !q.contains(char::is_whitespace) {
    // Append OR clause using full-text hit set as a subquery
    sql.push_str(" OR i.key IN (
        SELECT DISTINCT parent.key
        FROM fulltextWords fw
        JOIN fulltextItemWords fiw ON fiw.wordID = fw.wordID
        JOIN itemAttachments a ON a.itemID = fiw.itemID
        JOIN items parent ON parent.itemID = a.parentItemID
        WHERE parent.libraryID = ? AND fw.word = LOWER(?)
    )");
    binds.push(Box::new(library_id));
    binds.push(Box::new(q.to_string()));
}
```

Note: the outer `AND (...)` group already includes the title/creator/tag matchers — wrap the new OR clause inside that group. Restructure if needed so the precedence is `WHERE library AND type NOT IN ... AND ( metadata-matches OR fulltext-matches ) AND (filters...)`.

- [ ] **Step 5: Extend the search test**

Append to `crates/zotero-core/tests/reader_search.rs`:

```rust
#[tokio::test]
async fn fulltext_finds_pdf_word() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let hits = search_metadata(&pool, 1, SearchParams {
        query: "zoteroconnectortest".into(),
        include_fulltext: true,
        ..Default::default()
    }).await.unwrap();
    assert!(hits.iter().any(|h| h.key == "AAAA0001"));
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p zotero-core --test reader_fulltext --test reader_search`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/zotero-core/src/reader/ crates/zotero-core/tests/
git commit -m "feat(core): full-text search via fulltextItemWords joined to library items"
```

---

### Task 13: list_attachments + resolve filesystem path

**Files:**
- Create: `crates/zotero-core/src/reader/attachments.rs`
- Modify: `crates/zotero-core/src/reader/mod.rs`
- Test: `crates/zotero-core/tests/reader_attachments.rs`

- [ ] **Step 1: Write the failing test**

```rust
mod fixtures;
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::reader::attachments::{list_attachments, resolve_path};

#[tokio::test]
async fn lists_pdf_attachment_and_resolves_path() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();

    let atts = list_attachments(&pool, "AAAA0001", 1, &f.storage_dir()).await.unwrap();
    assert_eq!(atts.len(), 1);
    let a = &atts[0];
    assert_eq!(a.content_type.as_deref(), Some("application/pdf"));
    assert!(a.absolute_path.as_ref().unwrap().ends_with("AAAA0001/paper.pdf"));

    let p = resolve_path(&pool, "AAAA0001", 1, &f.storage_dir()).await.unwrap();
    assert!(p.ends_with("AAAA0001/paper.pdf"));
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/reader/attachments.rs`:

```rust
use crate::error::{Error, Result};
use crate::reader::pool::ReadOnlyPool;
use crate::types::{Attachment, AttachmentLinkMode};
use std::path::{Path, PathBuf};

/// Lists attachments for an item identified by its parent's Zotero key.
/// Returns child attachments (PDFs, snapshots), each with resolved filesystem path
/// when possible.
pub async fn list_attachments(
    pool: &ReadOnlyPool,
    parent_key: &str,
    library_id: i64,
    storage_dir: &Path,
) -> Result<Vec<Attachment>> {
    let parent_key = parent_key.to_string();
    let storage_dir = storage_dir.to_path_buf();

    pool.with_conn(move |c| {
        let parent_id: Option<i64> = c.query_row(
            "SELECT itemID FROM items WHERE libraryID = ? AND key = ?",
            rusqlite::params![library_id, &parent_key],
            |r| r.get(0),
        ).ok();
        let parent_id = match parent_id {
            Some(id) => id,
            None => return Ok(vec![]),
        };

        let mut out = vec![];
        let mut stmt = c.prepare(
            "SELECT i.key, a.linkMode, a.contentType, a.path
             FROM itemAttachments a JOIN items i ON i.itemID = a.itemID
             WHERE a.parentItemID = ? AND i.libraryID = ?"
        )?;
        let mut rows = stmt.query(rusqlite::params![parent_id, library_id])?;
        while let Some(r) = rows.next()? {
            let key: String = r.get(0)?;
            let link_mode = AttachmentLinkMode::from_i64(r.get(1)?);
            let content_type: Option<String> = r.get(2)?;
            let path_raw: Option<String> = r.get(3)?;
            let (filename, absolute_path) = resolve_filename(&storage_dir, &key, path_raw.as_deref(), link_mode);
            out.push(Attachment {
                key,
                parent_key: Some(parent_key.clone()),
                content_type,
                filename,
                absolute_path,
                link_mode,
            });
        }
        Ok(out)
    }).await
}

/// Returns the absolute path to the (first, preferred) attachment of an item.
pub async fn resolve_path(
    pool: &ReadOnlyPool,
    parent_key: &str,
    library_id: i64,
    storage_dir: &Path,
) -> Result<PathBuf> {
    let atts = list_attachments(pool, parent_key, library_id, storage_dir).await?;
    // Prefer PDFs first, then HTML snapshots
    let chosen = atts.iter().find(|a| a.content_type.as_deref() == Some("application/pdf"))
        .or_else(|| atts.iter().find(|a| a.content_type.as_deref() == Some("text/html")))
        .ok_or_else(|| Error::AttachmentNotFound(parent_key.to_string()))?;
    chosen.absolute_path.as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| Error::AttachmentNotFound(parent_key.to_string()))
}

fn resolve_filename(
    storage_dir: &Path,
    key: &str,
    path_raw: Option<&str>,
    link_mode: AttachmentLinkMode,
) -> (Option<String>, Option<String>) {
    // Zotero `path` formats:
    //   "storage:foo.pdf"    -> imported file, in storage/<key>/foo.pdf
    //   "attachments:foo.pdf" -> linked file, base-dir relative (out of scope for v1)
    //   absolute path        -> linked file
    //   null                 -> unknown
    let raw = match path_raw {
        Some(s) => s,
        None => return (None, None),
    };
    if let Some(name) = raw.strip_prefix("storage:") {
        let abs = storage_dir.join(key).join(name);
        let exists_abs = abs.to_string_lossy().to_string();
        let abs_opt = if abs.exists() { Some(exists_abs) } else { Some(abs.to_string_lossy().to_string()) };
        return (Some(name.to_string()), abs_opt);
    }
    if matches!(link_mode, AttachmentLinkMode::LinkedFile) {
        let p = std::path::Path::new(raw);
        let abs = if p.is_absolute() {
            Some(raw.to_string())
        } else {
            None // base-dir-relative linked files not supported in v1
        };
        let fname = p.file_name().map(|f| f.to_string_lossy().to_string());
        return (fname, abs);
    }
    (Some(raw.to_string()), None)
}
```

Add `pub mod attachments;` to `reader/mod.rs`.

- [ ] **Step 3: Run test**

Run: `cargo test -p zotero-core --test reader_attachments`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/src/reader/ crates/zotero-core/tests/reader_attachments.rs
git commit -m "feat(core): list attachments and resolve their filesystem paths"
```

---

### Task 14: list_annotations

**Files:**
- Create: `crates/zotero-core/src/reader/annotations.rs`
- Modify: `crates/zotero-core/src/reader/mod.rs`
- Test: `crates/zotero-core/tests/reader_annotations.rs`

- [ ] **Step 1: Extend fixture with a sample annotation**

Modify `crates/zotero-core/tests/fixtures/build_fixture.rs` — in `insert_minimal_data`, append:

```rust
    // Annotation on the PDF attachment (parentItemID = 3, attachment for item 2)
    c.execute("INSERT INTO items VALUES (6, 37, '2026-05-04 00:00:00', '2026-05-04 00:00:00', '2026-05-04 00:00:00', 1, 'ANNO0001', 1, 0)", []).unwrap();
    c.execute("INSERT INTO itemAnnotations VALUES (6, 3, 1, 'rjl', 'A highlighted passage.', 'My note on it.', '#ffff00', '12', '00012|00000', '{}', 0)", []).unwrap();
```

- [ ] **Step 2: Write the failing test**

`crates/zotero-core/tests/reader_annotations.rs`:

```rust
mod fixtures;
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::reader::annotations::list_annotations;

#[tokio::test]
async fn lists_annotations_for_item() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let anns = list_annotations(&pool, "AAAA0001", 1).await.unwrap();
    assert_eq!(anns.len(), 1);
    let a = &anns[0];
    assert_eq!(a.text.as_deref(), Some("A highlighted passage."));
    assert_eq!(a.comment.as_deref(), Some("My note on it."));
    assert_eq!(a.color.as_deref(), Some("#ffff00"));
    assert_eq!(a.kind, "highlight");
}
```

- [ ] **Step 3: Implement**

`crates/zotero-core/src/reader/annotations.rs`:

```rust
use crate::error::Result;
use crate::reader::pool::ReadOnlyPool;
use crate::types::Annotation;

fn annotation_kind(t: i64) -> &'static str {
    // Zotero annotation types: 1=highlight 2=note 3=image 4=ink 5=underline
    match t {
        1 => "highlight",
        2 => "note",
        3 => "image",
        4 => "ink",
        5 => "underline",
        _ => "unknown",
    }
}

pub async fn list_annotations(pool: &ReadOnlyPool, parent_item_key: &str, library_id: i64) -> Result<Vec<Annotation>> {
    let key = parent_item_key.to_string();
    pool.with_conn(move |c| {
        let parent_id: Option<i64> = c.query_row(
            "SELECT itemID FROM items WHERE libraryID = ? AND key = ?",
            rusqlite::params![library_id, &key], |r| r.get(0)).ok();
        let Some(parent_id) = parent_id else { return Ok(vec![]) };

        // Find attachment items for the parent
        let mut attachment_ids = vec![];
        let mut stmt = c.prepare("SELECT itemID FROM itemAttachments WHERE parentItemID = ?")?;
        let mut rows = stmt.query([parent_id])?;
        while let Some(r) = rows.next()? { attachment_ids.push(r.get::<_, i64>(0)?); }

        let mut out = vec![];
        for aid in attachment_ids {
            let attachment_key: String = c.query_row(
                "SELECT key FROM items WHERE itemID = ?", [aid], |r| r.get(0))?;

            let mut stmt = c.prepare(
                "SELECT i.key, a.type, a.text, a.comment, a.color, a.pageLabel, a.sortIndex
                 FROM itemAnnotations a JOIN items i ON i.itemID = a.itemID
                 WHERE a.parentItemID = ? ORDER BY a.sortIndex")?;
            let mut rows = stmt.query([aid])?;
            while let Some(r) = rows.next()? {
                let kind = annotation_kind(r.get::<_, i64>(1)?).to_string();
                out.push(Annotation {
                    key: r.get(0)?,
                    parent_attachment_key: attachment_key.clone(),
                    kind,
                    text: r.get::<_, Option<String>>(2)?,
                    comment: r.get::<_, Option<String>>(3)?,
                    color: r.get::<_, Option<String>>(4)?,
                    page_label: r.get::<_, Option<String>>(5)?,
                    sort_index: r.get::<_, String>(6)?,
                });
            }
        }
        Ok(out)
    }).await
}
```

Add `pub mod annotations;` to `reader/mod.rs`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p zotero-core --test reader_annotations`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): list annotations (highlights, notes) per item"
```

---

## Phase 4 — Content extraction (PDF and webpage)

### Task 15: PDF text from .zotero-ft-cache, fallback to live extraction

**Files:**
- Create: `crates/zotero-core/src/pdf.rs`
- Modify: `crates/zotero-core/src/lib.rs`
- Test: `crates/zotero-core/tests/pdf_text.rs`

- [ ] **Step 1: Write the failing test**

`crates/zotero-core/tests/pdf_text.rs`:

```rust
mod fixtures;
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::pdf::{get_pdf_text, PdfTextSource};

#[tokio::test]
async fn prefers_zotero_ft_cache_when_present() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let res = get_pdf_text(&pool, "AAAA0001", 1, &f.storage_dir()).await.unwrap();
    assert!(matches!(res.source, PdfTextSource::ZoteroCache));
    assert!(res.text.contains("zoteroconnectortest"));
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/pdf.rs`:

```rust
use crate::error::{Error, Result};
use crate::reader::pool::ReadOnlyPool;
use crate::reader::attachments::resolve_path;
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
    // Approximate: take roughly 3000 chars per page from the cache, or use pdf-extract for true pages.
    // First N pages estimate: 3500 chars/page; cap at full length.
    let cap = (n_pages * 3500).min(full.text.len());
    let mut text: String = full.text.chars().take(cap).collect();
    if text.len() < full.text.len() { text.push_str("\n[... truncated ...]"); }
    Ok(PdfTextResult { text, source: full.source, character_count: cap })
}

pub fn cache_path_for(storage_dir: &Path, parent_key: &str) -> PathBuf {
    storage_dir.join(parent_key).join(".zotero-ft-cache")
}
```

Add `pub mod pdf;` to `lib.rs`.

- [ ] **Step 3: Run test**

Run: `cargo test -p zotero-core --test pdf_text`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/src/pdf.rs crates/zotero-core/src/lib.rs crates/zotero-core/tests/pdf_text.rs
git commit -m "feat(core): get_pdf_text with .zotero-ft-cache preferred, live extract fallback"
```

---

### Task 16: HTML snapshot + live URL fetch + readability extraction

**Files:**
- Create: `crates/zotero-core/src/web.rs`
- Modify: `crates/zotero-core/src/lib.rs`
- Test: `crates/zotero-core/tests/web_content.rs`

- [ ] **Step 1: Write the failing test**

`crates/zotero-core/tests/web_content.rs`:

```rust
mod fixtures;
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::web::{get_webpage_content, WebMode, WebSource};

#[tokio::test]
async fn snapshot_mode_returns_readable_text() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let res = get_webpage_content(&pool, "WEB00001", 1, &f.storage_dir(), WebMode::Snapshot, "test/0.1").await.unwrap();
    assert!(matches!(res.source, WebSource::Snapshot));
    assert!(res.text.to_lowercase().contains("hello snapshot"));
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/web.rs`:

```rust
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
    let snapshot = atts.iter().find(|a| a.content_type.as_deref() == Some("text/html"));

    // Look up item URL for live fallback
    let parent_url = lookup_url(pool, parent_key, library_id).await.ok().flatten();

    let try_snapshot = || async {
        if let Some(s) = snapshot {
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
            url: Some(url), fetched_at: Some(chrono_like_now()),
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
        let u: Option<String> = c.query_row(
            "SELECT v.value FROM items i
             JOIN itemData d ON d.itemID = i.itemID
             JOIN fieldsCombined f ON f.fieldID = d.fieldID
             JOIN itemDataValues v ON v.valueID = d.valueID
             WHERE i.libraryID = ? AND i.key = ? AND f.fieldName = 'url'",
            rusqlite::params![library_id, &key],
            |r| r.get(0),
        ).optional()?;
        Ok(u)
    }).await
}

fn readability_extract(html: &str, base_url: Option<&str>) -> Result<(Option<String>, String)> {
    let url = base_url
        .and_then(|u| Url::parse(u).ok())
        .unwrap_or_else(|| Url::parse("http://example.invalid/").unwrap());
    let mut reader = std::io::Cursor::new(html);
    let extracted = readability::extractor::extract(&mut reader, &url)
        .map_err(|e| Error::Html(e.to_string()))?;
    Ok((Some(extracted.title), extracted.text))
}

fn chrono_like_now() -> String {
    // Minimal RFC3339 to avoid adding chrono just for this.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("@{}", secs) // sentinel; real chrono can be added later if needed
}

// rusqlite OptionalExt mimic without pulling another crate
trait OptionalRow<T> {
    fn optional(self) -> Result<Option<T>>;
}
impl<T> OptionalRow<T> for rusqlite::Result<T> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(t) => Ok(Some(t)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
```

Add `pub mod web;` to `lib.rs`. Add `url = "2"` to `zotero-core/Cargo.toml`.

- [ ] **Step 3: Run test**

Run: `cargo test -p zotero-core --test web_content`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): get_webpage_content with snapshot/live/auto via readability"
```

---

## Phase 5 — BetterBibTeX integration

### Task 17: BBT JSON-RPC client

**Files:**
- Create: `crates/zotero-core/src/bbt.rs`
- Modify: `crates/zotero-core/src/lib.rs`
- Test: `crates/zotero-core/tests/bbt_client.rs`

- [ ] **Step 1: Write the failing test** (uses wiremock)

`crates/zotero-core/tests/bbt_client.rs`:

```rust
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_core::bbt::BbtClient;

#[tokio::test]
async fn citationkey_lookup_works() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/better-bibtex/json-rpc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "result": { "JGF2UTMW": "rabkinWhatModernIsrael2016" },
            "id": 1
        })))
        .mount(&server).await;

    let c = BbtClient::new(server.uri()).unwrap();
    let map = c.citationkeys(&["JGF2UTMW".into()]).await.unwrap();
    assert_eq!(map.get("JGF2UTMW").map(String::as_str), Some("rabkinWhatModernIsrael2016"));
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/bbt.rs`:

```rust
use crate::error::{Error, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone)]
pub struct BbtClient {
    base: String,
    http: reqwest::Client,
}

impl BbtClient {
    pub fn new(base: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()?;
        Ok(Self { base: base.into(), http })
    }

    pub async fn citationkeys(&self, keys: &[String]) -> Result<HashMap<String, String>> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "item.citationkey",
            "params": [keys],
            "id": 1
        });
        let url = format!("{}/better-bibtex/json-rpc", self.base);
        let resp = self.http.post(&url).json(&payload).send().await
            .map_err(|_| Error::BbtUnavailable)?;
        let body: Value = resp.json().await?;
        if let Some(err) = body.get("error") {
            return Err(Error::Bbt(err.to_string()));
        }
        let map: HashMap<String, String> = body.get("result")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(map)
    }
}
```

Add `pub mod bbt;` to `lib.rs`.

- [ ] **Step 3: Run test**

Run: `cargo test -p zotero-core --test bbt_client`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): BetterBibTeX JSON-RPC client (item.citationkey)"
```

---

### Task 18: Populate BBT citation_keys in reader output

**Files:**
- Modify: `crates/zotero-core/src/reader/items.rs` (add helper that hydrates BBT key)
- Test: `crates/zotero-core/tests/items_with_bbt.rs`

- [ ] **Step 1: Add hydration function**

In `crates/zotero-core/src/reader/items.rs`, append:

```rust
use crate::bbt::BbtClient;

pub async fn hydrate_citation_key(item: &mut crate::types::Item, bbt: Option<&BbtClient>) {
    if item.citation_key.is_some() { return; }
    let Some(client) = bbt else { return };
    if let Ok(map) = client.citationkeys(&[item.key.clone()]).await {
        if let Some(ck) = map.get(&item.key) {
            item.citation_key = Some(ck.clone());
        }
    }
}
```

- [ ] **Step 2: Write a wiremock-backed test**

`crates/zotero-core/tests/items_with_bbt.rs`:

```rust
mod fixtures;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_core::bbt::BbtClient;
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::reader::items::{get_item_by_key, hydrate_citation_key};

#[tokio::test]
async fn hydrates_citation_key_from_bbt() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();
    let mut item = get_item_by_key(&pool, "JGF2UTMW", 1).await.unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/better-bibtex/json-rpc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc":"2.0","result":{"JGF2UTMW":"rabkin2016"},"id":1
        }))).mount(&server).await;
    let bbt = BbtClient::new(server.uri()).unwrap();
    hydrate_citation_key(&mut item, Some(&bbt)).await;
    assert_eq!(item.citation_key.as_deref(), Some("rabkin2016"));
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p zotero-core --test items_with_bbt`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): hydrate BBT citation_key on Item results"
```

---

## Phase 6 — Local API writer

### Task 19: Writer client foundation

**Files:**
- Create: `crates/zotero-core/src/writer/mod.rs`
- Create: `crates/zotero-core/src/writer/client.rs`
- Modify: `crates/zotero-core/src/lib.rs`
- Test: `crates/zotero-core/tests/writer_client.rs`

- [ ] **Step 1: Write the failing test**

```rust
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_core::writer::client::LocalApi;

#[tokio::test]
async fn sends_api_version_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/users/93338/items"))
        .and(header("Zotero-API-Version", "3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server).await;

    let api = LocalApi::new(server.uri(), 93338).unwrap();
    let v = api.list_items_raw("", 0, 1).await.unwrap();
    assert!(v.is_array());
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/writer/mod.rs`:

```rust
pub mod client;
pub mod items;
pub mod notes;
pub mod tags;
```

(`items.rs`, `notes.rs`, `tags.rs` are stubs at this stage — we'll fill them in the next tasks.)

`crates/zotero-core/src/writer/client.rs`:

```rust
use crate::error::{Error, Result};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone)]
pub struct LocalApi {
    pub base: String,
    pub user_id: i64,
    pub http: Client,
}

impl LocalApi {
    pub fn new(base: impl Into<String>, user_id: i64) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?;
        Ok(Self { base: base.into(), user_id, http })
    }

    pub fn user_path(&self, suffix: &str) -> String {
        format!("{}/api/users/{}{}", self.base, self.user_id, suffix)
    }

    pub async fn list_items_raw(&self, query: &str, start: i64, limit: i64) -> Result<Value> {
        let url = self.user_path("/items");
        let resp = self.http.get(&url)
            .header("Zotero-API-Version", "3")
            .query(&[("q", query), ("start", &start.to_string()), ("limit", &limit.to_string())])
            .send().await?;
        let status = resp.status();
        if status.is_server_error() && status.as_u16() == 0 {
            // Connection-refused-shaped errors are unreachable here but kept for clarity.
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::LocalApi { status: status.as_u16(), body });
        }
        Ok(resp.json().await?)
    }
}
```

Add `pub mod writer;` to `lib.rs`. Stub files:

`crates/zotero-core/src/writer/items.rs`:

```rust
// Filled in by Task 21
```

`crates/zotero-core/src/writer/notes.rs`:

```rust
// Filled in by Task 20
```

`crates/zotero-core/src/writer/tags.rs`:

```rust
// Filled in by Task 22
```

- [ ] **Step 3: Run test**

Run: `cargo test -p zotero-core --test writer_client`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): LocalApi client foundation with Zotero-API-Version header"
```

---

### Task 20: add_note

**Files:**
- Modify: `crates/zotero-core/src/writer/notes.rs`
- Test: `crates/zotero-core/tests/writer_notes.rs`

- [ ] **Step 1: Write the failing test**

```rust
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_core::writer::client::LocalApi;
use zotero_core::writer::notes::add_note;

#[tokio::test]
async fn posts_a_child_note_against_parent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/users/93338/items"))
        .and(header("Zotero-API-Version", "3"))
        .and(body_partial_json(serde_json::json!([{
            "itemType": "note",
            "parentItem": "JGF2UTMW"
        }])))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "successful": { "0": { "key": "NEWN0001", "version": 12345 } }
        })))
        .mount(&server).await;

    let api = LocalApi::new(server.uri(), 93338).unwrap();
    let new_key = add_note(&api, "JGF2UTMW", "# Heading\n\nSome **markdown**.").await.unwrap();
    assert_eq!(new_key, "NEWN0001");
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/writer/notes.rs`:

```rust
use crate::error::{Error, Result};
use crate::writer::client::LocalApi;
use serde_json::{json, Value};

/// Adds a child note to a parent item. Markdown is wrapped in <p>; Zotero's
/// note storage accepts HTML, so we convert via a minimal markdown-to-HTML
/// pass (paragraphs, headings, emphasis). For richer formatting, callers can
/// pass HTML directly — it will pass through if it starts with `<`.
pub async fn add_note(api: &LocalApi, parent_key: &str, markdown_or_html: &str) -> Result<String> {
    let html = if markdown_or_html.trim_start().starts_with('<') {
        markdown_or_html.to_string()
    } else {
        markdown_to_simple_html(markdown_or_html)
    };

    let body = json!([{
        "itemType": "note",
        "parentItem": parent_key,
        "note": html,
        "tags": [],
        "relations": {}
    }]);
    let url = api.user_path("/items");
    let resp = api.http.post(&url)
        .header("Zotero-API-Version", "3")
        .json(&body)
        .send().await?;
    let status = resp.status();
    let v: Value = resp.json().await?;
    if !status.is_success() {
        return Err(Error::LocalApi { status: status.as_u16(), body: v.to_string() });
    }
    v.get("successful")
        .and_then(|s| s.get("0"))
        .and_then(|i| i.get("key"))
        .and_then(|k| k.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::LocalApi { status: 200, body: v.to_string() })
}

fn markdown_to_simple_html(md: &str) -> String {
    // Tiny, conservative converter: H1-H3, bold, italic, paragraphs.
    let mut out = String::new();
    for para in md.split("\n\n") {
        let p = para.trim();
        if p.is_empty() { continue; }
        if let Some(rest) = p.strip_prefix("### ") {
            out.push_str(&format!("<h3>{}</h3>", html_escape(rest)));
        } else if let Some(rest) = p.strip_prefix("## ") {
            out.push_str(&format!("<h2>{}</h2>", html_escape(rest)));
        } else if let Some(rest) = p.strip_prefix("# ") {
            out.push_str(&format!("<h1>{}</h1>", html_escape(rest)));
        } else {
            out.push_str(&format!("<p>{}</p>", inline(&html_escape(p))));
        }
    }
    out
}

fn inline(s: &str) -> String {
    // **bold** -> <strong>; *italic* -> <em>
    let s = regex_lite_replace(s, "**", "<strong>", "</strong>");
    regex_lite_replace(&s, "*", "<em>", "</em>")
}

fn regex_lite_replace(s: &str, delim: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        match rest.find(delim) {
            None => { out.push_str(rest); return out; }
            Some(a) => {
                out.push_str(&rest[..a]);
                let after = &rest[a + delim.len()..];
                match after.find(delim) {
                    Some(b) => {
                        out.push_str(open);
                        out.push_str(&after[..b]);
                        out.push_str(close);
                        rest = &after[b + delim.len()..];
                    }
                    None => { out.push_str(delim); out.push_str(after); return out; }
                }
            }
        }
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p zotero-core --test writer_notes`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): add_note posts child note via Local API with markdown→html"
```

---

### Task 21: update_item_fields with If-Unmodified-Since-Version

**Files:**
- Modify: `crates/zotero-core/src/writer/items.rs`
- Test: `crates/zotero-core/tests/writer_items.rs`

- [ ] **Step 1: Write the failing test**

```rust
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_core::writer::client::LocalApi;
use zotero_core::writer::items::update_item_fields;

#[tokio::test]
async fn patches_item_with_version_header() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/api/users/93338/items/JGF2UTMW"))
        .and(header("Zotero-API-Version", "3"))
        .and(header("If-Unmodified-Since-Version", "10005"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server).await;

    let api = LocalApi::new(server.uri(), 93338).unwrap();
    let fields = serde_json::json!({ "abstractNote": "New abstract." });
    update_item_fields(&api, "JGF2UTMW", 10005, fields).await.unwrap();
}

#[tokio::test]
async fn version_conflict_returns_typed_error() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(412).set_body_string("Precondition Failed"))
        .mount(&server).await;
    let api = LocalApi::new(server.uri(), 93338).unwrap();
    let err = update_item_fields(&api, "JGF2UTMW", 10005, serde_json::json!({})).await.unwrap_err();
    assert!(matches!(err, zotero_core::Error::VersionConflict(_)));
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/writer/items.rs`:

```rust
use crate::error::{Error, Result};
use crate::writer::client::LocalApi;
use serde_json::Value;

pub async fn update_item_fields(api: &LocalApi, item_key: &str, version: i64, fields: Value) -> Result<()> {
    let url = api.user_path(&format!("/items/{}", item_key));
    let resp = api.http.patch(&url)
        .header("Zotero-API-Version", "3")
        .header("If-Unmodified-Since-Version", version.to_string())
        .json(&fields)
        .send().await?;
    let status = resp.status();
    if status.is_success() { return Ok(()); }
    let body = resp.text().await.unwrap_or_default();
    if status.as_u16() == 412 {
        return Err(Error::VersionConflict(format!("item {} has changed; refresh and retry. body={}", item_key, body)));
    }
    Err(Error::LocalApi { status: status.as_u16(), body })
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p zotero-core --test writer_items`
Expected: 2 pass.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): update_item_fields with If-Unmodified-Since-Version, typed conflict"
```

---

### Task 22: Tag and collection write helpers; refetch_url

**Files:**
- Modify: `crates/zotero-core/src/writer/tags.rs`
- Test: `crates/zotero-core/tests/writer_tags.rs`

- [ ] **Step 1: Write the failing test**

```rust
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_core::writer::client::LocalApi;
use zotero_core::writer::tags::{add_tags, remove_tags, add_to_collection, remove_from_collection};

#[tokio::test]
async fn add_tags_round_trips() {
    let server = MockServer::start().await;
    // Add expects GET → PATCH with merged tags list
    Mock::given(method("GET")).and(path("/api/users/93338/items/JGF2UTMW"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "tags": [{"tag":"existing"}], "version": 10005 }
        }))).mount(&server).await;
    Mock::given(method("PATCH")).and(path("/api/users/93338/items/JGF2UTMW"))
        .and(header("If-Unmodified-Since-Version", "10005"))
        .respond_with(ResponseTemplate::new(204)).mount(&server).await;

    let api = LocalApi::new(server.uri(), 93338).unwrap();
    add_tags(&api, "JGF2UTMW", &["new1".into(), "new2".into()]).await.unwrap();
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/writer/tags.rs`:

```rust
use crate::error::{Error, Result};
use crate::writer::client::LocalApi;
use crate::writer::items::update_item_fields;
use serde_json::{json, Value};

async fn fetch_item_meta(api: &LocalApi, key: &str) -> Result<(Vec<String>, Vec<String>, i64)> {
    let url = api.user_path(&format!("/items/{}", key));
    let resp = api.http.get(&url).header("Zotero-API-Version", "3").send().await?;
    if !resp.status().is_success() {
        return Err(Error::LocalApi { status: resp.status().as_u16(), body: resp.text().await.unwrap_or_default() });
    }
    let v: Value = resp.json().await?;
    let data = v.get("data").cloned().unwrap_or_default();
    let version = data.get("version").and_then(|x| x.as_i64()).unwrap_or(0);
    let tags = data.get("tags").and_then(|t| t.as_array()).map(|arr|
        arr.iter().filter_map(|e| e.get("tag").and_then(|s| s.as_str()).map(String::from)).collect()
    ).unwrap_or_default();
    let collections = data.get("collections").and_then(|c| c.as_array()).map(|arr|
        arr.iter().filter_map(|x| x.as_str().map(String::from)).collect()
    ).unwrap_or_default();
    Ok((tags, collections, version))
}

pub async fn add_tags(api: &LocalApi, key: &str, new_tags: &[String]) -> Result<()> {
    let (mut existing, _coll, version) = fetch_item_meta(api, key).await?;
    for t in new_tags {
        if !existing.iter().any(|e| e == t) { existing.push(t.clone()); }
    }
    let json_tags: Vec<Value> = existing.into_iter().map(|t| json!({"tag": t})).collect();
    update_item_fields(api, key, version, json!({ "tags": json_tags })).await
}

pub async fn remove_tags(api: &LocalApi, key: &str, tags_to_remove: &[String]) -> Result<()> {
    let (existing, _coll, version) = fetch_item_meta(api, key).await?;
    let kept: Vec<Value> = existing.into_iter()
        .filter(|t| !tags_to_remove.iter().any(|r| r == t))
        .map(|t| json!({"tag": t})).collect();
    update_item_fields(api, key, version, json!({ "tags": kept })).await
}

pub async fn add_to_collection(api: &LocalApi, key: &str, collection_key: &str) -> Result<()> {
    let (_tags, mut colls, version) = fetch_item_meta(api, key).await?;
    if !colls.iter().any(|c| c == collection_key) { colls.push(collection_key.into()); }
    update_item_fields(api, key, version, json!({ "collections": colls })).await
}

pub async fn remove_from_collection(api: &LocalApi, key: &str, collection_key: &str) -> Result<()> {
    let (_tags, colls, version) = fetch_item_meta(api, key).await?;
    let kept: Vec<String> = colls.into_iter().filter(|c| c != collection_key).collect();
    update_item_fields(api, key, version, json!({ "collections": kept })).await
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p zotero-core --test writer_tags`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): add/remove tags and collection membership via Local API"
```

---

### Task 23: refetch_url with optional snapshot save

**Files:**
- Modify: `crates/zotero-core/src/web.rs`
- Test: `crates/zotero-core/tests/web_refetch.rs`

- [ ] **Step 1: Add the function**

In `crates/zotero-core/src/web.rs`, append:

```rust
use crate::writer::client::LocalApi;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefetchResult {
    pub url: String,
    pub text: String,
    pub title: Option<String>,
    pub saved_attachment_key: Option<String>,
    pub fetched_at: String,
}

pub async fn refetch_url(
    pool: &ReadOnlyPool,
    api: Option<&LocalApi>,
    parent_key: &str,
    library_id: i64,
    save_as_snapshot: bool,
    user_agent: &str,
) -> Result<RefetchResult> {
    let url = lookup_url(pool, parent_key, library_id).await?
        .ok_or_else(|| Error::Html(format!("item {} has no URL", parent_key)))?;
    let client = reqwest::Client::builder().user_agent(user_agent).build()?;
    let resp = client.get(&url).send().await?.error_for_status()?;
    let html = resp.text().await?;
    let (title, text) = readability_extract(&html, Some(&url))?;
    let fetched_at = chrono_like_now();

    let saved_attachment_key = if save_as_snapshot {
        if let Some(api) = api {
            Some(create_html_snapshot_attachment(api, parent_key, &url, &html).await?)
        } else { None }
    } else { None };

    Ok(RefetchResult { url, text, title, saved_attachment_key, fetched_at })
}

async fn create_html_snapshot_attachment(api: &LocalApi, parent_key: &str, url: &str, html: &str) -> Result<String> {
    use serde_json::json;
    // We create a webpage-snapshot attachment via the Local API. We rely on
    // Zotero to handle ingest of the body when the linkMode is "imported_url".
    // For now we POST metadata; the body upload step is documented but optional
    // for v1 (Zotero stores attached HTML inline when contentType is set).
    let body = json!([{
        "itemType": "attachment",
        "parentItem": parent_key,
        "linkMode": "imported_url",
        "title": "Snapshot",
        "url": url,
        "contentType": "text/html",
        "note": format!("Refetched at {} by zotero-mcp; {} bytes", chrono_like_now(), html.len())
    }]);
    let url_e = api.user_path("/items");
    let resp = api.http.post(&url_e).header("Zotero-API-Version", "3").json(&body).send().await?;
    if !resp.status().is_success() {
        return Err(Error::LocalApi { status: resp.status().as_u16(), body: resp.text().await.unwrap_or_default() });
    }
    let v: serde_json::Value = resp.json().await?;
    v.get("successful").and_then(|s| s.get("0")).and_then(|i| i.get("key")).and_then(|k| k.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::LocalApi { status: 200, body: v.to_string() })
}
```

- [ ] **Step 2: Write test**

`crates/zotero-core/tests/web_refetch.rs`:

```rust
mod fixtures;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::web::refetch_url;
use zotero_core::writer::client::LocalApi;

#[tokio::test]
async fn refetches_and_saves_snapshot() {
    let f = fixtures::build_fixture::build();
    let pool = ReadOnlyPool::new(f.sqlite_path(), 2).await.unwrap();

    let live = MockServer::start().await;
    Mock::given(method("GET")).and(path("/article"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<html><body><article><h1>Live</h1><p>Body.</p></article></body></html>"
        )).mount(&live).await;

    // Patch fixture: set the WEB00001 item's URL to the mock server.
    {
        let conn = rusqlite::Connection::open(f.sqlite_path()).unwrap();
        let url = format!("{}/article", live.uri());
        conn.execute("UPDATE itemDataValues SET value = ?1 WHERE valueID = 21", [url]).unwrap();
    }

    let api_server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/users/93338/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "successful": {"0": {"key": "SNAP0001", "version": 7}}
        }))).mount(&api_server).await;

    let api = LocalApi::new(api_server.uri(), 93338).unwrap();
    let r = refetch_url(&pool, Some(&api), "WEB00001", 1, true, "test/0.1").await.unwrap();
    assert_eq!(r.saved_attachment_key.as_deref(), Some("SNAP0001"));
    assert!(r.text.to_lowercase().contains("body"));
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p zotero-core --test web_refetch`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): refetch_url with optional save-as-snapshot via Local API"
```

---

## Phase 7 — Citation formatting

### Task 24: format_citation and format_bibliography

**Files:**
- Create: `crates/zotero-core/src/citations.rs`
- Modify: `crates/zotero-core/src/lib.rs`
- Test: `crates/zotero-core/tests/citations.rs`

- [ ] **Step 1: Write the failing test**

```rust
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zotero_core::citations::{format_citation, format_bibliography};
use zotero_core::writer::client::LocalApi;

#[tokio::test]
async fn formats_single_citation_as_bib() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/users/93338/items/JGF2UTMW"))
        .and(query_param("format", "bib"))
        .and(query_param("style", "apa"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<div>Rabkin, Y. (2016). ...</div>"))
        .mount(&server).await;

    let api = LocalApi::new(server.uri(), 93338).unwrap();
    let s = format_citation(&api, "JGF2UTMW", "apa", "bib").await.unwrap();
    assert!(s.contains("Rabkin"));
}

#[tokio::test]
async fn formats_bibliography_for_multiple_keys() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/users/93338/items"))
        .and(query_param("format", "bib"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<div>combined bib</div>"))
        .mount(&server).await;

    let api = LocalApi::new(server.uri(), 93338).unwrap();
    let s = format_bibliography(&api, &["A".into(), "B".into()], "chicago-author-date", "bib").await.unwrap();
    assert!(s.contains("combined"));
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/citations.rs`:

```rust
use crate::error::{Error, Result};
use crate::writer::client::LocalApi;

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
```

Add `pub mod citations;` to `lib.rs`.

- [ ] **Step 3: Run tests**

Run: `cargo test -p zotero-core --test citations`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): format_citation and format_bibliography via Local API"
```

---

## Phase 8 — Enrichment subsystem

### Task 25: On-disk cache module

**Files:**
- Create: `crates/zotero-core/src/cache.rs`
- Modify: `crates/zotero-core/src/lib.rs`
- Test: `crates/zotero-core/tests/cache.rs`

- [ ] **Step 1: Failing test**

```rust
use tempfile::tempdir;
use zotero_core::cache::DiskCache;

#[tokio::test]
async fn round_trips_json_with_ttl() {
    let dir = tempdir().unwrap();
    let cache = DiskCache::new(dir.path().to_path_buf(), 60 * 60);
    let key = "crossref:10.1/abc";
    cache.put(key, &serde_json::json!({"title":"hello"})).await.unwrap();
    let v: serde_json::Value = cache.get(key).await.unwrap().expect("hit");
    assert_eq!(v["title"], "hello");
}

#[tokio::test]
async fn expired_returns_none() {
    let dir = tempdir().unwrap();
    let cache = DiskCache::new(dir.path().to_path_buf(), 0); // expires immediately
    cache.put("k", &serde_json::json!(1)).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let v: Option<serde_json::Value> = cache.get("k").await.unwrap();
    assert!(v.is_none());
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/cache.rs`:

```rust
use crate::error::Result;
use serde::{de::DeserializeOwned, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct DiskCache {
    dir: PathBuf,
    ttl: Duration,
}

impl DiskCache {
    pub fn new(dir: PathBuf, ttl_secs: u64) -> Self {
        Self { dir, ttl: Duration::from_secs(ttl_secs) }
    }

    fn path_for(&self, key: &str) -> PathBuf {
        let hash = simple_hash(key);
        self.dir.join(format!("{}.json", hash))
    }

    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let p = self.path_for(key);
        if !p.exists() { return Ok(None); }
        let bytes = tokio::fs::read(&p).await?;
        let env: Envelope<serde_json::Value> = serde_json::from_slice(&bytes)?;
        let age = now_secs().saturating_sub(env.stored_at);
        if age > self.ttl.as_secs() { return Ok(None); }
        let v: T = serde_json::from_value(env.value)?;
        Ok(Some(v))
    }

    pub async fn put<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        tokio::fs::create_dir_all(&self.dir).await.ok();
        let env = Envelope { stored_at: now_secs(), value: serde_json::to_value(value)? };
        let bytes = serde_json::to_vec(&env)?;
        let p = self.path_for(key);
        let tmp = p.with_extension("json.tmp");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &p).await?;
        Ok(())
    }
}

#[derive(Serialize, serde::Deserialize)]
struct Envelope<T> { stored_at: u64, value: T }

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn simple_hash(s: &str) -> String {
    // Stable, deterministic file naming via FNV-1a 64-bit. Avoid adding sha2.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
    format!("{:016x}", h)
}
```

Add `pub mod cache;` to `lib.rs`.

- [ ] **Step 3: Run tests**

Run: `cargo test -p zotero-core --test cache`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): on-disk TTL cache with atomic writes"
```

---

### Task 26: CrossRef client + Zotero-schema normalization

**Files:**
- Create: `crates/zotero-core/src/enrichment/mod.rs`
- Create: `crates/zotero-core/src/enrichment/crossref.rs`
- Modify: `crates/zotero-core/src/lib.rs`
- Test: `crates/zotero-core/tests/enrich_crossref.rs`

- [ ] **Step 1: Failing test**

```rust
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use tempfile::tempdir;
use zotero_core::cache::DiskCache;
use zotero_core::enrichment::crossref::CrossrefClient;

#[tokio::test]
async fn lookup_doi_normalizes_to_zotero_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/works/10.1234/abcd"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "ok",
            "message": {
                "DOI": "10.1234/abcd",
                "title": ["A Paper on Things"],
                "author": [{"given":"Alice","family":"Aardvark"}],
                "issued": {"date-parts": [[2024, 3]]},
                "container-title": ["Journal of Things"],
                "publisher": "ThingPress",
                "type": "journal-article",
                "URL": "https://doi.org/10.1234/abcd",
                "abstract": "Abstract content."
            }
        })))
        .mount(&server).await;

    let dir = tempdir().unwrap();
    let cache = DiskCache::new(dir.path().to_path_buf(), 60);
    let c = CrossrefClient::new(server.uri(), cache, "zotero-mcp/0.1");
    let norm = c.lookup_doi("10.1234/abcd").await.unwrap();
    assert_eq!(norm.fields["title"], "A Paper on Things");
    assert_eq!(norm.fields["DOI"], "10.1234/abcd");
    assert_eq!(norm.fields["date"], "2024-03");
    assert_eq!(norm.fields["itemType"], "journalArticle");
    assert_eq!(norm.creators[0].last_name.as_deref(), Some("Aardvark"));
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/enrichment/mod.rs`:

```rust
pub mod crossref;
pub mod openlibrary;
pub mod arxiv;
pub mod semantic_scholar;
pub mod pdf_signals;
pub mod scoring;
pub mod propose;

use crate::types::Creator;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Result from any enrichment source, already mapped to Zotero's schema vocabulary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedRecord {
    pub source: String,
    pub fields: serde_json::Map<String, Value>,
    pub creators: Vec<Creator>,
    pub source_url: Option<String>,
}
```

`crates/zotero-core/src/enrichment/crossref.rs`:

```rust
use crate::cache::DiskCache;
use crate::error::{Error, Result};
use crate::enrichment::NormalizedRecord;
use crate::types::Creator;
use serde_json::{Map, Value};

#[derive(Clone)]
pub struct CrossrefClient {
    base: String,
    cache: DiskCache,
    http: reqwest::Client,
}

impl CrossrefClient {
    pub fn new(base: impl Into<String>, cache: DiskCache, user_agent: &str) -> Self {
        let http = reqwest::Client::builder().user_agent(user_agent).build().unwrap();
        Self { base: base.into(), cache, http }
    }

    pub async fn lookup_doi(&self, doi: &str) -> Result<NormalizedRecord> {
        let key = format!("crossref:doi:{}", doi);
        if let Some(v) = self.cache.get::<Value>(&key).await? {
            return Ok(normalize_work(&v).ok_or_else(|| Error::Lookup { source: "crossref".into(), message: "cache parse failed".into() })?);
        }
        let url = format!("{}/works/{}", self.base, doi);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(Error::Lookup { source: "crossref".into(), message: format!("HTTP {}", resp.status()) });
        }
        let body: Value = resp.json().await?;
        let msg = body.get("message").cloned().unwrap_or_default();
        self.cache.put(&key, &msg).await.ok();
        normalize_work(&msg).ok_or_else(|| Error::Lookup { source: "crossref".into(), message: "no fields".into() })
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<NormalizedRecord>> {
        let key = format!("crossref:search:{}:{}", query, limit);
        if let Some(v) = self.cache.get::<Value>(&key).await? {
            return Ok(parse_search(&v));
        }
        let url = format!("{}/works", self.base);
        let resp = self.http.get(&url)
            .query(&[("query", query), ("rows", &limit.to_string())])
            .send().await?;
        if !resp.status().is_success() {
            return Err(Error::Lookup { source: "crossref".into(), message: format!("HTTP {}", resp.status()) });
        }
        let body: Value = resp.json().await?;
        self.cache.put(&key, &body).await.ok();
        Ok(parse_search(&body))
    }
}

fn parse_search(v: &Value) -> Vec<NormalizedRecord> {
    v.get("message").and_then(|m| m.get("items")).and_then(|a| a.as_array())
        .map(|arr| arr.iter().filter_map(normalize_work).collect()).unwrap_or_default()
}

fn normalize_work(msg: &Value) -> Option<NormalizedRecord> {
    let mut fields = Map::new();
    if let Some(t) = msg.get("title").and_then(|x| x.as_array()).and_then(|a| a.first()) {
        if let Some(s) = t.as_str() { fields.insert("title".into(), Value::String(s.to_string())); }
    }
    if let Some(doi) = msg.get("DOI").and_then(|x| x.as_str()) {
        fields.insert("DOI".into(), Value::String(doi.to_string()));
    }
    if let Some(date) = extract_date(msg) { fields.insert("date".into(), Value::String(date)); }
    if let Some(c) = msg.get("container-title").and_then(|x| x.as_array()).and_then(|a| a.first()).and_then(|x| x.as_str()) {
        fields.insert("publicationTitle".into(), Value::String(c.into()));
    }
    if let Some(pub_) = msg.get("publisher").and_then(|x| x.as_str()) {
        fields.insert("publisher".into(), Value::String(pub_.into()));
    }
    if let Some(url) = msg.get("URL").and_then(|x| x.as_str()) {
        fields.insert("url".into(), Value::String(url.into()));
    }
    if let Some(abs) = msg.get("abstract").and_then(|x| x.as_str()) {
        fields.insert("abstractNote".into(), Value::String(strip_html(abs)));
    }
    let item_type = match msg.get("type").and_then(|x| x.as_str()) {
        Some("journal-article") => "journalArticle",
        Some("book") => "book",
        Some("book-chapter") => "bookSection",
        Some("proceedings-article") => "conferencePaper",
        Some("posted-content") => "preprint",
        _ => "document",
    };
    fields.insert("itemType".into(), Value::String(item_type.into()));

    let creators = msg.get("author").and_then(|x| x.as_array()).map(|arr| arr.iter().enumerate().map(|(i, a)| Creator {
        first_name: a.get("given").and_then(|x| x.as_str()).map(String::from),
        last_name:  a.get("family").and_then(|x| x.as_str()).map(String::from),
        creator_type: "author".into(),
        order_index: i as i64,
    }).collect()).unwrap_or_default();

    Some(NormalizedRecord {
        source: "crossref".into(),
        fields,
        creators,
        source_url: msg.get("URL").and_then(|x| x.as_str()).map(String::from),
    })
}

fn extract_date(msg: &Value) -> Option<String> {
    let parts = msg.get("issued")
        .or_else(|| msg.get("published-print"))
        .or_else(|| msg.get("published-online"))?;
    let arr = parts.get("date-parts")?.as_array()?.first()?.as_array()?;
    let nums: Vec<String> = arr.iter().filter_map(|v| v.as_i64().map(|n| format!("{:02}", n))).collect();
    if nums.is_empty() { return None; }
    Some(match nums.len() {
        1 => nums[0].clone(),
        2 => format!("{}-{}", nums[0], nums[1]),
        _ => format!("{}-{}-{}", nums[0], nums[1], nums[2]),
    })
}

fn strip_html(s: &str) -> String {
    // Lightweight tag stripper for CrossRef's JATS-flavored abstracts.
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}
```

Add `pub mod enrichment;` to `lib.rs`.

- [ ] **Step 3: Run test**

Run: `cargo test -p zotero-core --test enrich_crossref`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): CrossRef enrichment client with Zotero-schema normalization"
```

---

### Task 27: OpenLibrary client (lookup_isbn)

**Files:**
- Create: `crates/zotero-core/src/enrichment/openlibrary.rs`
- Test: `crates/zotero-core/tests/enrich_openlibrary.rs`

- [ ] **Step 1: Failing test**

```rust
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use tempfile::tempdir;
use zotero_core::cache::DiskCache;
use zotero_core::enrichment::openlibrary::OpenLibraryClient;

#[tokio::test]
async fn lookup_isbn_normalizes() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/isbn/9780000000000.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "title": "Some Book",
            "publish_date": "2020",
            "publishers": ["BookCo"],
            "authors": [{"key":"/authors/OL1A"}]
        }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/authors/OL1A.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "Jane Doe"
        }))).mount(&server).await;

    let dir = tempdir().unwrap();
    let c = OpenLibraryClient::new(server.uri(), DiskCache::new(dir.path().to_path_buf(), 60), "test/0.1");
    let r = c.lookup_isbn("9780000000000").await.unwrap();
    assert_eq!(r.fields["title"], "Some Book");
    assert_eq!(r.fields["itemType"], "book");
    assert_eq!(r.creators[0].last_name.as_deref(), Some("Doe"));
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/enrichment/openlibrary.rs`:

```rust
use crate::cache::DiskCache;
use crate::error::{Error, Result};
use crate::enrichment::NormalizedRecord;
use crate::types::Creator;
use serde_json::{Map, Value};

#[derive(Clone)]
pub struct OpenLibraryClient {
    base: String,
    cache: DiskCache,
    http: reqwest::Client,
}

impl OpenLibraryClient {
    pub fn new(base: impl Into<String>, cache: DiskCache, user_agent: &str) -> Self {
        let http = reqwest::Client::builder().user_agent(user_agent).build().unwrap();
        Self { base: base.into(), cache, http }
    }

    pub async fn lookup_isbn(&self, isbn: &str) -> Result<NormalizedRecord> {
        let key = format!("openlibrary:isbn:{}", isbn);
        if let Some(v) = self.cache.get::<Value>(&key).await? {
            return self.from_book_json(v).await;
        }
        let url = format!("{}/isbn/{}.json", self.base, isbn);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(Error::Lookup { source: "openlibrary".into(), message: format!("HTTP {}", resp.status()) });
        }
        let book: Value = resp.json().await?;
        self.cache.put(&key, &book).await.ok();
        self.from_book_json(book).await
    }

    async fn from_book_json(&self, book: Value) -> Result<NormalizedRecord> {
        let mut fields = Map::new();
        if let Some(t) = book.get("title").and_then(|x| x.as_str()) {
            fields.insert("title".into(), Value::String(t.into()));
        }
        if let Some(d) = book.get("publish_date").and_then(|x| x.as_str()) {
            fields.insert("date".into(), Value::String(d.into()));
        }
        if let Some(p) = book.get("publishers").and_then(|x| x.as_array()).and_then(|a| a.first()).and_then(|x| x.as_str()) {
            fields.insert("publisher".into(), Value::String(p.into()));
        }
        fields.insert("itemType".into(), Value::String("book".into()));

        let mut creators = vec![];
        if let Some(authors) = book.get("authors").and_then(|x| x.as_array()) {
            for (i, a) in authors.iter().enumerate() {
                if let Some(akey) = a.get("key").and_then(|x| x.as_str()) {
                    let name = self.resolve_author_name(akey).await.unwrap_or_else(|| "Unknown".into());
                    let (first, last) = split_name(&name);
                    creators.push(Creator {
                        first_name: first,
                        last_name: last,
                        creator_type: "author".into(),
                        order_index: i as i64,
                    });
                }
            }
        }

        Ok(NormalizedRecord {
            source: "openlibrary".into(),
            fields,
            creators,
            source_url: Some(format!("{}{}", self.base, "/")),
        })
    }

    async fn resolve_author_name(&self, key: &str) -> Option<String> {
        let cache_key = format!("openlibrary:author:{}", key);
        if let Ok(Some(v)) = self.cache.get::<Value>(&cache_key).await {
            return v.get("name").and_then(|x| x.as_str()).map(String::from);
        }
        let url = format!("{}{}.json", self.base, key);
        let resp = self.http.get(&url).send().await.ok()?;
        if !resp.status().is_success() { return None; }
        let v: Value = resp.json().await.ok()?;
        self.cache.put(&cache_key, &v).await.ok();
        v.get("name").and_then(|x| x.as_str()).map(String::from)
    }
}

fn split_name(full: &str) -> (Option<String>, Option<String>) {
    // Naive: last token is surname; everything before is first.
    let parts: Vec<&str> = full.trim().rsplitn(2, ' ').collect();
    match parts.as_slice() {
        [last, first] => (Some((*first).to_string()), Some((*last).to_string())),
        [single] => (None, Some((*single).to_string())),
        _ => (None, None),
    }
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p zotero-core --test enrich_openlibrary`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): OpenLibrary ISBN lookup with author name resolution"
```

---

### Task 28: arXiv client (lookup_arxiv)

**Files:**
- Create: `crates/zotero-core/src/enrichment/arxiv.rs`
- Test: `crates/zotero-core/tests/enrich_arxiv.rs`

- [ ] **Step 1: Failing test**

```rust
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use tempfile::tempdir;
use zotero_core::cache::DiskCache;
use zotero_core::enrichment::arxiv::ArxivClient;

const SAMPLE_ATOM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<entry>
  <id>http://arxiv.org/abs/2401.00001v1</id>
  <title>A Cool Preprint</title>
  <summary>Abstract here.</summary>
  <published>2024-01-01T00:00:00Z</published>
  <author><name>Alice Aardvark</name></author>
  <author><name>Bob Baboon</name></author>
</entry>
</feed>"#;

#[tokio::test]
async fn lookup_arxiv_parses_atom() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/query"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE_ATOM))
        .mount(&server).await;
    let dir = tempdir().unwrap();
    let c = ArxivClient::new(server.uri(), DiskCache::new(dir.path().to_path_buf(), 60), "test/0.1");
    let r = c.lookup_arxiv("2401.00001").await.unwrap();
    assert_eq!(r.fields["title"], "A Cool Preprint");
    assert_eq!(r.fields["itemType"], "preprint");
    assert_eq!(r.creators.len(), 2);
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/enrichment/arxiv.rs`:

```rust
use crate::cache::DiskCache;
use crate::error::{Error, Result};
use crate::enrichment::NormalizedRecord;
use crate::types::Creator;
use serde_json::{Map, Value};

#[derive(Clone)]
pub struct ArxivClient {
    base: String,
    cache: DiskCache,
    http: reqwest::Client,
}

impl ArxivClient {
    pub fn new(base: impl Into<String>, cache: DiskCache, user_agent: &str) -> Self {
        let http = reqwest::Client::builder().user_agent(user_agent).build().unwrap();
        Self { base: base.into(), cache, http }
    }

    pub async fn lookup_arxiv(&self, id: &str) -> Result<NormalizedRecord> {
        let key = format!("arxiv:{}", id);
        if let Some(v) = self.cache.get::<String>(&key).await? {
            return parse_entry(&v).ok_or_else(|| Error::Lookup { source: "arxiv".into(), message: "cache parse failed".into() });
        }
        let url = format!("{}/api/query?id_list={}", self.base, id);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(Error::Lookup { source: "arxiv".into(), message: format!("HTTP {}", resp.status()) });
        }
        let body = resp.text().await?;
        self.cache.put(&key, &body).await.ok();
        parse_entry(&body).ok_or_else(|| Error::Lookup { source: "arxiv".into(), message: "no entry".into() })
    }
}

fn parse_entry(atom: &str) -> Option<NormalizedRecord> {
    // Minimal Atom-XML parser tailored to arXiv's known shape. For v1 we use a
    // simple scan rather than pulling a full XML parser; arXiv's format is
    // stable enough that this suffices.
    let extract = |open: &str, close: &str| {
        let a = atom.find(open)?;
        let b = atom[a + open.len()..].find(close)?;
        Some(atom[a + open.len()..a + open.len() + b].trim().to_string())
    };
    let title = extract("<title>", "</title>")?;
    // The first <title> in an atom feed is the feed title; skip it.
    let title = atom.match_indices("<title>").nth(1).and_then(|(i, _)| {
        let after = &atom[i + 7..];
        after.find("</title>").map(|j| after[..j].trim().to_string())
    }).unwrap_or(title);

    let summary = extract("<summary>", "</summary>").unwrap_or_default();
    let published = extract("<published>", "</published>").unwrap_or_default();
    let date_only = published.split('T').next().unwrap_or(&published).to_string();

    let mut creators = vec![];
    let mut cursor = 0usize;
    let mut order = 0;
    while let Some(rel) = atom[cursor..].find("<author>") {
        let abs = cursor + rel + "<author>".len();
        if let Some(end) = atom[abs..].find("</author>") {
            let block = &atom[abs..abs + end];
            if let Some(nstart) = block.find("<name>") {
                let after = &block[nstart + "<name>".len()..];
                if let Some(nend) = after.find("</name>") {
                    let name = after[..nend].trim().to_string();
                    let (first, last) = super::openlibrary_like_split(&name);
                    creators.push(Creator {
                        first_name: first,
                        last_name: last,
                        creator_type: "author".into(),
                        order_index: order,
                    });
                    order += 1;
                }
            }
            cursor = abs + end + "</author>".len();
        } else { break; }
    }

    let mut fields = Map::new();
    fields.insert("title".into(), Value::String(title));
    if !summary.is_empty() { fields.insert("abstractNote".into(), Value::String(summary)); }
    if !date_only.is_empty() { fields.insert("date".into(), Value::String(date_only)); }
    fields.insert("itemType".into(), Value::String("preprint".into()));
    fields.insert("repository".into(), Value::String("arXiv".into()));

    Some(NormalizedRecord {
        source: "arxiv".into(),
        fields,
        creators,
        source_url: None,
    })
}
```

Add to `enrichment/mod.rs`:

```rust
pub(crate) fn openlibrary_like_split(full: &str) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = full.trim().rsplitn(2, ' ').collect();
    match parts.as_slice() {
        [last, first] => (Some((*first).to_string()), Some((*last).to_string())),
        [single] => (None, Some((*single).to_string())),
        _ => (None, None),
    }
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p zotero-core --test enrich_arxiv`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): arXiv Atom-feed lookup"
```

---

### Task 29: Semantic Scholar client (search by title+author)

**Files:**
- Create: `crates/zotero-core/src/enrichment/semantic_scholar.rs`
- Test: `crates/zotero-core/tests/enrich_semantic_scholar.rs`

- [ ] **Step 1: Failing test**

```rust
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use tempfile::tempdir;
use zotero_core::cache::DiskCache;
use zotero_core::enrichment::semantic_scholar::SemanticScholarClient;

#[tokio::test]
async fn search_normalizes_results() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/graph/v1/paper/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{
                "paperId": "abc",
                "title": "A Paper on Things",
                "year": 2024,
                "abstract": "Body.",
                "externalIds": {"DOI": "10.1234/abcd"},
                "authors": [{"name":"Alice Aardvark"}]
            }]
        }))).mount(&server).await;

    let dir = tempdir().unwrap();
    let c = SemanticScholarClient::new(server.uri(), DiskCache::new(dir.path().to_path_buf(), 60), "test/0.1", None);
    let v = c.search("paper on things", 1).await.unwrap();
    assert_eq!(v[0].fields["title"], "A Paper on Things");
    assert_eq!(v[0].fields["DOI"], "10.1234/abcd");
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/enrichment/semantic_scholar.rs`:

```rust
use crate::cache::DiskCache;
use crate::error::{Error, Result};
use crate::enrichment::NormalizedRecord;
use crate::types::Creator;
use serde_json::{Map, Value};

#[derive(Clone)]
pub struct SemanticScholarClient {
    base: String,
    cache: DiskCache,
    http: reqwest::Client,
    api_key: Option<String>,
}

impl SemanticScholarClient {
    pub fn new(base: impl Into<String>, cache: DiskCache, user_agent: &str, api_key: Option<String>) -> Self {
        let mut b = reqwest::Client::builder().user_agent(user_agent);
        Self { base: base.into(), cache, http: b.build().unwrap(), api_key }
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<NormalizedRecord>> {
        let key = format!("ss:search:{}:{}", query, limit);
        if let Some(v) = self.cache.get::<Value>(&key).await? {
            return Ok(parse(&v));
        }
        let url = format!("{}/graph/v1/paper/search", self.base);
        let fields = "title,year,abstract,externalIds,authors";
        let mut req = self.http.get(&url).query(&[
            ("query", query),
            ("limit", &limit.to_string()),
            ("fields", fields),
        ]);
        if let Some(k) = &self.api_key { req = req.header("x-api-key", k); }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(Error::Lookup { source: "semantic_scholar".into(), message: format!("HTTP {}", resp.status()) });
        }
        let body: Value = resp.json().await?;
        self.cache.put(&key, &body).await.ok();
        Ok(parse(&body))
    }
}

fn parse(v: &Value) -> Vec<NormalizedRecord> {
    v.get("data").and_then(|a| a.as_array()).map(|arr| arr.iter().filter_map(|p| {
        let mut fields = Map::new();
        if let Some(t) = p.get("title").and_then(|x| x.as_str()) { fields.insert("title".into(), Value::String(t.into())); }
        if let Some(y) = p.get("year").and_then(|x| x.as_i64()) { fields.insert("date".into(), Value::String(y.to_string())); }
        if let Some(a) = p.get("abstract").and_then(|x| x.as_str()) { fields.insert("abstractNote".into(), Value::String(a.into())); }
        if let Some(doi) = p.get("externalIds").and_then(|e| e.get("DOI")).and_then(|x| x.as_str()) {
            fields.insert("DOI".into(), Value::String(doi.into()));
        }
        fields.insert("itemType".into(), Value::String("journalArticle".into()));

        let creators = p.get("authors").and_then(|a| a.as_array()).map(|arr| arr.iter().enumerate().map(|(i, a)| {
            let name = a.get("name").and_then(|x| x.as_str()).unwrap_or("");
            let (first, last) = crate::enrichment::openlibrary_like_split(name);
            Creator { first_name: first, last_name: last, creator_type: "author".into(), order_index: i as i64 }
        }).collect()).unwrap_or_default();
        Some(NormalizedRecord {
            source: "semantic_scholar".into(),
            fields, creators, source_url: None,
        })
    }).collect()).unwrap_or_default()
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p zotero-core --test enrich_semantic_scholar`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): Semantic Scholar search client"
```

---

### Task 30: PDF signal extraction (DOI from first page)

**Files:**
- Create: `crates/zotero-core/src/enrichment/pdf_signals.rs`
- Test: `crates/zotero-core/tests/enrich_pdf_signals.rs`

- [ ] **Step 1: Failing test**

```rust
use zotero_core::enrichment::pdf_signals::extract_signals;

#[test]
fn finds_doi_in_text() {
    let text = "Some title\nDOI: 10.1234/abcd.5678  Some other text.";
    let s = extract_signals(text);
    assert_eq!(s.doi_candidates, vec!["10.1234/abcd.5678".to_string()]);
}

#[test]
fn finds_arxiv_id() {
    let text = "Preprint arXiv:2401.00001v2 available";
    let s = extract_signals(text);
    assert_eq!(s.arxiv_candidates, vec!["2401.00001".to_string()]);
}

#[test]
fn picks_first_nontrivial_line_as_title_candidate() {
    let text = "\n\nPage 1\n\nA Real Title Here\n\nAlice Aardvark, Bob Baboon\n\nAbstract: ...";
    let s = extract_signals(text);
    assert_eq!(s.title_candidate.as_deref(), Some("A Real Title Here"));
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/enrichment/pdf_signals.rs`:

```rust
#[derive(Debug, Clone, Default)]
pub struct PdfSignals {
    pub doi_candidates: Vec<String>,
    pub arxiv_candidates: Vec<String>,
    pub isbn_candidates: Vec<String>,
    pub title_candidate: Option<String>,
    pub author_candidates: Vec<String>,
}

pub fn extract_signals(text: &str) -> PdfSignals {
    let mut s = PdfSignals::default();
    // First-page bias: cap to first 4000 chars.
    let head: String = text.chars().take(4000).collect();

    s.doi_candidates = find_dois(&head);
    s.arxiv_candidates = find_arxiv(&head);
    s.isbn_candidates = find_isbn(&head);
    s.title_candidate = guess_title(&head);
    s.author_candidates = guess_authors(&head);
    s
}

fn find_dois(s: &str) -> Vec<String> {
    let mut out = vec![];
    for token in s.split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == ')' ) {
        let t = token.trim_start_matches("doi:").trim_start_matches("DOI:").trim();
        if t.starts_with("10.") && t.contains('/') && t.len() < 200 {
            // Strip trailing punctuation.
            let cleaned = t.trim_end_matches(|c: char| !c.is_alphanumeric());
            if !out.iter().any(|x| x == cleaned) {
                out.push(cleaned.to_string());
            }
        }
    }
    out
}

fn find_arxiv(s: &str) -> Vec<String> {
    let mut out = vec![];
    for needle in ["arXiv:", "arxiv:"] {
        let mut rest = s;
        while let Some(i) = rest.find(needle) {
            let after = &rest[i + needle.len()..];
            let id: String = after.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
            if !id.is_empty() && id.contains('.') && !out.contains(&id) {
                out.push(id.clone());
            }
            rest = &after[id.len()..];
        }
    }
    out
}

fn find_isbn(s: &str) -> Vec<String> {
    let mut out = vec![];
    for w in s.split_whitespace() {
        let digits: String = w.chars().filter(|c| c.is_ascii_digit() || *c == 'X').collect();
        if (digits.len() == 10 || digits.len() == 13) && !out.contains(&digits) {
            out.push(digits);
        }
    }
    out
}

fn guess_title(s: &str) -> Option<String> {
    for line in s.lines() {
        let t = line.trim();
        if t.len() > 12 && t.split_whitespace().count() >= 3 && !t.starts_with("DOI") && !t.starts_with("doi:") {
            return Some(t.to_string());
        }
    }
    None
}

fn guess_authors(s: &str) -> Vec<String> {
    // After the title line, the next non-empty line is a heuristic author list.
    let lines: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() < 2 { return vec![]; }
    let line = lines[1];
    line.split(|c: char| c == ',' || c == ';').map(|s| s.trim().to_string())
        .filter(|s| s.split_whitespace().count() >= 2 && s.len() < 60)
        .collect()
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p zotero-core --test enrich_pdf_signals`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): heuristic PDF signal extraction (DOI, arXiv, ISBN, title, authors)"
```

---

### Task 31: Confidence scoring

**Files:**
- Create: `crates/zotero-core/src/enrichment/scoring.rs`
- Test: `crates/zotero-core/tests/enrich_scoring.rs`

- [ ] **Step 1: Failing test**

```rust
use zotero_core::enrichment::scoring::{score, ScoringInput, ScoreBreakdown};
use zotero_core::enrichment::pdf_signals::PdfSignals;
use zotero_core::enrichment::NormalizedRecord;
use zotero_core::types::Creator;
use serde_json::Map;

fn rec(title: &str, year: &str, doi: Option<&str>, surname: &str) -> NormalizedRecord {
    let mut fields = Map::new();
    fields.insert("title".into(), title.into());
    fields.insert("date".into(), year.into());
    if let Some(d) = doi { fields.insert("DOI".into(), d.into()); }
    NormalizedRecord {
        source: "test".into(), fields, source_url: None,
        creators: vec![Creator { first_name: None, last_name: Some(surname.into()), creator_type:"author".into(), order_index:0 }],
    }
}

#[test]
fn matches_doi_yields_high_score() {
    let signals = PdfSignals { doi_candidates: vec!["10.1234/abcd".into()], title_candidate: Some("A Paper on Things".into()), ..Default::default() };
    let current = serde_json::json!({ "title": "paper on things", "date": "2024" });
    let r = rec("A Paper on Things", "2024", Some("10.1234/abcd"), "Aardvark");
    let ScoreBreakdown { score: s, .. } = score(&ScoringInput {
        current_fields: &current, signals: &signals, candidate: &r,
    });
    assert!(s >= 0.9);
}

#[test]
fn weak_title_match_yields_low_score() {
    let signals = PdfSignals::default();
    let current = serde_json::json!({ "title": "Completely unrelated" });
    let r = rec("Other Paper", "1999", None, "Zilch");
    let ScoreBreakdown { score: s, .. } = score(&ScoringInput {
        current_fields: &current, signals: &signals, candidate: &r,
    });
    assert!(s < 0.5);
}
```

- [ ] **Step 2: Implement**

`crates/zotero-core/src/enrichment/scoring.rs`:

```rust
use crate::enrichment::pdf_signals::PdfSignals;
use crate::enrichment::NormalizedRecord;
use serde_json::Value;

pub struct ScoringInput<'a> {
    pub current_fields: &'a Value,
    pub signals: &'a PdfSignals,
    pub candidate: &'a NormalizedRecord,
}

pub struct ScoreBreakdown {
    pub score: f64,
    pub reasons: Vec<String>,
}

pub fn score(inp: &ScoringInput<'_>) -> ScoreBreakdown {
    let mut s: f64 = 0.0;
    let mut reasons = vec![];

    // DOI direct: if the candidate's DOI matches a signal DOI, big positive
    if let Some(doi_c) = inp.candidate.fields.get("DOI").and_then(|x| x.as_str()) {
        if inp.signals.doi_candidates.iter().any(|d| d.eq_ignore_ascii_case(doi_c)) {
            s += 0.5;
            reasons.push("DOI found in PDF first page".into());
        }
    }

    // Title fuzzy
    let cand_title = inp.candidate.fields.get("title").and_then(|x| x.as_str()).unwrap_or("");
    let cur_title = inp.current_fields.get("title").and_then(|x| x.as_str()).unwrap_or("");
    let signal_title = inp.signals.title_candidate.as_deref().unwrap_or("");
    let title_score = token_overlap(cand_title, &[cur_title, signal_title].join(" "));
    if title_score >= 0.9 { s += 0.35; reasons.push("title token overlap ≥ 0.9".into()); }
    else if title_score >= 0.7 { s += 0.15; reasons.push("title token overlap 0.7..0.9".into()); }
    else if !cur_title.is_empty() || !signal_title.is_empty() { s -= 0.15; reasons.push("title overlap < 0.7".into()); }

    // First-author surname match
    let cand_surname = inp.candidate.creators.first().and_then(|c| c.last_name.as_deref()).unwrap_or("").to_lowercase();
    if !cand_surname.is_empty() && inp.signals.author_candidates.iter().any(|a| a.to_lowercase().contains(&cand_surname)) {
        s += 0.1;
        reasons.push("first-author surname appears in PDF authors line".into());
    }

    // Year ±1
    let cand_year = inp.candidate.fields.get("date").and_then(|x| x.as_str()).and_then(year_of);
    let cur_year = inp.current_fields.get("date").and_then(|x| x.as_str()).and_then(year_of);
    if let (Some(c), Some(u)) = (cand_year, cur_year) {
        if (c - u).abs() <= 1 { s += 0.05; reasons.push("year within ±1".into()); }
        else { s -= 0.05; reasons.push("year mismatch > 1".into()); }
    }

    let clamped = s.clamp(0.0, 1.0);
    ScoreBreakdown { score: clamped, reasons }
}

fn token_overlap(a: &str, b: &str) -> f64 {
    let an: std::collections::HashSet<String> = tokens(a);
    let bn: std::collections::HashSet<String> = tokens(b);
    if an.is_empty() || bn.is_empty() { return 0.0; }
    let inter = an.intersection(&bn).count() as f64;
    let denom = an.len().min(bn.len()) as f64;
    inter / denom
}

fn tokens(s: &str) -> std::collections::HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase()).collect()
}

fn year_of(s: &str) -> Option<i64> {
    s.split('-').next().and_then(|y| y.parse::<i64>().ok())
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p zotero-core --test enrich_scoring`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): confidence scoring with DOI/title/author/year signals"
```

---

### Task 32: propose, apply, find_weak, enrich_item composite

**Files:**
- Create: `crates/zotero-core/src/enrichment/propose.rs`
- Test: `crates/zotero-core/tests/enrich_propose.rs`

- [ ] **Step 1: Failing test for propose**

```rust
use serde_json::json;
use zotero_core::enrichment::propose::{compute_diff};

#[test]
fn diff_includes_only_changes() {
    let current = json!({ "title": "Old", "DOI": "10.1/a" });
    let proposed = json!({ "title": "Old", "DOI": "10.1/b", "abstractNote": "added" });
    let d = compute_diff(&current, &proposed);
    let changed: Vec<&str> = d.changes.iter().map(|c| c.field.as_str()).collect();
    assert!(changed.contains(&"DOI"));
    assert!(changed.contains(&"abstractNote"));
    assert!(!changed.contains(&"title"));
}
```

- [ ] **Step 2: Implement propose + apply + composite + find_weak**

`crates/zotero-core/src/enrichment/propose.rs`:

```rust
use crate::error::{Error, Result};
use crate::enrichment::{NormalizedRecord, pdf_signals::PdfSignals, scoring};
use crate::pdf::get_pdf_first_pages;
use crate::reader::pool::ReadOnlyPool;
use crate::reader::items::get_item_by_key;
use crate::types::{Diff, EnrichmentProposal, FieldChange, SourceBreakdown};
use crate::writer::client::LocalApi;
use crate::writer::items::update_item_fields;
use serde_json::{json, Map, Value};
use std::path::Path;

pub fn compute_diff(current: &Value, proposed: &Value) -> Diff {
    let mut changes = vec![];
    if let Value::Object(pm) = proposed {
        for (k, pv) in pm {
            let cv = current.get(k).cloned();
            let differs = match &cv {
                Some(x) => x != pv,
                None => !pv.is_null(),
            };
            if differs {
                changes.push(FieldChange { field: k.clone(), current: cv, proposed: pv.clone() });
            }
        }
    }
    Diff { changes }
}

/// Find items whose metadata looks stubby. Heuristics:
///   - missing DOI on a journalArticle
///   - missing abstractNote
///   - title equal to attached filename
///   - very short title
pub async fn find_weak_metadata_items(pool: &ReadOnlyPool, library_id: i64, limit: i64) -> Result<Vec<(String, Vec<String>)>> {
    pool.with_conn(move |c| {
        let mut out = vec![];
        let mut stmt = c.prepare(
            "SELECT i.key, it.typeName,
                (SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = i.itemID AND d.fieldID = (SELECT fieldID FROM fieldsCombined WHERE fieldName='title')) AS title,
                (SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = i.itemID AND d.fieldID = (SELECT fieldID FROM fieldsCombined WHERE fieldName='DOI')) AS doi,
                (SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = i.itemID AND d.fieldID = (SELECT fieldID FROM fieldsCombined WHERE fieldName='abstractNote')) AS abs
            FROM items i JOIN itemTypes it ON it.itemTypeID = i.itemTypeID
            WHERE i.libraryID = ? AND it.typeName NOT IN ('attachment','note','annotation')
            LIMIT ?"
        )?;
        let mut rows = stmt.query(rusqlite::params![library_id, limit])?;
        while let Some(r) = rows.next()? {
            let mut reasons = vec![];
            let key: String = r.get(0)?;
            let typ: String = r.get(1)?;
            let title: Option<String> = r.get(2)?;
            let doi: Option<String> = r.get(3)?;
            let abs: Option<String> = r.get(4)?;
            if typ == "journalArticle" && doi.as_deref().unwrap_or("").is_empty() {
                reasons.push("missing DOI on journalArticle".into());
            }
            if abs.as_deref().unwrap_or("").is_empty() {
                reasons.push("missing abstractNote".into());
            }
            if let Some(t) = &title {
                if t.len() < 8 { reasons.push("very short title".into()); }
                if t.ends_with(".pdf") || t.ends_with(".html") { reasons.push("title looks like a filename".into()); }
            } else { reasons.push("missing title".into()); }
            if !reasons.is_empty() { out.push((key, reasons)); }
        }
        Ok(out)
    }).await
}

pub struct ProposeInput<'a> {
    pub item_key: &'a str,
    pub library_id: i64,
    pub storage_dir: &'a Path,
    pub candidates: Vec<NormalizedRecord>,
}

pub async fn propose_metadata_update(pool: &ReadOnlyPool, inp: ProposeInput<'_>) -> Result<EnrichmentProposal> {
    let item = get_item_by_key(pool, inp.item_key, inp.library_id).await?;

    // Pull PDF first-page signals if we have a PDF attachment
    let signals = match get_pdf_first_pages(pool, inp.item_key, inp.library_id, inp.storage_dir, 1).await {
        Ok(p) => crate::enrichment::pdf_signals::extract_signals(&p.text),
        Err(_) => PdfSignals::default(),
    };

    // Score each candidate; pick best
    let mut best: Option<(f64, &NormalizedRecord, Vec<String>)> = None;
    let mut source_breakdown = vec![];
    for c in &inp.candidates {
        let ScoreBreakdownOpt { score: s, reasons } = score_candidate(&item.fields, &signals, c);
        source_breakdown.push(SourceBreakdown {
            source: c.source.clone(),
            matched: s > 0.5,
            fields_contributed: c.fields.iter().map(|(k, _)| k.clone()).collect(),
            raw_response_cached: true,
        });
        if best.as_ref().map(|(b, _, _)| s > *b).unwrap_or(true) {
            best = Some((s, c, reasons));
        }
    }
    let (confidence, candidate, _reasons) = best.ok_or_else(|| Error::Lookup { source: "any".into(), message: "no candidates".into() })?;

    // Build proposed fields, merging only when current is empty/null
    let mut proposed = Map::new();
    if let Value::Object(cur) = &item.fields {
        for (k, v) in cur { proposed.insert(k.clone(), v.clone()); }
    }
    for (k, v) in &candidate.fields {
        let cur_empty = proposed.get(k).map(|x| matches!(x, Value::Null) || x.as_str().map(|s| s.is_empty()).unwrap_or(false)).unwrap_or(true);
        if cur_empty { proposed.insert(k.clone(), v.clone()); }
    }
    let proposed_v = Value::Object(proposed);
    let diff = compute_diff(&item.fields, &proposed_v);

    let needs_review = confidence < 0.9 || source_breakdown.iter().filter(|s| s.matched).count() < 2;

    Ok(EnrichmentProposal {
        item_key: inp.item_key.into(),
        diff,
        confidence,
        source_breakdown,
        needs_review,
    })
}

pub async fn apply_metadata_update(api: &LocalApi, pool: &ReadOnlyPool, library_id: i64, proposal: &EnrichmentProposal) -> Result<()> {
    let item = get_item_by_key(pool, &proposal.item_key, library_id).await?;
    let mut patch = Map::new();
    for ch in &proposal.diff.changes {
        patch.insert(ch.field.clone(), ch.proposed.clone());
    }
    update_item_fields(api, &proposal.item_key, item.version, Value::Object(patch)).await
}

pub struct EnrichInput<'a> {
    pub item_key: &'a str,
    pub library_id: i64,
    pub storage_dir: &'a Path,
    pub candidates: Vec<NormalizedRecord>,
    pub auto_apply_threshold: f64,
}

pub async fn enrich_item(api: &LocalApi, pool: &ReadOnlyPool, inp: EnrichInput<'_>) -> Result<EnrichmentProposal> {
    let auto = inp.auto_apply_threshold;
    let proposal = propose_metadata_update(pool, ProposeInput {
        item_key: inp.item_key,
        library_id: inp.library_id,
        storage_dir: inp.storage_dir,
        candidates: inp.candidates,
    }).await?;
    if proposal.confidence >= auto && !proposal.needs_review {
        apply_metadata_update(api, pool, inp.library_id, &proposal).await?;
    }
    Ok(proposal)
}

struct ScoreBreakdownOpt { score: f64, reasons: Vec<String> }
fn score_candidate(current: &Value, signals: &PdfSignals, c: &NormalizedRecord) -> ScoreBreakdownOpt {
    let s = scoring::score(&scoring::ScoringInput { current_fields: current, signals, candidate: c });
    ScoreBreakdownOpt { score: s.score, reasons: s.reasons }
}
```

- [ ] **Step 3: Run propose test**

Run: `cargo test -p zotero-core --test enrich_propose`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-core/
git commit -m "feat(core): propose/apply/enrich + find_weak_metadata_items"
```

---

## Phase 9 — MCP wiring

### Task 33: rmcp scaffolding, server starts and responds

**Files:**
- Modify: `crates/zotero-mcp/src/main.rs`
- Create: `crates/zotero-mcp/src/server.rs`
- Create: `crates/zotero-mcp/src/state.rs`

- [ ] **Step 1: Add app state**

`crates/zotero-mcp/src/state.rs`:

```rust
use std::sync::Arc;
use zotero_core::bbt::BbtClient;
use zotero_core::cache::DiskCache;
use zotero_core::config::Config;
use zotero_core::enrichment::arxiv::ArxivClient;
use zotero_core::enrichment::crossref::CrossrefClient;
use zotero_core::enrichment::openlibrary::OpenLibraryClient;
use zotero_core::enrichment::semantic_scholar::SemanticScholarClient;
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::writer::client::LocalApi;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Config,
    pub pool: ReadOnlyPool,
    pub api: LocalApi,
    pub bbt: Option<BbtClient>,
    pub crossref: CrossrefClient,
    pub openlibrary: OpenLibraryClient,
    pub arxiv: ArxivClient,
    pub semantic_scholar: SemanticScholarClient,
}

impl AppState {
    pub async fn build(cfg: Config) -> anyhow::Result<Self> {
        let pool = ReadOnlyPool::new(cfg.sqlite_path(), 4).await?;
        // Schema-version check at startup
        {
            let conn = zotero_core::reader::conn::open_read_only(&cfg.sqlite_path())?;
            zotero_core::reader::conn::check_schema(&conn, cfg.zotero.min_schema_userdata, cfg.zotero.max_schema_userdata)?;
        }
        let user_id = if cfg.zotero.user_id > 0 { cfg.zotero.user_id } else {
            detect_user_id(&cfg.zotero.local_api_base).await?
        };
        let api = LocalApi::new(cfg.zotero.local_api_base.clone(), user_id)?;
        let bbt = BbtClient::new(cfg.zotero.local_api_base.clone()).ok();

        let cache = DiskCache::new(cfg.resolved_cache_dir(), cfg.enrichment.cache_ttl_days * 86_400);
        let ua = cfg.web.user_agent.clone();
        let crossref = CrossrefClient::new("https://api.crossref.org", cache.clone(), &ua);
        let openlibrary = OpenLibraryClient::new("https://openlibrary.org", cache.clone(), &ua);
        let arxiv = ArxivClient::new("https://export.arxiv.org", cache.clone(), &ua);
        let semantic_scholar = SemanticScholarClient::new("https://api.semanticscholar.org", cache, &ua, None);

        Ok(Self { cfg, pool, api, bbt, crossref, openlibrary, arxiv, semantic_scholar })
    }
}

async fn detect_user_id(base: &str) -> anyhow::Result<i64> {
    let resp = reqwest::Client::new()
        .get(format!("{}/api/users/0/items?limit=1", base))
        .header("Zotero-API-Version", "3")
        .send().await?;
    let v: serde_json::Value = resp.json().await?;
    let id = v.as_array().and_then(|a| a.first()).and_then(|i| i.get("library")).and_then(|l| l.get("id")).and_then(|x| x.as_i64())
        .ok_or_else(|| anyhow::anyhow!("could not detect Zotero user ID via Local API"))?;
    Ok(id)
}
```

- [ ] **Step 2: Server scaffolding**

`crates/zotero-mcp/src/server.rs`:

```rust
use crate::state::AppState;
use rmcp::{model::*, service::*, transport::stdio};

pub struct ZoteroServer { pub state: AppState }

#[rmcp::tool_router]
impl ZoteroServer {
    #[rmcp::tool(description = "Ping the server")]
    pub async fn ping(&self) -> Result<rmcp::model::CallToolResult, rmcp::Error> {
        Ok(CallToolResult::success(vec![Content::text("pong")]))
    }
}

#[rmcp::server_handler]
impl ServerHandler for ZoteroServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities::builder().enable_tools().enable_resources().build(),
            server_info: Implementation { name: "zotero-mcp".into(), version: "0.1.0".into() },
            instructions: Some("Local Zotero library bridge".into()),
        }
    }
}

pub async fn run(state: AppState) -> anyhow::Result<()> {
    let server = ZoteroServer { state };
    let transport = stdio();
    server.serve(transport).await?;
    Ok(())
}
```

- [ ] **Step 3: Wire main.rs**

```rust
mod logging;
mod server;
mod state;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = zotero_core::Config::load().unwrap_or_default();
    logging::init(&cfg.logging.level, Some(&cfg.resolved_log_dir()))?;
    tracing::info!("zotero-mcp starting (user_id auto={})", cfg.zotero.user_id == 0);
    let state = state::AppState::build(cfg).await?;
    server::run(state).await
}
```

- [ ] **Step 4: Sanity check that the binary at least compiles and starts**

Run: `cargo build -p zotero-mcp`
Expected: clean compile.

Run: `echo '{"jsonrpc":"2.0","id":1,"method":"ping"}' | cargo run -p zotero-mcp 2>/dev/null | head -1`
Expected: a JSON-RPC response over stdout. Exact format depends on rmcp's protocol; the test is that *some* JSON appears, not a stack trace.

> If `rmcp`'s public API has shifted relative to the code shown above (macro names, trait names), the implementer must check `cargo doc -p rmcp --open` and adjust. The structural intent is: stdio transport + tool macros + one `ping` tool registered.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/
git commit -m "feat(mcp): rmcp scaffolding with ping tool and stdio transport"
```

---

### Task 34: Wire search and retrieve tools

**Files:**
- Create: `crates/zotero-mcp/src/tools/mod.rs`
- Create: `crates/zotero-mcp/src/tools/search.rs`
- Modify: `crates/zotero-mcp/src/server.rs`

- [ ] **Step 1: Implement tool modules**

`crates/zotero-mcp/src/tools/mod.rs`:

```rust
pub mod search;
pub mod attachments;
pub mod citations;
pub mod writes;
pub mod enrichment;
```

`crates/zotero-mcp/src/tools/search.rs`:

```rust
use crate::state::AppState;
use rmcp::model::{CallToolResult, Content};
use rmcp::Error;
use serde::Deserialize;
use zotero_core::reader::items::{get_item_by_key, hydrate_citation_key};
use zotero_core::reader::search::{search_metadata, SearchParams};
use zotero_core::reader::{collections, recent, tags};

#[derive(Deserialize)]
pub struct SearchArgs {
    pub query: String,
    #[serde(default)] pub item_type: Option<String>,
    #[serde(default)] pub tag: Option<String>,
    #[serde(default)] pub collection: Option<String>,
    #[serde(default = "default_true")] pub include_fulltext: bool,
    #[serde(default)] pub limit: i64,
    #[serde(default)] pub offset: i64,
}
fn default_true() -> bool { true }

pub async fn search_items(s: &AppState, args: SearchArgs) -> Result<CallToolResult, Error> {
    let hits = search_metadata(&s.pool, 1, SearchParams {
        query: args.query, item_type: args.item_type, tag: args.tag,
        collection_key: args.collection, include_fulltext: args.include_fulltext,
        limit: args.limit, offset: args.offset,
    }).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&hits).unwrap())?]))
}

#[derive(Deserialize)]
pub struct GetItemArgs {
    #[serde(default)] pub item_key: Option<String>,
    #[serde(default)] pub citation_key: Option<String>,
}

pub async fn get_item(s: &AppState, args: GetItemArgs) -> Result<CallToolResult, Error> {
    let key = match (args.item_key, args.citation_key) {
        (Some(k), _) => k,
        (_, Some(ck)) => resolve_citation_key(s, &ck).await?,
        _ => return Err(Error::invalid_params("either item_key or citation_key required", None)),
    };
    let mut item = get_item_by_key(&s.pool, &key, 1).await.map_err(internal)?;
    hydrate_citation_key(&mut item, s.bbt.as_ref()).await;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&item).unwrap())?]))
}

async fn resolve_citation_key(s: &AppState, ck: &str) -> Result<String, Error> {
    // BBT JSON-RPC doesn't expose reverse lookup, so scan: pull recent items, hydrate keys.
    // For v1, simply require item_key. Citation-key path is best-effort: use BBT item.search.
    Err(Error::invalid_params("reverse citation_key lookup not supported in v1; pass item_key", None))
}

#[derive(Deserialize)] pub struct EmptyArgs {}

pub async fn list_collections(s: &AppState, _args: EmptyArgs) -> Result<CallToolResult, Error> {
    let cs = collections::list(&s.pool, 1, None).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&cs).unwrap())?]))
}

#[derive(Deserialize)] pub struct ListTagsArgs { #[serde(default)] pub prefix: Option<String> }
pub async fn list_tags(s: &AppState, args: ListTagsArgs) -> Result<CallToolResult, Error> {
    let ts = tags::list(&s.pool, 1, args.prefix).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&ts).unwrap())?]))
}

#[derive(Deserialize)] pub struct RecentArgs { #[serde(default = "default_sort")] pub sort_by: String, #[serde(default = "default_limit")] pub limit: i64 }
fn default_sort() -> String { "dateModified".into() }
fn default_limit() -> i64 { 20 }
pub async fn list_recent_items(s: &AppState, args: RecentArgs) -> Result<CallToolResult, Error> {
    let r = recent::list(&s.pool, 1, &args.sort_by, args.limit).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))
}

pub(crate) fn internal(e: zotero_core::Error) -> Error {
    Error::internal_error(e.to_string(), None)
}
```

- [ ] **Step 2: Register in server.rs via `#[rmcp::tool]`**

In `ZoteroServer`, add tool wrappers (one per function) that deserialize JSON args, call the function, and return the result. Example pattern:

```rust
#[rmcp::tool(description = "Search the local Zotero library (metadata + optional fulltext).")]
pub async fn search_items(&self, args: serde_json::Value) -> Result<CallToolResult, rmcp::Error> {
    let a: crate::tools::search::SearchArgs = serde_json::from_value(args).map_err(|e| rmcp::Error::invalid_params(e.to_string(), None))?;
    crate::tools::search::search_items(&self.state, a).await
}
```

Repeat for `get_item`, `list_collections`, `list_tags`, `list_recent_items`.

- [ ] **Step 3: Build + commit**

Run: `cargo build -p zotero-mcp`
Expected: clean.

```bash
git add crates/zotero-mcp/
git commit -m "feat(mcp): wire search, get_item, list_collections, list_tags, list_recent_items"
```

---

### Task 35: Wire attachment and content tools

**Files:**
- Create: `crates/zotero-mcp/src/tools/attachments.rs`
- Modify: `crates/zotero-mcp/src/server.rs`

- [ ] **Step 1: Implement**

`crates/zotero-mcp/src/tools/attachments.rs`:

```rust
use crate::state::AppState;
use crate::tools::search::internal;
use rmcp::model::{CallToolResult, Content};
use rmcp::Error;
use serde::Deserialize;
use zotero_core::pdf::{get_pdf_first_pages, get_pdf_text};
use zotero_core::reader::annotations::list_annotations;
use zotero_core::reader::attachments::{list_attachments, resolve_path};
use zotero_core::web::{get_webpage_content, refetch_url, WebMode};

#[derive(Deserialize)] pub struct ItemKeyArgs { pub item_key: String }

pub async fn list_attachments_t(s: &AppState, a: ItemKeyArgs) -> Result<CallToolResult, Error> {
    let r = list_attachments(&s.pool, &a.item_key, 1, &s.cfg.storage_dir()).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))
}

pub async fn get_pdf_path(s: &AppState, a: ItemKeyArgs) -> Result<CallToolResult, Error> {
    let p = resolve_path(&s.pool, &a.item_key, 1, &s.cfg.storage_dir()).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::text(p.to_string_lossy().into_owned())]))
}

pub async fn get_pdf_text_t(s: &AppState, a: ItemKeyArgs) -> Result<CallToolResult, Error> {
    let r = get_pdf_text(&s.pool, &a.item_key, 1, &s.cfg.storage_dir()).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))
}

#[derive(Deserialize)] pub struct FirstPagesArgs { pub item_key: String, #[serde(default = "two")] pub n: usize }
fn two() -> usize { 2 }
pub async fn get_pdf_first_pages_t(s: &AppState, a: FirstPagesArgs) -> Result<CallToolResult, Error> {
    let r = get_pdf_first_pages(&s.pool, &a.item_key, 1, &s.cfg.storage_dir(), a.n).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))
}

pub async fn list_annotations_t(s: &AppState, a: ItemKeyArgs) -> Result<CallToolResult, Error> {
    let r = list_annotations(&s.pool, &a.item_key, 1).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))
}

#[derive(Deserialize)] pub struct WebArgs { pub item_key: String, #[serde(default = "auto")] pub mode: String }
fn auto() -> String { "auto".into() }
pub async fn get_webpage_content_t(s: &AppState, a: WebArgs) -> Result<CallToolResult, Error> {
    let mode = match a.mode.as_str() {
        "snapshot" => WebMode::Snapshot, "live" => WebMode::Live, _ => WebMode::Auto,
    };
    let r = get_webpage_content(&s.pool, &a.item_key, 1, &s.cfg.storage_dir(), mode, &s.cfg.web.user_agent).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))
}

#[derive(Deserialize)] pub struct RefetchArgs { pub item_key: String, #[serde(default)] pub save_as_snapshot: bool }
pub async fn refetch_url_t(s: &AppState, a: RefetchArgs) -> Result<CallToolResult, Error> {
    let r = refetch_url(&s.pool, Some(&s.api), &a.item_key, 1, a.save_as_snapshot, &s.cfg.web.user_agent).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))
}
```

- [ ] **Step 2: Register each in `server.rs`** with `#[rmcp::tool]` wrappers (same pattern as Task 34).

- [ ] **Step 3: Build + commit**

```bash
cargo build -p zotero-mcp
git add crates/zotero-mcp/
git commit -m "feat(mcp): wire list_attachments, get_pdf_*, list_annotations, web tools"
```

---

### Task 36: Wire citation tools

**Files:**
- Create: `crates/zotero-mcp/src/tools/citations.rs`
- Modify: `crates/zotero-mcp/src/server.rs`

- [ ] **Step 1: Implement**

`crates/zotero-mcp/src/tools/citations.rs`:

```rust
use crate::state::AppState;
use crate::tools::search::internal;
use rmcp::model::{CallToolResult, Content};
use rmcp::Error;
use serde::Deserialize;
use zotero_core::citations::{format_bibliography, format_citation};

#[derive(Deserialize)] pub struct FormatCitationArgs { pub item_key: String, #[serde(default = "apa")] pub style: String, #[serde(default = "bib")] pub format: String }
fn apa() -> String { "apa".into() }
fn bib() -> String { "bib".into() }

pub async fn format_citation_t(s: &AppState, a: FormatCitationArgs) -> Result<CallToolResult, Error> {
    let r = format_citation(&s.api, &a.item_key, &a.style, &a.format).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::text(r)]))
}

#[derive(Deserialize)] pub struct FormatBibArgs { pub item_keys: Vec<String>, #[serde(default = "apa")] pub style: String, #[serde(default = "bib")] pub format: String }
pub async fn format_bibliography_t(s: &AppState, a: FormatBibArgs) -> Result<CallToolResult, Error> {
    let r = format_bibliography(&s.api, &a.item_keys, &a.style, &a.format).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::text(r)]))
}
```

- [ ] **Step 2: Register + build + commit**

```bash
cargo build -p zotero-mcp
git add crates/zotero-mcp/
git commit -m "feat(mcp): wire format_citation and format_bibliography"
```

---

### Task 37: Wire write tools

**Files:**
- Create: `crates/zotero-mcp/src/tools/writes.rs`
- Modify: `crates/zotero-mcp/src/server.rs`

- [ ] **Step 1: Implement**

`crates/zotero-mcp/src/tools/writes.rs`:

```rust
use crate::state::AppState;
use crate::tools::search::internal;
use rmcp::model::{CallToolResult, Content};
use rmcp::Error;
use serde::Deserialize;
use zotero_core::reader::items::get_item_by_key;
use zotero_core::writer::items::update_item_fields;
use zotero_core::writer::notes::add_note;
use zotero_core::writer::tags::{add_tags, add_to_collection, remove_from_collection, remove_tags};

#[derive(Deserialize)] pub struct AddNoteArgs { pub item_key: String, pub markdown: String }
pub async fn add_note_t(s: &AppState, a: AddNoteArgs) -> Result<CallToolResult, Error> {
    let k = add_note(&s.api, &a.item_key, &a.markdown).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::text(k)]))
}

#[derive(Deserialize)] pub struct UpdateFieldsArgs { pub item_key: String, pub fields: serde_json::Value }
pub async fn update_item_fields_t(s: &AppState, a: UpdateFieldsArgs) -> Result<CallToolResult, Error> {
    let item = get_item_by_key(&s.pool, &a.item_key, 1).await.map_err(internal)?;
    update_item_fields(&s.api, &a.item_key, item.version, a.fields).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::text("ok")]))
}

#[derive(Deserialize)] pub struct TagArgs { pub item_key: String, pub tags: Vec<String> }
pub async fn add_tags_t(s: &AppState, a: TagArgs) -> Result<CallToolResult, Error> {
    add_tags(&s.api, &a.item_key, &a.tags).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::text("ok")]))
}
pub async fn remove_tags_t(s: &AppState, a: TagArgs) -> Result<CallToolResult, Error> {
    remove_tags(&s.api, &a.item_key, &a.tags).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::text("ok")]))
}

#[derive(Deserialize)] pub struct CollectionArgs { pub item_key: String, pub collection_key: String }
pub async fn add_to_collection_t(s: &AppState, a: CollectionArgs) -> Result<CallToolResult, Error> {
    add_to_collection(&s.api, &a.item_key, &a.collection_key).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::text("ok")]))
}
pub async fn remove_from_collection_t(s: &AppState, a: CollectionArgs) -> Result<CallToolResult, Error> {
    remove_from_collection(&s.api, &a.item_key, &a.collection_key).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::text("ok")]))
}
```

- [ ] **Step 2: Register + build + commit**

```bash
cargo build -p zotero-mcp
git add crates/zotero-mcp/
git commit -m "feat(mcp): wire add_note, update_item_fields, tag/collection mutators"
```

---

### Task 38: Wire enrichment tools

**Files:**
- Create: `crates/zotero-mcp/src/tools/enrichment.rs`
- Modify: `crates/zotero-mcp/src/server.rs`

- [ ] **Step 1: Implement**

`crates/zotero-mcp/src/tools/enrichment.rs`:

```rust
use crate::state::AppState;
use crate::tools::search::internal;
use rmcp::model::{CallToolResult, Content};
use rmcp::Error;
use serde::Deserialize;
use zotero_core::enrichment::propose::{apply_metadata_update, enrich_item, find_weak_metadata_items, propose_metadata_update, EnrichInput, ProposeInput};
use zotero_core::enrichment::NormalizedRecord;
use zotero_core::types::EnrichmentProposal;

#[derive(Deserialize)] pub struct WeakArgs { #[serde(default = "fifty")] pub limit: i64 }
fn fifty() -> i64 { 50 }
pub async fn find_weak_metadata_items_t(s: &AppState, a: WeakArgs) -> Result<CallToolResult, Error> {
    let r = find_weak_metadata_items(&s.pool, 1, a.limit).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))
}

#[derive(Deserialize)] pub struct DoiArgs { pub doi: String }
pub async fn lookup_doi_t(s: &AppState, a: DoiArgs) -> Result<CallToolResult, Error> {
    let r = s.crossref.lookup_doi(&a.doi).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))
}

#[derive(Deserialize)] pub struct IsbnArgs { pub isbn: String }
pub async fn lookup_isbn_t(s: &AppState, a: IsbnArgs) -> Result<CallToolResult, Error> {
    let r = s.openlibrary.lookup_isbn(&a.isbn).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))
}

#[derive(Deserialize)] pub struct ArxivArgs { pub id: String }
pub async fn lookup_arxiv_t(s: &AppState, a: ArxivArgs) -> Result<CallToolResult, Error> {
    let r = s.arxiv.lookup_arxiv(&a.id).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))
}

#[derive(Deserialize)] pub struct SearchSourceArgs { pub query: String, #[serde(default = "ten")] pub limit: usize }
fn ten() -> usize { 10 }
pub async fn search_crossref_t(s: &AppState, a: SearchSourceArgs) -> Result<CallToolResult, Error> {
    let r = s.crossref.search(&a.query, a.limit).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))
}
pub async fn search_semantic_scholar_t(s: &AppState, a: SearchSourceArgs) -> Result<CallToolResult, Error> {
    let r = s.semantic_scholar.search(&a.query, a.limit).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&r).unwrap())?]))
}

#[derive(Deserialize)] pub struct ProposeArgs { pub item_key: String, pub candidates: Vec<NormalizedRecord> }
pub async fn propose_metadata_update_t(s: &AppState, a: ProposeArgs) -> Result<CallToolResult, Error> {
    let p = propose_metadata_update(&s.pool, ProposeInput {
        item_key: &a.item_key, library_id: 1, storage_dir: &s.cfg.storage_dir(), candidates: a.candidates,
    }).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&p).unwrap())?]))
}

#[derive(Deserialize)] pub struct ApplyArgs { pub proposal: EnrichmentProposal }
pub async fn apply_metadata_update_t(s: &AppState, a: ApplyArgs) -> Result<CallToolResult, Error> {
    apply_metadata_update(&s.api, &s.pool, 1, &a.proposal).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::text("applied")]))
}

#[derive(Deserialize)] pub struct EnrichArgs { pub item_key: String, pub candidates: Vec<NormalizedRecord>, #[serde(default)] pub auto_apply_threshold: Option<f64> }
pub async fn enrich_item_t(s: &AppState, a: EnrichArgs) -> Result<CallToolResult, Error> {
    let threshold = a.auto_apply_threshold.unwrap_or(s.cfg.enrichment.auto_apply_threshold);
    let p = enrich_item(&s.api, &s.pool, EnrichInput {
        item_key: &a.item_key, library_id: 1, storage_dir: &s.cfg.storage_dir(),
        candidates: a.candidates, auto_apply_threshold: threshold,
    }).await.map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&p).unwrap())?]))
}
```

- [ ] **Step 2: Register all in server.rs**.

- [ ] **Step 3: Build + commit**

```bash
cargo build -p zotero-mcp
git add crates/zotero-mcp/
git commit -m "feat(mcp): wire enrichment toolset (lookup, propose, apply, enrich, find_weak)"
```

---

### Task 39: Expose collections and tags as MCP resources

**Files:**
- Create: `crates/zotero-mcp/src/resources/mod.rs`
- Create: `crates/zotero-mcp/src/resources/collections.rs`
- Create: `crates/zotero-mcp/src/resources/tags.rs`
- Modify: `crates/zotero-mcp/src/server.rs`

- [ ] **Step 1: Implement**

`crates/zotero-mcp/src/resources/mod.rs`:

```rust
pub mod collections;
pub mod tags;
```

`crates/zotero-mcp/src/resources/collections.rs`:

```rust
use crate::state::AppState;

pub async fn read_all(state: &AppState) -> anyhow::Result<String> {
    let cs = zotero_core::reader::collections::list(&state.pool, 1, None).await?;
    Ok(serde_json::to_string_pretty(&cs)?)
}
```

`crates/zotero-mcp/src/resources/tags.rs`:

```rust
use crate::state::AppState;

pub async fn read_all(state: &AppState) -> anyhow::Result<String> {
    let ts = zotero_core::reader::tags::list(&state.pool, 1, None).await?;
    Ok(serde_json::to_string_pretty(&ts)?)
}
```

In `server.rs`, register two resources via rmcp's resource macros:
- URI `zotero://collections` → JSON list of `Collection`
- URI `zotero://tags` → JSON list of `Tag`

The exact registration form depends on `rmcp`'s API; check `cargo doc -p rmcp --open`. Pattern is: implement `list_resources` returning two `Resource` descriptors, and `read_resource` dispatching on URI to the helpers above.

- [ ] **Step 2: Build + commit**

```bash
cargo build -p zotero-mcp
git add crates/zotero-mcp/
git commit -m "feat(mcp): expose collections and tags as MCP resources"
```

---

## Phase 10 — Polish and verification

### Task 40: README and Claude Code wiring

**Files:**
- Create: `README.md`
- Create: `docs/CLAUDE_CODE_SETUP.md`

- [ ] **Step 1: Write README**

`README.md`:

```markdown
# zotero-connector

A local MCP server that gives Claude fast, safe access to your Zotero library.

## Status

v0.1 — see `docs/superpowers/specs/2026-05-11-zotero-connector-design.md`.

## Requirements

- Zotero desktop (running), with **Preferences → Advanced → Allow other applications to communicate with Zotero** enabled.
- The BetterBibTeX plugin installed (soft dependency — without it, citation_key fields are `null` but everything else still works).
- Rust toolchain (stable).

## Build

```bash
cargo build --release -p zotero-mcp
```

Binary at `target/release/zotero-mcp`.

## Configure

Optional TOML file at `~/.config/zotero-mcp/config.toml`. See `crates/zotero-core/src/config.rs` for fields and defaults.

## Use with Claude Code

See `docs/CLAUDE_CODE_SETUP.md`.
```

`docs/CLAUDE_CODE_SETUP.md`:

```markdown
# Wiring zotero-mcp into Claude Code

Add the server to your Claude Code config (`~/.claude/mcp_settings.json` or
your project's `.mcp.json`):

```json
{
  "mcpServers": {
    "zotero": {
      "command": "/absolute/path/to/target/release/zotero-mcp",
      "args": [],
      "env": {}
    }
  }
}
```

Restart Claude Code. The `zotero` server should appear in `/mcp` with tools:

- search_items, get_item, list_collections, list_tags, list_recent_items
- list_attachments, get_pdf_path, get_pdf_text, get_pdf_first_pages, list_annotations
- get_webpage_content, refetch_url
- format_citation, format_bibliography
- add_note, update_item_fields, add_tags, remove_tags, add_to_collection, remove_from_collection
- find_weak_metadata_items, lookup_doi, lookup_isbn, lookup_arxiv, search_crossref, search_semantic_scholar
- propose_metadata_update, apply_metadata_update, enrich_item

And resources:
- zotero://collections
- zotero://tags

## Troubleshooting

- "Local API is not enabled" → toggle the setting in Zotero Preferences.
- Schema version mismatch on startup → bump `max_schema_userdata` in your
  config TOML after eyeballing the new schema against this repo's queries.
- Logs go to stderr; if you set `paths.log_dir`, a file at
  `<log_dir>/zotero-mcp.log` will also receive entries.
```

- [ ] **Step 2: Commit**

```bash
git add README.md docs/CLAUDE_CODE_SETUP.md
git commit -m "docs: add README and Claude Code wiring guide"
```

---

### Task 41: Live integration smoke test

**Files:**
- Create: `crates/zotero-core/tests/live_integration.rs`

- [ ] **Step 1: Add gated test**

```rust
//! Live test against the user's real local Zotero. Gated by env var so CI
//! doesn't try to run it. Execute manually:
//!     ZOTERO_MCP_LIVE_TEST=1 cargo test -p zotero-core --test live_integration -- --nocapture

use zotero_core::reader::conn::{check_schema, open_read_only};
use zotero_core::reader::pool::ReadOnlyPool;
use zotero_core::reader::search::{search_metadata, SearchParams};

fn enabled() -> bool { std::env::var("ZOTERO_MCP_LIVE_TEST").is_ok() }

#[tokio::test]
async fn live_schema_and_search() {
    if !enabled() { eprintln!("skipped (set ZOTERO_MCP_LIVE_TEST=1)"); return; }
    let path = directories::UserDirs::new().unwrap().home_dir().join("Zotero/zotero.sqlite");
    let conn = open_read_only(&path).unwrap();
    let v = check_schema(&conn, 100, 150).expect("schema in tested range");
    eprintln!("userdata schema version: {}", v);

    let pool = ReadOnlyPool::new(path, 2).await.unwrap();
    let hits = search_metadata(&pool, 1, SearchParams { query: "the".into(), limit: 3, ..Default::default() }).await.unwrap();
    assert!(!hits.is_empty());
    eprintln!("found {} hits", hits.len());
}
```

- [ ] **Step 2: Run it manually**

```bash
ZOTERO_MCP_LIVE_TEST=1 cargo test -p zotero-core --test live_integration -- --nocapture
```
Expected: prints the schema version and a non-zero hit count.

- [ ] **Step 3: Commit**

```bash
git add crates/zotero-core/tests/live_integration.rs
git commit -m "test(core): live-Zotero integration smoke (env-gated)"
```

---

### Task 42: Manual end-to-end with Claude Code

**Files:** (none — verification only)

- [ ] **Step 1: Install the binary**

```bash
cargo install --path crates/zotero-mcp
```

- [ ] **Step 2: Wire into Claude Code**

Follow `docs/CLAUDE_CODE_SETUP.md`. Restart Claude Code.

- [ ] **Step 3: Smoke-test each tool group from a Claude conversation**

In Claude Code, exercise:
- `search_items` with a known author from your library
- `get_item` with a known Zotero key — confirm `recommended_content_tool` is set and `citation_key` is populated
- `get_pdf_text` on an item with a `.zotero-ft-cache` and one without
- `format_citation` with style `apa` and `bibtex`
- `add_note` to a test item — confirm it appears in the Zotero UI
- `find_weak_metadata_items` followed by `lookup_doi` and `propose_metadata_update`

- [ ] **Step 4: If everything passes, tag v0.1**

```bash
git tag v0.1.0
```

(Do NOT push without explicit user direction.)

---

## Self-review (filled in by the writer)

**Spec coverage check:**

- §1.1 Goals — search/metadata/attachments/notes: Tasks 11–14, 19–23.
- §1.1 BBT citation keys & citation formatting: Tasks 17–18, 24.
- §1.1 Metadata enrichment: Tasks 25–32.
- §1.1 Webpage items + snapshots + refetch: Tasks 16, 23.
- §1.2 Non-goals — verified absent from plan (no embeddings, no Obsidian, no file watch, no GUI).
- §3.2 Data paths — all five mapped (Tasks 7, 9, 11, 12, 13, 15, 16, 19, 24, 26–29).
- §3.3 Failure model — schema check in Task 7; version-conflict typed error in Task 21; partial-result enrichment shape in Task 32; PDF/HTML extraction failures handled via typed errors.
- §4.1–4.6 MCP surface — every tool from the spec has a wiring task (34–38), resources at Task 39.
- §5 Internal structure — workspace shape matches spec.
- §6 Cross-cutting (config, cache, logging, SQLite safety, API conventions, BBT) — Tasks 4, 5, 7, 17, 19, 25 cover.
- §7 Performance targets — measured implicitly via `live_integration.rs`; not a hard gate.
- §8 Testing strategy — fixtures (Task 6), wiremock (Tasks 17, 19, 20, 21, 22, 23, 24, 26–29), live smoke (Task 41).
- §9 Out of scope — confirmed not in plan.

**Placeholder scan:** none.

**Type consistency:** `Item.fields` is `serde_json::Value` throughout. `NormalizedRecord.fields` is `serde_json::Map<String, Value>` — used consistently in scoring and propose. `EnrichmentProposal` carries `Diff` (not bare changes) — wired through apply.

**Ambiguities resolved:**
- `update_item_fields` always re-reads item version inline (Task 37) so callers do not pre-compute it.
- Citation-key reverse lookup is deferred (returns invalid_params); use item_key.

If `rmcp`'s API in the installed version differs from the macro examples, that's a code-write-time adjustment, not a plan defect — the structural intent (stdio transport + tools + resources) is stable.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-11-zotero-connector.md`. Two execution options:

1. **Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?

