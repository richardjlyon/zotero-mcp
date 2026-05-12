# OAuth Token Durability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make OAuth tokens survive launchd restarts and add the `refresh_token` grant so Cowork users (and future spec-compliant Claude.ai clients) can go a full working day without re-authenticating in the browser.

**Architecture:** Move token storage out of in-memory HashMaps into a 0600 JSON file at `<config_dir>/tokens.json` via a new `TokenStore` module. Tokens are stored as SHA-256 hashes at rest. Auth-code grants now mint a `(access, refresh)` pair sharing a `chain_id`. The new `refresh_token` grant rotates the refresh token (one-time-use); replay of a consumed refresh triggers chain-wide revocation per OAuth 2.1 §4.3.1.

**Tech Stack:** Rust 2024, axum 0.8, tokio sync primitives, sha2, base64, the existing `rand`/`directories`/`toml` crates already in the workspace. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-05-12-oauth-token-durability-design.md` (commit `09a2dcf`).

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `crates/zotero-mcp/src/oauth/token_store.rs` | **CREATE** | All token persistence: load/save, mint, validate, consume, revoke. Self-contained. |
| `crates/zotero-mcp/src/oauth.rs` | **MODIFY** | Wire `TokenStore` into `OAuthState`. Extend `OAuthConfig` with TTL fields. Add `refresh_token` grant arm. Update discovery doc. Update `TokenResponse` shape. Declare `mod token_store;`. |
| `crates/zotero-mcp/src/http_transport.rs` | **MODIFY** | Add the cross-module restart-survival integration test. (No production-code change — `OAuthState`'s public surface stays the same.) |
| `crates/zotero-mcp/src/main.rs` | **MODIFY** | If `OAuthState::new` signature changed, update the construction site. |

The existing `oauth.rs` becomes the parent of a new `oauth::token_store` submodule. Rust 2018+ permits `oauth.rs` and `oauth/` as siblings — no rename of `oauth.rs` to `mod.rs` is needed.

---

## Task 1: Add configurable TTL fields to `OAuthConfig`

**Files:**
- Modify: `crates/zotero-mcp/src/oauth.rs` (struct `OAuthConfig`, lines 58–66)
- Test: same file, existing `mod tests`

- [ ] **Step 1: Write failing test**

Add to `mod tests` in `oauth.rs`:

```rust
#[test]
fn config_loads_with_default_ttls_when_unset() {
    let toml_str = r#"
        client_id = "x"
        client_secret = "y"
        issuer = "https://example.test"
    "#;
    let cfg: OAuthConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.access_token_ttl_secs, None);
    assert_eq!(cfg.refresh_token_ttl_secs, None);
    assert_eq!(cfg.effective_access_ttl().as_secs(), 7 * 24 * 3600);
    assert_eq!(cfg.effective_refresh_ttl().as_secs(), 90 * 24 * 3600);
}

#[test]
fn config_loads_with_explicit_ttls() {
    let toml_str = r#"
        client_id = "x"
        client_secret = "y"
        issuer = "https://example.test"
        access_token_ttl_secs = 3600
        refresh_token_ttl_secs = 86400
    "#;
    let cfg: OAuthConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.effective_access_ttl().as_secs(), 3600);
    assert_eq!(cfg.effective_refresh_ttl().as_secs(), 86400);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --package zotero-mcp --lib oauth::tests::config_loads
```

Expected: compilation error — fields don't exist; method `effective_access_ttl` not found.

- [ ] **Step 3: Implement the change**

Replace the `OAuthConfig` struct (currently `oauth.rs:58–66`) with:

```rust
pub const DEFAULT_ACCESS_TOKEN_TTL_SECS: u64 = 7 * 24 * 3600;     // 7 days
pub const DEFAULT_REFRESH_TOKEN_TTL_SECS: u64 = 90 * 24 * 3600;   // 90 days

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub issuer: String,
    #[serde(default)]
    pub access_token_ttl_secs: Option<u64>,
    #[serde(default)]
    pub refresh_token_ttl_secs: Option<u64>,
}

impl OAuthConfig {
    pub fn effective_access_ttl(&self) -> std::time::Duration {
        std::time::Duration::from_secs(
            self.access_token_ttl_secs.unwrap_or(DEFAULT_ACCESS_TOKEN_TTL_SECS),
        )
    }
    pub fn effective_refresh_ttl(&self) -> std::time::Duration {
        std::time::Duration::from_secs(
            self.refresh_token_ttl_secs.unwrap_or(DEFAULT_REFRESH_TOKEN_TTL_SECS),
        )
    }
}
```

Keep the existing `impl OAuthConfig { pub fn load_or_generate(...) ... }` block intact — these new methods sit alongside it.

Update the existing `OAuthConfig` literal in `config_roundtrips_through_disk_with_secure_perms` (around `oauth.rs:611`) to set the new fields:

```rust
let original = OAuthConfig {
    client_id: "id-x".into(),
    client_secret: "secret-y".into(),
    issuer: "https://example.test".into(),
    access_token_ttl_secs: None,
    refresh_token_ttl_secs: None,
};
```

Same for `test_state()` at `oauth.rs:642`:

```rust
fn test_state() -> OAuthState {
    OAuthState::new(OAuthConfig {
        client_id: "test-id".into(),
        client_secret: "test-secret".into(),
        issuer: "https://example.test".into(),
        access_token_ttl_secs: None,
        refresh_token_ttl_secs: None,
    })
}
```

And the same pattern in `test_oauth_state()` in `http_transport.rs:238`.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --package zotero-mcp --lib oauth
```

Expected: all existing tests pass plus the two new TTL tests.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/oauth.rs crates/zotero-mcp/src/http_transport.rs
git commit -m "feat(oauth): configurable TTL fields with 7d/90d defaults"
```

---

## Task 2: Create `TokenStore` skeleton with `load()` for missing/corrupt files

**Files:**
- Create: `crates/zotero-mcp/src/oauth/token_store.rs`
- Modify: `crates/zotero-mcp/src/oauth.rs` (add `mod token_store;` declaration)

- [ ] **Step 1: Declare the submodule**

Near the top of `crates/zotero-mcp/src/oauth.rs` (after the doc comment, before the `use` statements), add:

```rust
mod token_store;
pub use token_store::{ChainId, RefreshError, TokenStore};
```

- [ ] **Step 2: Create the new module file with skeleton**

Create `crates/zotero-mcp/src/oauth/token_store.rs`:

```rust
//! File-backed access + refresh token store for the OAuth surface.
//!
//! Tokens are stored as `sha256(raw_token)` hex strings — never plaintext —
//! so the on-disk file does not contain bearer values that would be valid
//! if leaked. Validation uses constant-time comparison of digests.
//!
//! Refresh tokens are one-time-use. A refresh token presented twice signals
//! a leak (per RFC 6749 §10.4 / OAuth 2.1 §4.3.1) and triggers revocation of
//! the entire `chain_id` family.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

const SCHEMA_VERSION: u32 = 1;

pub type ChainId = String;

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error("unknown refresh token")]
    Unknown,
    #[error("refresh token expired")]
    Expired,
    #[error("refresh token already consumed (replay attack signal); chain {0} revoked")]
    Replayed(ChainId),
}

/// One stored access token (in-memory and on-disk shape).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct AccessRecord {
    token_hash: String,
    expires_at: u64,
    chain_id: ChainId,
}

/// One stored refresh token.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct RefreshRecord {
    token_hash: String,
    expires_at: u64,
    chain_id: ChainId,
    /// Unix seconds when this refresh token was first consumed via the
    /// `refresh_token` grant. `None` while still usable. We keep consumed
    /// records around (until natural expiry) so we can detect replay.
    consumed_at: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    /// Hex SHA-256 of the OAuth client_id at the time the file was written.
    /// Mismatch on load means the user regenerated `oauth.toml`; we wipe.
    client_id_hash: String,
    access: Vec<AccessRecord>,
    refresh: Vec<RefreshRecord>,
    revoked_chains: Vec<ChainId>,
}

/// Index built over `Snapshot` for O(1) token lookup. Rebuilt on load and
/// after every mutation.
#[derive(Default)]
struct Index {
    access_by_hash: HashMap<String, AccessRecord>,
    refresh_by_hash: HashMap<String, RefreshRecord>,
    revoked: std::collections::HashSet<ChainId>,
}

pub struct TokenStore {
    inner: Arc<Inner>,
}

struct Inner {
    state: RwLock<Index>,
    path: PathBuf,
    access_ttl: Duration,
    refresh_ttl: Duration,
    client_id_hash: String,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{:02x}", byte);
    }
    hex
}

impl TokenStore {
    /// Load (or initialize) the store at `path`.
    ///
    /// - Missing file → empty store (info log).
    /// - Unreadable/corrupt file → rename aside to `tokens.json.broken-{ts}`, start empty (warn log).
    /// - `client_id_hash` mismatch → wipe (warn log). Handles `oauth.toml` regeneration.
    /// - Otherwise → drop expired entries and load.
    pub fn load(
        path: PathBuf,
        client_id: &str,
        access_ttl: Duration,
        refresh_ttl: Duration,
    ) -> anyhow::Result<Self> {
        let client_id_hash = sha256_hex(client_id);
        let mut snapshot = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Snapshot>(&bytes) {
                Ok(snap) if snap.version == SCHEMA_VERSION => snap,
                Ok(_) | Err(_) => {
                    let backup =
                        path.with_extension(format!("json.broken-{}", unix_now()));
                    let _ = std::fs::rename(&path, &backup);
                    tracing::warn!(
                        path = %path.display(),
                        backup = %backup.display(),
                        "tokens.json corrupt or wrong schema version; renamed aside, starting fresh"
                    );
                    Snapshot::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(path = %path.display(), "no tokens.json found; starting fresh");
                Snapshot::default()
            }
            Err(e) => return Err(anyhow::anyhow!("read {}: {e}", path.display())),
        };

        if !snapshot.client_id_hash.is_empty() && snapshot.client_id_hash != client_id_hash {
            tracing::warn!(
                "tokens.json client_id_hash mismatch; wiping (oauth.toml was likely regenerated)"
            );
            snapshot = Snapshot::default();
        }
        snapshot.client_id_hash = client_id_hash.clone();
        snapshot.version = SCHEMA_VERSION;

        let now = unix_now();
        snapshot.access.retain(|r| r.expires_at > now);
        snapshot.refresh.retain(|r| r.expires_at > now);

        let mut index = Index::default();
        for r in &snapshot.access {
            index.access_by_hash.insert(r.token_hash.clone(), r.clone());
        }
        for r in &snapshot.refresh {
            index.refresh_by_hash.insert(r.token_hash.clone(), r.clone());
        }
        for c in &snapshot.revoked_chains {
            index.revoked.insert(c.clone());
        }

        Ok(Self {
            inner: Arc::new(Inner {
                state: RwLock::new(index),
                path,
                access_ttl,
                refresh_ttl,
                client_id_hash,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh(dir: &TempDir) -> TokenStore {
        TokenStore::load(
            dir.path().join("tokens.json"),
            "client-id-1",
            Duration::from_secs(60),
            Duration::from_secs(600),
        )
        .unwrap()
    }

    #[test]
    fn load_treats_missing_file_as_empty() {
        let dir = TempDir::new().unwrap();
        let store = fresh(&dir);
        // Just confirms construction succeeds with no file present.
        let _ = store;
    }

    #[test]
    fn load_renames_corrupt_file_aside_and_starts_fresh() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("tokens.json");
        std::fs::write(&path, b"this is not valid json").unwrap();
        let _ = TokenStore::load(
            path.clone(),
            "client-id-1",
            Duration::from_secs(60),
            Duration::from_secs(600),
        )
        .unwrap();
        assert!(!path.exists(), "original corrupt file should have been moved aside");
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            entries.iter().any(|name| name.starts_with("tokens.json.broken-")),
            "expected backup file, got {entries:?}"
        );
    }
}
```

- [ ] **Step 3: Run the tests**

```bash
cargo test --package zotero-mcp --lib oauth::token_store
```

Expected: both tests pass. If `thiserror` isn't already imported, the compile will fail — check `Cargo.toml`. (`thiserror.workspace = true` is already a dependency per the file we read.)

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-mcp/src/oauth.rs crates/zotero-mcp/src/oauth/token_store.rs
git commit -m "feat(oauth): TokenStore skeleton with load() handling missing/corrupt files"
```

---

## Task 3: `TokenStore::load()` — wipe on `client_id_hash` mismatch + drop expired

**Files:**
- Test only: `crates/zotero-mcp/src/oauth/token_store.rs` (extend `mod tests`)

The implementation already covers these cases (Task 2's `load()` handles them). This task just adds the regression tests so we know the behavior is locked.

- [ ] **Step 1: Write tests**

Append to `mod tests` in `token_store.rs`:

```rust
#[test]
fn load_wipes_store_on_client_id_hash_mismatch() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("tokens.json");
    let snap = Snapshot {
        version: SCHEMA_VERSION,
        client_id_hash: sha256_hex("OLD-CLIENT-ID"),
        access: vec![AccessRecord {
            token_hash: "deadbeef".into(),
            expires_at: unix_now() + 9999,
            chain_id: "chain-x".into(),
        }],
        refresh: vec![],
        revoked_chains: vec![],
    };
    std::fs::write(&path, serde_json::to_vec(&snap).unwrap()).unwrap();

    let store = TokenStore::load(
        path,
        "NEW-CLIENT-ID",
        Duration::from_secs(60),
        Duration::from_secs(600),
    )
    .unwrap();
    let idx = store.inner.state.try_read().unwrap();
    assert!(idx.access_by_hash.is_empty(), "tokens issued under old client_id must be wiped");
}

#[test]
fn load_drops_expired_access_and_refresh_records() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("tokens.json");
    let now = unix_now();
    let snap = Snapshot {
        version: SCHEMA_VERSION,
        client_id_hash: sha256_hex("client-id-1"),
        access: vec![
            AccessRecord {
                token_hash: "fresh-access".into(),
                expires_at: now + 600,
                chain_id: "c1".into(),
            },
            AccessRecord {
                token_hash: "stale-access".into(),
                expires_at: now - 1,
                chain_id: "c1".into(),
            },
        ],
        refresh: vec![
            RefreshRecord {
                token_hash: "fresh-refresh".into(),
                expires_at: now + 600,
                chain_id: "c1".into(),
                consumed_at: None,
            },
            RefreshRecord {
                token_hash: "stale-refresh".into(),
                expires_at: now - 1,
                chain_id: "c1".into(),
                consumed_at: None,
            },
        ],
        revoked_chains: vec![],
    };
    std::fs::write(&path, serde_json::to_vec(&snap).unwrap()).unwrap();
    let store = fresh_with_path(&path);
    let idx = store.inner.state.try_read().unwrap();
    assert!(idx.access_by_hash.contains_key("fresh-access"));
    assert!(!idx.access_by_hash.contains_key("stale-access"));
    assert!(idx.refresh_by_hash.contains_key("fresh-refresh"));
    assert!(!idx.refresh_by_hash.contains_key("stale-refresh"));
}

fn fresh_with_path(path: &Path) -> TokenStore {
    TokenStore::load(
        path.to_path_buf(),
        "client-id-1",
        Duration::from_secs(60),
        Duration::from_secs(600),
    )
    .unwrap()
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test --package zotero-mcp --lib oauth::token_store
```

Expected: all four tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/zotero-mcp/src/oauth/token_store.rs
git commit -m "test(oauth): cover client_id_hash wipe + expired-record pruning"
```

---

## Task 4: `TokenStore::mint_pair()` with atomic persistence

**Files:**
- Modify: `crates/zotero-mcp/src/oauth/token_store.rs`

- [ ] **Step 1: Write failing tests**

Append to `mod tests`:

```rust
#[tokio::test]
async fn mint_pair_returns_two_distinct_tokens() {
    let dir = TempDir::new().unwrap();
    let store = fresh(&dir);
    let pair = store.mint_pair(None).await.unwrap();
    assert_ne!(pair.access_token, pair.refresh_token);
    assert!(pair.access_token.len() >= 32);
    assert!(pair.refresh_token.len() >= 32);
    assert!(!pair.chain_id.is_empty());
}

#[tokio::test]
async fn mint_pair_persists_to_disk_with_mode_0600() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("tokens.json");
    let store = fresh_with_path(&path);
    let _ = store.mint_pair(None).await.unwrap();
    assert!(path.exists(), "mint_pair must persist to disk");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[tokio::test]
async fn tokens_at_rest_are_hashed_not_plaintext() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("tokens.json");
    let store = fresh_with_path(&path);
    let pair = store.mint_pair(None).await.unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let body = String::from_utf8(bytes).unwrap();
    assert!(
        !body.contains(&pair.access_token),
        "raw access token must not appear on disk"
    );
    assert!(
        !body.contains(&pair.refresh_token),
        "raw refresh token must not appear on disk"
    );
    assert!(
        body.contains(&sha256_hex(&pair.access_token)),
        "expected access-token hash in file"
    );
}

#[tokio::test]
async fn mint_pair_with_existing_chain_id_keeps_chain() {
    let dir = TempDir::new().unwrap();
    let store = fresh(&dir);
    let first = store.mint_pair(None).await.unwrap();
    let second = store
        .mint_pair(Some(first.chain_id.clone()))
        .await
        .unwrap();
    assert_eq!(first.chain_id, second.chain_id);
    assert_ne!(first.access_token, second.access_token);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --package zotero-mcp --lib oauth::token_store::tests::mint
```

Expected: compilation error — no `mint_pair` method.

- [ ] **Step 3: Implement `mint_pair`, `persist`, and `MintedPair`**

Add to `impl TokenStore`:

```rust
/// Result of `mint_pair`. Caller is expected to return these to the OAuth client.
#[derive(Debug, Clone)]
pub struct MintedPair {
    pub access_token: String,
    pub refresh_token: String,
    pub access_ttl: Duration,
    pub refresh_ttl: Duration,
    pub chain_id: ChainId,
}

impl TokenStore {
    /// Mint a new (access, refresh) pair. Pass `None` for `chain_id` to start
    /// a new chain (use case: `authorization_code` grant). Pass `Some(id)` to
    /// continue an existing chain (use case: `refresh_token` grant rotation).
    /// Persists to disk before returning. On persist failure logs an error and
    /// keeps the in-memory state — the caller still gets a valid pair.
    pub async fn mint_pair(
        &self,
        chain_id: Option<ChainId>,
    ) -> anyhow::Result<MintedPair> {
        let chain_id = chain_id.unwrap_or_else(opaque_id);
        let access_token = opaque_id();
        let refresh_token = opaque_id();
        let now = unix_now();
        let access_record = AccessRecord {
            token_hash: sha256_hex(&access_token),
            expires_at: now + self.inner.access_ttl.as_secs(),
            chain_id: chain_id.clone(),
        };
        let refresh_record = RefreshRecord {
            token_hash: sha256_hex(&refresh_token),
            expires_at: now + self.inner.refresh_ttl.as_secs(),
            chain_id: chain_id.clone(),
            consumed_at: None,
        };

        {
            let mut idx = self.inner.state.write().await;
            idx.access_by_hash.insert(access_record.token_hash.clone(), access_record);
            idx.refresh_by_hash.insert(refresh_record.token_hash.clone(), refresh_record);
            self.persist_locked(&idx);
        }

        Ok(MintedPair {
            access_token,
            refresh_token,
            access_ttl: self.inner.access_ttl,
            refresh_ttl: self.inner.refresh_ttl,
            chain_id,
        })
    }

    /// Serialize the current index back to a Snapshot and atomically write
    /// the file (temp + rename). On failure, log and continue.
    fn persist_locked(&self, idx: &Index) {
        let mut access: Vec<_> = idx.access_by_hash.values().cloned().collect();
        let mut refresh: Vec<_> = idx.refresh_by_hash.values().cloned().collect();
        access.sort_by(|a, b| a.token_hash.cmp(&b.token_hash));
        refresh.sort_by(|a, b| a.token_hash.cmp(&b.token_hash));
        let mut revoked: Vec<_> = idx.revoked.iter().cloned().collect();
        revoked.sort();

        let snap = Snapshot {
            version: SCHEMA_VERSION,
            client_id_hash: self.inner.client_id_hash.clone(),
            access,
            refresh,
            revoked_chains: revoked,
        };
        let bytes = match serde_json::to_vec_pretty(&snap) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "could not serialize token snapshot; in-memory state preserved");
                return;
            }
        };
        if let Err(e) = atomic_write_0600(&self.inner.path, &bytes) {
            tracing::error!(
                path = %self.inner.path.display(),
                error = %e,
                "could not persist tokens.json; in-memory state preserved"
            );
        }
    }
}

fn opaque_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

fn atomic_write_0600(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("json.tmp.{:08x}", rand::random::<u32>()));
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}
```

Re-export `MintedPair` from the parent module by editing the line you added in Task 2 inside `oauth.rs`:

```rust
pub use token_store::{ChainId, MintedPair, RefreshError, TokenStore};
```

- [ ] **Step 4: Run tests**

```bash
cargo test --package zotero-mcp --lib oauth::token_store
```

Expected: all tests in this module pass.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/oauth/token_store.rs crates/zotero-mcp/src/oauth.rs
git commit -m "feat(oauth): TokenStore::mint_pair with atomic 0600 persistence"
```

---

## Task 5: `TokenStore::validate_access()`

**Files:**
- Modify: `crates/zotero-mcp/src/oauth/token_store.rs`

- [ ] **Step 1: Write failing tests**

Append to `mod tests`:

```rust
#[tokio::test]
async fn validate_access_returns_true_for_freshly_minted_token() {
    let dir = TempDir::new().unwrap();
    let store = fresh(&dir);
    let pair = store.mint_pair(None).await.unwrap();
    assert!(store.validate_access(&pair.access_token).await);
}

#[tokio::test]
async fn validate_access_returns_false_for_unknown_token() {
    let dir = TempDir::new().unwrap();
    let store = fresh(&dir);
    assert!(!store.validate_access("not-a-real-token").await);
}

#[tokio::test]
async fn validate_access_returns_false_after_expiry() {
    let dir = TempDir::new().unwrap();
    let store = TokenStore::load(
        dir.path().join("tokens.json"),
        "client-id-1",
        Duration::from_secs(0),    // immediate expiry
        Duration::from_secs(600),
    )
    .unwrap();
    let pair = store.mint_pair(None).await.unwrap();
    // Sleep 1s so unix-second resolution lapses past expires_at.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!store.validate_access(&pair.access_token).await);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --package zotero-mcp --lib oauth::token_store::tests::validate
```

Expected: compilation error — no `validate_access` method.

- [ ] **Step 3: Implement**

Add to `impl TokenStore`:

```rust
/// Validate an access token. Returns `true` iff the token was issued, has
/// not expired, and its chain has not been revoked.
pub async fn validate_access(&self, raw: &str) -> bool {
    let hash = sha256_hex(raw);
    let idx = self.inner.state.read().await;
    let Some(record) = idx.access_by_hash.get(&hash) else {
        return false;
    };
    if record.expires_at <= unix_now() {
        return false;
    }
    if idx.revoked.contains(&record.chain_id) {
        return false;
    }
    true
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test --package zotero-mcp --lib oauth::token_store
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/oauth/token_store.rs
git commit -m "feat(oauth): TokenStore::validate_access with expiry + revocation checks"
```

---

## Task 6: `TokenStore::consume_refresh()` with replay detection

**Files:**
- Modify: `crates/zotero-mcp/src/oauth/token_store.rs`

- [ ] **Step 1: Write failing tests**

Append to `mod tests`:

```rust
#[tokio::test]
async fn consume_refresh_returns_chain_id_on_first_use() {
    let dir = TempDir::new().unwrap();
    let store = fresh(&dir);
    let pair = store.mint_pair(None).await.unwrap();
    let chain = store.consume_refresh(&pair.refresh_token).await.unwrap();
    assert_eq!(chain, pair.chain_id);
}

#[tokio::test]
async fn consume_refresh_replay_returns_replayed_with_chain_id() {
    let dir = TempDir::new().unwrap();
    let store = fresh(&dir);
    let pair = store.mint_pair(None).await.unwrap();
    let _first = store.consume_refresh(&pair.refresh_token).await.unwrap();
    let err = store.consume_refresh(&pair.refresh_token).await.unwrap_err();
    match err {
        RefreshError::Replayed(chain) => assert_eq!(chain, pair.chain_id),
        other => panic!("expected Replayed, got {other:?}"),
    }
}

#[tokio::test]
async fn consume_refresh_unknown_returns_unknown() {
    let dir = TempDir::new().unwrap();
    let store = fresh(&dir);
    let err = store.consume_refresh("never-issued").await.unwrap_err();
    assert!(matches!(err, RefreshError::Unknown), "got {err:?}");
}

#[tokio::test]
async fn consume_refresh_expired_returns_expired() {
    let dir = TempDir::new().unwrap();
    let store = TokenStore::load(
        dir.path().join("tokens.json"),
        "client-id-1",
        Duration::from_secs(60),
        Duration::from_secs(0),    // refresh expires immediately
    )
    .unwrap();
    let pair = store.mint_pair(None).await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let err = store.consume_refresh(&pair.refresh_token).await.unwrap_err();
    assert!(matches!(err, RefreshError::Expired), "got {err:?}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --package zotero-mcp --lib oauth::token_store::tests::consume
```

Expected: compilation error — no `consume_refresh` method.

- [ ] **Step 3: Implement**

Add to `impl TokenStore`:

```rust
/// Consume a refresh token. Returns the chain_id on success.
///
/// Returns `Replayed { chain_id }` if the token was already consumed —
/// this is a leak signal and the caller MUST follow up with
/// `revoke_chain(chain_id)`. Returns `Unknown` for a refresh token in an
/// already-revoked chain (we don't disclose chain identity to a caller
/// who doesn't already know it).
pub async fn consume_refresh(&self, raw: &str) -> Result<ChainId, RefreshError> {
    let hash = sha256_hex(raw);
    let mut idx = self.inner.state.write().await;
    let now = unix_now();

    // Inspect first (immutable borrow only) — we need to drop this borrow
    // before the second get_mut so the borrow checker is happy with the
    // subsequent self.persist_locked(&idx) call.
    let (chain_id, expires_at, consumed_at) = match idx.refresh_by_hash.get(&hash) {
        Some(r) => (r.chain_id.clone(), r.expires_at, r.consumed_at),
        None => return Err(RefreshError::Unknown),
    };
    if idx.revoked.contains(&chain_id) {
        return Err(RefreshError::Unknown);
    }
    if expires_at <= now {
        return Err(RefreshError::Expired);
    }
    if consumed_at.is_some() {
        return Err(RefreshError::Replayed(chain_id));
    }

    // Mutate (fresh mutable borrow now that the read borrow is gone).
    idx.refresh_by_hash.get_mut(&hash).unwrap().consumed_at = Some(now);
    self.persist_locked(&idx);
    Ok(chain_id)
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test --package zotero-mcp --lib oauth::token_store
```

Expected: all tests in module pass.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/oauth/token_store.rs
git commit -m "feat(oauth): TokenStore::consume_refresh with replay detection"
```

---

## Task 7: `TokenStore::revoke_chain()`

**Files:**
- Modify: `crates/zotero-mcp/src/oauth/token_store.rs`

- [ ] **Step 1: Write failing tests**

Append to `mod tests`:

```rust
#[tokio::test]
async fn revoke_chain_invalidates_all_access_tokens_in_chain() {
    let dir = TempDir::new().unwrap();
    let store = fresh(&dir);
    let pair = store.mint_pair(None).await.unwrap();
    assert!(store.validate_access(&pair.access_token).await);
    store.revoke_chain(pair.chain_id.clone()).await;
    assert!(!store.validate_access(&pair.access_token).await);
}

#[tokio::test]
async fn revoke_chain_invalidates_subsequent_refresh_consumption() {
    let dir = TempDir::new().unwrap();
    let store = fresh(&dir);
    let pair = store.mint_pair(None).await.unwrap();
    store.revoke_chain(pair.chain_id.clone()).await;
    let err = store.consume_refresh(&pair.refresh_token).await.unwrap_err();
    // Replay would be wrong — the token was never consumed; correct error
    // is that the chain is revoked. We treat that as Unknown for the caller
    // (no point telling the world which chain). Adjust below if needed.
    assert!(
        matches!(err, RefreshError::Unknown),
        "revoked-chain refresh should look Unknown to callers; got {err:?}"
    );
}

#[tokio::test]
async fn revoke_chain_persists_to_disk() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("tokens.json");
    let store = fresh_with_path(&path);
    let pair = store.mint_pair(None).await.unwrap();
    store.revoke_chain(pair.chain_id.clone()).await;
    drop(store);
    let store2 = fresh_with_path(&path);
    assert!(!store2.validate_access(&pair.access_token).await);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --package zotero-mcp --lib oauth::token_store::tests::revoke
```

Expected: compilation error — no `revoke_chain` method.

- [ ] **Step 3: Implement**

Add to `impl TokenStore`:

```rust
/// Mark a chain as revoked. All access tokens in this chain stop validating
/// immediately; any refresh tokens in this chain stop being consumable.
pub async fn revoke_chain(&self, chain_id: ChainId) {
    let mut idx = self.inner.state.write().await;
    idx.revoked.insert(chain_id);
    self.persist_locked(&idx);
}
```

The revoked-chain check is already part of `consume_refresh` (added in Task 6). No further changes to that function are required here — just the new `revoke_chain` method above.

- [ ] **Step 4: Run tests**

```bash
cargo test --package zotero-mcp --lib oauth::token_store
```

Expected: all tests in module pass.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/oauth/token_store.rs
git commit -m "feat(oauth): TokenStore::revoke_chain with persistence"
```

---

## Task 8: Wire `TokenStore` into `OAuthState`

**Files:**
- Modify: `crates/zotero-mcp/src/oauth.rs` — replace the in-memory `tokens` map with `TokenStore`.

- [ ] **Step 1: Modify `Inner` and `OAuthState::new`**

Replace the `Inner` struct in `oauth.rs` (currently lines 161–170) with:

```rust
struct Inner {
    config: OAuthConfig,
    /// In-memory authorization-code store. Codes are single-use, 5-minute TTL —
    /// surviving a server restart is not a goal for this short-lived state.
    codes: RwLock<HashMap<String, AuthCode>>,
    tokens: TokenStore,
}
```

Replace `OAuthState::new` (currently lines 180–188) with:

```rust
impl OAuthState {
    /// Construct an OAuthState backed by a TokenStore at `tokens_path`.
    /// Use `OAuthState::with_tokens_path` to supply the path explicitly,
    /// or `OAuthState::from_config_path` to derive it from the standard
    /// ProjectDirs location (test code uses the former, production uses the latter).
    pub fn with_tokens_path(config: OAuthConfig, tokens_path: PathBuf) -> anyhow::Result<Self> {
        let access_ttl = config.effective_access_ttl();
        let refresh_ttl = config.effective_refresh_ttl();
        let tokens = TokenStore::load(tokens_path, &config.client_id, access_ttl, refresh_ttl)?;
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                codes: RwLock::new(HashMap::new()),
                tokens,
            }),
        })
    }

    /// Standard production constructor: derive the tokens path from the
    /// same ProjectDirs base used by `oauth.toml`.
    pub fn from_default_path(config: OAuthConfig) -> anyhow::Result<Self> {
        let dir = directories::ProjectDirs::from("dev", "zotero-mcp", "zotero-mcp")
            .ok_or_else(|| anyhow::anyhow!("could not resolve ProjectDirs for tokens.json"))?
            .config_dir()
            .to_path_buf();
        Self::with_tokens_path(config, dir.join("tokens.json"))
    }
}
```

Replace `validate_token` (currently lines 204–208):

```rust
pub async fn validate_token(&self, token: &str) -> bool {
    self.inner.tokens.validate_access(token).await
}
```

Delete the old `mint_token` method (currently lines 210–219). Token issuance now goes through `TokenStore::mint_pair`.

Inside the test helper `test_state()` (around line 642), update to:

```rust
fn test_state() -> OAuthState {
    let dir = tempdir();
    OAuthState::with_tokens_path(
        OAuthConfig {
            client_id: "test-id".into(),
            client_secret: "test-secret".into(),
            issuer: "https://example.test".into(),
            access_token_ttl_secs: None,
            refresh_token_ttl_secs: None,
        },
        dir.join("tokens.json"),
    )
    .unwrap()
}
```

The existing `tempdir()` helper in oauth.rs:632 returns a `PathBuf`, that's fine.

- [ ] **Step 2: Compile-check by running existing oauth tests**

```bash
cargo test --package zotero-mcp --lib oauth
```

Expected: compile errors at every old call site of `mint_token`. Find and update the callers in `handle_client_credentials` (around line 352) and `handle_authorization_code` (around line 410):

In `handle_client_credentials`, replace:
```rust
let (token, ttl) = state.mint_token().await;
```
with:
```rust
let pair = match state.inner.tokens.mint_pair(None).await {
    Ok(p) => p,
    Err(e) => {
        tracing::error!(error = %e, "mint_pair failed for client_credentials");
        return invalid_client();
    }
};
let token = pair.access_token;
let ttl = pair.access_ttl.as_secs();
```

In `handle_authorization_code`, replace the same `(token, ttl) = state.mint_token().await` line with the same block — but Task 10 will replace it again to also return the refresh token, so that's fine.

- [ ] **Step 3: Run tests**

```bash
cargo test --package zotero-mcp --lib
```

Expected: all existing oauth + http_transport tests pass. The `expires_in:3600` assertion in `token_endpoint_issues_for_valid_credentials_via_body` (around line 674) will now fail — TTL default changed to 7 days. Update that assertion:

```rust
assert!(body.contains("\"expires_in\":604800"));
```

- [ ] **Step 4: Update `main.rs` construction site**

In `crates/zotero-mcp/src/main.rs` (line 68), replace:

```rust
let oauth_state = oauth::OAuthConfig::load_or_generate(issuer_hint)?
    .map(oauth::OAuthState::new);
```

with:

```rust
let oauth_state = match oauth::OAuthConfig::load_or_generate(issuer_hint)? {
    Some(cfg) => Some(oauth::OAuthState::from_default_path(cfg)?),
    None => None,
};
```

- [ ] **Step 5: Run full test suite**

```bash
cargo test --package zotero-mcp
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/zotero-mcp/src/oauth.rs crates/zotero-mcp/src/main.rs
git commit -m "feat(oauth): wire TokenStore into OAuthState; tokens persist across restarts"
```

---

## Task 9: Advertise `refresh_token` grant in discovery doc

**Files:**
- Modify: `crates/zotero-mcp/src/oauth.rs` (`AuthorizationServerMetadata`, around line 257)

- [ ] **Step 1: Write failing test**

Find the existing test `discovery_documents_advertise_correct_endpoints` (around line 747). Add this assertion after the existing `client_credentials` check:

```rust
assert!(body.contains("\"refresh_token\""), "discovery must advertise refresh_token grant; body was: {body}");
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test --package zotero-mcp --lib oauth::tests::discovery_documents_advertise_correct_endpoints
```

Expected: assertion failure.

- [ ] **Step 3: Implement**

In `authorization_server_metadata` (around line 249), change `grant_types_supported`:

```rust
grant_types_supported: &["authorization_code", "refresh_token", "client_credentials"],
```

- [ ] **Step 4: Run test**

```bash
cargo test --package zotero-mcp --lib oauth::tests::discovery_documents_advertise_correct_endpoints
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/oauth.rs
git commit -m "feat(oauth): advertise refresh_token grant in discovery metadata"
```

---

## Task 10: `authorization_code` grant returns a refresh token

**Files:**
- Modify: `crates/zotero-mcp/src/oauth.rs` — `TokenResponse`, `handle_authorization_code`, `token_ok`

- [ ] **Step 1: Write failing tests**

Add to `mod tests` in `oauth.rs`:

```rust
#[tokio::test]
async fn auth_code_response_includes_refresh_token() {
    let state = test_state();
    let verifier = "the-verifier-of-reasonable-length";
    let challenge = challenge_for(verifier);
    let auth_uri = format!(
        "/authorize?response_type=code&client_id=test-id&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback&code_challenge={challenge}&code_challenge_method=S256&state=s",
    );
    let resp = router(state.clone())
        .oneshot(Request::builder().uri(auth_uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let location = resp.headers().get(header::LOCATION).unwrap().to_str().unwrap().to_string();
    let code = location.split_once("code=").and_then(|(_, r)| r.split('&').next()).unwrap().to_string();

    let body = format!(
        "grant_type=authorization_code&code={code}&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback&code_verifier={verifier}&client_id=test-id"
    );
    let resp = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/oauth/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    assert!(body.contains("\"access_token\""), "body was: {body}");
    assert!(body.contains("\"refresh_token\""), "body was: {body}");
    assert!(body.contains("\"refresh_expires_in\":7776000"), "body was: {body}");
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test --package zotero-mcp --lib oauth::tests::auth_code_response_includes_refresh_token
```

Expected: assertion failure (no `refresh_token` field in response).

- [ ] **Step 3: Update `TokenResponse` and `token_ok`**

Replace `TokenResponse` (currently lines 300–306):

```rust
#[derive(Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_expires_in: Option<u64>,
    scope: &'static str,
}
```

Replace `token_ok` (currently lines 419–430) with two helpers:

```rust
fn token_ok_access_only(token: String, ttl: u64) -> axum::response::Response {
    (
        StatusCode::OK,
        Json(TokenResponse {
            access_token: token,
            token_type: "Bearer",
            expires_in: ttl,
            refresh_token: None,
            refresh_expires_in: None,
            scope: "mcp",
        }),
    )
        .into_response()
}

fn token_ok_pair(pair: MintedPair) -> axum::response::Response {
    (
        StatusCode::OK,
        Json(TokenResponse {
            access_token: pair.access_token,
            token_type: "Bearer",
            expires_in: pair.access_ttl.as_secs(),
            refresh_token: Some(pair.refresh_token),
            refresh_expires_in: Some(pair.refresh_ttl.as_secs()),
            scope: "mcp",
        }),
    )
        .into_response()
}
```

In `handle_client_credentials` (around line 352), replace the new `token_ok(token, ttl)` call (added in Task 8) with `token_ok_access_only(token, ttl)`. Refresh tokens are not appropriate for the client_credentials grant per RFC 6749 §4.4.3.

In `handle_authorization_code` (around line 410), replace the entire block from `let pair = match state.inner.tokens.mint_pair(None).await` (the Task-8 addition) through the existing `token_ok(token, ttl)` line with:

```rust
let pair = match state.inner.tokens.mint_pair(None).await {
    Ok(p) => p,
    Err(e) => {
        tracing::error!(error = %e, "mint_pair failed for authorization_code");
        return invalid_grant("internal token store error");
    }
};
tracing::info!(
    grant = "authorization_code",
    chain_id = %pair.chain_id,
    expires_in = pair.access_ttl.as_secs(),
    "OAuth token pair minted"
);
token_ok_pair(pair)
```

- [ ] **Step 4: Run all oauth tests**

```bash
cargo test --package zotero-mcp --lib oauth
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/zotero-mcp/src/oauth.rs
git commit -m "feat(oauth): authorization_code grant returns access+refresh pair"
```

---

## Task 11: `refresh_token` grant handler

**Files:**
- Modify: `crates/zotero-mcp/src/oauth.rs` — `TokenRequest`, `token_handler`, new `handle_refresh_token`

- [ ] **Step 1: Write failing tests**

Add to `mod tests`:

```rust
async fn auth_code_full_flow(state: OAuthState) -> (String, String) {
    let verifier = "verifier-string-of-decent-length";
    let challenge = challenge_for(verifier);
    let auth_uri = format!(
        "/authorize?response_type=code&client_id=test-id&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback&code_challenge={challenge}&code_challenge_method=S256&state=s",
    );
    let resp = router(state.clone())
        .oneshot(Request::builder().uri(auth_uri).body(Body::empty()).unwrap())
        .await.unwrap();
    let location = resp.headers().get(header::LOCATION).unwrap().to_str().unwrap().to_string();
    let code = location.split_once("code=").and_then(|(_, r)| r.split('&').next()).unwrap().to_string();
    let body = format!(
        "grant_type=authorization_code&code={code}&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback&code_verifier={verifier}&client_id=test-id"
    );
    let resp = router(state).oneshot(
        Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body)).unwrap(),
    ).await.unwrap();
    let body_str = body_string(resp).await;
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    let access = parsed["access_token"].as_str().unwrap().to_string();
    let refresh = parsed["refresh_token"].as_str().unwrap().to_string();
    (access, refresh)
}

async fn post_token(state: OAuthState, body: &str) -> axum::response::Response {
    router(state).oneshot(
        Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body.to_string())).unwrap(),
    ).await.unwrap()
}

#[tokio::test]
async fn refresh_token_grant_returns_new_access_and_refresh() {
    let state = test_state();
    let (orig_access, refresh) = auth_code_full_flow(state.clone()).await;
    let resp = post_token(
        state.clone(),
        &format!("grant_type=refresh_token&refresh_token={refresh}&client_id=test-id"),
    ).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    let new_access = parsed["access_token"].as_str().unwrap();
    let new_refresh = parsed["refresh_token"].as_str().unwrap();
    assert_ne!(new_access, orig_access);
    assert_ne!(new_refresh, refresh);
    assert!(state.validate_token(new_access).await);
}

#[tokio::test]
async fn refresh_token_grant_invalidates_old_refresh_token() {
    let state = test_state();
    let (_, refresh) = auth_code_full_flow(state.clone()).await;
    // First use OK
    let r1 = post_token(state.clone(),
        &format!("grant_type=refresh_token&refresh_token={refresh}&client_id=test-id")).await;
    assert_eq!(r1.status(), StatusCode::OK);
    // Second use of same refresh token must fail
    let r2 = post_token(state,
        &format!("grant_type=refresh_token&refresh_token={refresh}&client_id=test-id")).await;
    assert_eq!(r2.status(), StatusCode::BAD_REQUEST);
    let body = body_string(r2).await;
    assert!(body.contains("invalid_grant"));
}

#[tokio::test]
async fn refresh_token_replay_revokes_chain() {
    let state = test_state();
    let (orig_access, refresh) = auth_code_full_flow(state.clone()).await;
    let r1 = post_token(state.clone(),
        &format!("grant_type=refresh_token&refresh_token={refresh}&client_id=test-id")).await;
    let body = body_string(r1).await;
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    let new_access = parsed["access_token"].as_str().unwrap().to_string();
    // Replay original refresh — must revoke chain.
    let _ = post_token(state.clone(),
        &format!("grant_type=refresh_token&refresh_token={refresh}&client_id=test-id")).await;
    // Both the new access AND the original access must now be invalid.
    assert!(!state.validate_token(&new_access).await, "new access should be revoked after replay");
    assert!(!state.validate_token(&orig_access).await, "original access should be revoked after replay");
}

#[tokio::test]
async fn refresh_token_grant_with_unknown_token_returns_invalid_grant() {
    let state = test_state();
    let resp = post_token(state,
        "grant_type=refresh_token&refresh_token=never-issued&client_id=test-id").await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_string(resp).await;
    assert!(body.contains("invalid_grant"));
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test --package zotero-mcp --lib oauth::tests::refresh_token
```

Expected: compilation/runtime errors — `unsupported_grant_type` returned for `refresh_token`.

- [ ] **Step 3: Add `refresh_token` field to `TokenRequest`**

Edit the `TokenRequest` struct (currently lines 277–298) — add this field anywhere in the struct:

```rust
#[serde(default)]
refresh_token: Option<String>,
```

- [ ] **Step 4: Add `refresh_token` arm to `token_handler`**

In `token_handler` (currently lines 315–334), add a new arm before the `_` catchall:

```rust
"refresh_token" => handle_refresh_token(state, headers, body).await,
```

- [ ] **Step 5: Implement `handle_refresh_token`**

Add this function alongside the other handlers (e.g. just below `handle_authorization_code`):

```rust
async fn handle_refresh_token(
    state: OAuthState,
    headers: HeaderMap,
    body: TokenRequest,
) -> axum::response::Response {
    // Optional client authentication — same logic as handle_authorization_code.
    if let Some((client_id, client_secret)) = resolve_client_credentials(&headers, &body) {
        let expected = &state.inner.config;
        if !constant_time_eq(client_id.as_bytes(), expected.client_id.as_bytes())
            || !constant_time_eq(client_secret.as_bytes(), expected.client_secret.as_bytes())
        {
            return invalid_client();
        }
    } else if let Some(client_id) = body.client_id.as_deref() {
        if !constant_time_eq(client_id.as_bytes(), state.inner.config.client_id.as_bytes()) {
            return invalid_client();
        }
    }

    let Some(presented) = body.refresh_token.as_deref() else {
        return invalid_grant("missing refresh_token");
    };

    let chain_id = match state.inner.tokens.consume_refresh(presented).await {
        Ok(chain) => chain,
        Err(RefreshError::Replayed(chain)) => {
            tracing::warn!(chain_id = %chain, "refresh-token replay detected; revoking chain");
            state.inner.tokens.revoke_chain(chain).await;
            return invalid_grant("refresh token replay");
        }
        Err(RefreshError::Expired) => return invalid_grant("refresh token expired"),
        Err(RefreshError::Unknown) => return invalid_grant("unknown refresh token"),
    };

    let pair = match state.inner.tokens.mint_pair(Some(chain_id.clone())).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "mint_pair failed during refresh_token grant");
            return invalid_grant("internal token store error");
        }
    };
    tracing::info!(
        grant = "refresh_token",
        chain_id = %chain_id,
        expires_in = pair.access_ttl.as_secs(),
        "OAuth token pair minted (refreshed)"
    );
    token_ok_pair(pair)
}
```

- [ ] **Step 6: Run all oauth tests**

```bash
cargo test --package zotero-mcp --lib oauth
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/zotero-mcp/src/oauth.rs
git commit -m "feat(oauth): refresh_token grant with rotation + replay detection"
```

---

## Task 12: Cross-module integration test for restart-survival

**Files:**
- Modify: `crates/zotero-mcp/src/http_transport.rs` — add the regression test that proves the launchd-restart bug is fixed.

- [ ] **Step 1: Write the test**

Append to `mod tests` in `http_transport.rs`:

```rust
#[tokio::test]
async fn tokens_survive_oauth_state_recreation() {
    use crate::oauth::{OAuthConfig, OAuthState};
    let dir = tempfile::TempDir::new().unwrap();
    let tokens_path = dir.path().join("tokens.json");
    let config = OAuthConfig {
        client_id: "test-id".into(),
        client_secret: "test-secret".into(),
        issuer: "https://example.test".into(),
        access_token_ttl_secs: None,
        refresh_token_ttl_secs: None,
    };

    // Simulate the HTTP server's first lifetime: mint a token via the public
    // store API, then drop everything as if launchd killed the process.
    let access_token = {
        let state_a = OAuthState::with_tokens_path(config.clone(), tokens_path.clone()).unwrap();
        let pair = state_a.inner_for_test_mint_pair().await;
        assert!(state_a.validate_token(&pair.access_token).await);
        pair.access_token
    };

    // Simulate launchd restart: brand-new OAuthState reading the same file.
    let state_b = OAuthState::with_tokens_path(config, tokens_path).unwrap();

    // The original access token MUST still validate. This is the regression
    // test for the in-memory-only token bug we shipped before.
    assert!(
        state_b.validate_token(&access_token).await,
        "access token issued before restart must still validate after restart"
    );
}
```

For the test to compile, expose a small test-only mint helper on `OAuthState` so the test doesn't need to drive the full HTTP flow. In `oauth.rs`, add at the bottom of `impl OAuthState` (gated by `#[cfg(test)]`):

```rust
#[cfg(test)]
impl OAuthState {
    pub async fn inner_for_test_mint_pair(&self) -> crate::oauth::MintedPair {
        self.inner.tokens.mint_pair(None).await.unwrap()
    }
}
```

- [ ] **Step 2: Run the test**

```bash
cargo test --package zotero-mcp --lib http_transport::tests::tokens_survive
```

Expected: pass.

- [ ] **Step 3: Run the entire suite to check nothing broke**

```bash
cargo test --package zotero-mcp
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/zotero-mcp/src/http_transport.rs crates/zotero-mcp/src/oauth.rs
git commit -m "test(oauth): regression test — tokens survive OAuthState recreation"
```

---

## Task 13: Documentation updates

**Files:**
- Modify: `README.md` — OAuth configuration subsection
- Modify: `docs/CLAUDE_COWORK_SETUP.md` — note the new behavior

- [ ] **Step 1: Read the current README OAuth section**

```bash
grep -n -A 60 "OAuth configuration" README.md | head -100
```

- [ ] **Step 2: Update the README OAuth section**

Find the existing "OAuth configuration" subsection. Add a new paragraph after the existing description of `oauth.toml`:

```markdown
### Token durability

Access and refresh tokens are persisted to `<config_dir>/tokens.json` (mode 0600,
hashed at rest with SHA-256). This means OAuth sessions survive `launchd`
restarts, system sleep, log out/in, and `zotero-mcp setup` re-bootstrap — the
connector keeps working without re-authenticating in the browser.

Default TTLs:

| Token | Default TTL | Override field in `oauth.toml` |
|---|---|---|
| Access token | 7 days | `access_token_ttl_secs` |
| Refresh token | 90 days | `refresh_token_ttl_secs` |

The 7-day access TTL is a workaround for the [open Anthropic bug](https://github.com/anthropics/claude-ai-mcp/issues/228) where
`mcp-proxy.anthropic.com` ignores refresh tokens. Once Anthropic ships their proxy fix, you can lower this back to 1 hour:

```toml
access_token_ttl_secs = 3600
```

Refresh tokens follow OAuth 2.1 §4.3.1: one-time-use with rotation. If a refresh
token is replayed (a leak signal), the entire token chain is revoked and you're
forced through one fresh browser auth.

To revoke all tokens manually:

```bash
rm "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/tokens.json"
launchctl bootout gui/$UID/com.zotero-mcp.http
launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
```
```

- [ ] **Step 3: Update CLAUDE_COWORK_SETUP.md** if it mentions hourly re-auth

```bash
grep -n "expir\|re-auth\|hour" docs/CLAUDE_COWORK_SETUP.md
```

If any line implies hourly re-auth, replace it with the new 7-day-default reality. If the file doesn't mention it, no change needed.

- [ ] **Step 4: Commit**

```bash
git add README.md docs/CLAUDE_COWORK_SETUP.md
git commit -m "docs(oauth): document token persistence + configurable TTLs"
```

---

## Task 14: Manual end-to-end verification (not automated)

This task is a checklist for the implementer to run after the code lands. No commits.

- [ ] **Step 1: Install the new binary**

```bash
cargo install --path crates/zotero-mcp --force
```

- [ ] **Step 2: Force a clean restart**

```bash
launchctl bootout gui/$UID/com.zotero-mcp.http 2>/dev/null
launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
```

Wait 5 seconds for the server to come up.

- [ ] **Step 3: Verify the persistence file exists with correct perms**

```bash
ls -l "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/tokens.json" 2>&1
```

Expected, if Cowork has been used at least once since install: file exists, mode `-rw-------`. If it doesn't exist yet, that's fine — it's only created after the first successful OAuth flow.

- [ ] **Step 4: Trigger one re-auth via Cowork** (browser flow), then watch it work without re-auth

Open Cowork. Click reconnect on the Zotero connector. Complete the browser auth flow. Use a Zotero tool. **Now confirm:**

```bash
ls -l "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/tokens.json"
cat "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/tokens.json" | head -30
```

Expected: file exists, mode 0600, content is JSON with `token_hash` (hex) entries — **never raw bearer values**.

- [ ] **Step 5: Bounce the daemon mid-conversation**

```bash
launchctl bootout gui/$UID/com.zotero-mcp.http
launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
```

Then, in the same Cowork conversation, immediately call a Zotero tool. **Expected:** tool call succeeds without re-auth. (Before this fix, this would 401 because tokens were lost when the process died.)

- [ ] **Step 6: Use the connector for a full working day**

Goal: confirm no browser re-auth is required. The 7-day TTL means even at the worst-case (server restart 5 minutes after first use), you get 7 days.

- [ ] **Step 7: (Optional) Test the spec-correct refresh path against Claude Code direct**

Lower the access TTL to confirm refresh tokens work:

```bash
# Edit ~/Library/Application Support/dev.zotero-mcp.zotero-mcp/oauth.toml
# Add: access_token_ttl_secs = 90
launchctl bootout gui/$UID/com.zotero-mcp.http
launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
```

Connect Claude Code CLI directly to the connector (not via Cowork — Cowork goes through the broken proxy). Use a tool. Wait 100 seconds. Use another tool. Tail the server log:

```bash
tail -f ~/Library/Logs/zotero-mcp/http.out.log
```

Expected: a line like `OAuth token pair minted (refreshed) chain_id=…` — proves Claude Code sent `grant_type=refresh_token` and our server honored it.

Restore the TTL when done:

```bash
# Remove or change access_token_ttl_secs back to your preference
launchctl bootout gui/$UID/com.zotero-mcp.http
launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
```

---

## Spec coverage check

| Spec section | Tasks |
|---|---|
| Decision 1 (hybrid: long access + refresh + persistence) | 4, 8, 10, 11 |
| Decision 2 (JSON file, mode 0600) | 2, 4 |
| Decision 3 (7-day access TTL default) | 1 |
| Decision 4 (90-day refresh TTL default) | 1 |
| Decision 5 (refresh rotation) | 6, 11 |
| Decision 6 (hashed at rest) | 4 |
| Decision 7 (token_store.rs split) | 2 |
| Path 1 (auth_code returns refresh) | 10 |
| Path 2 (refresh_token grant + replay) | 11 |
| Path 3 (validate_access via store) | 5, 8 |
| Path 4 (cold-start file load with wipe + prune) | 2, 3, 8 |
| F1 (missing tokens.json → empty) | 2 |
| F2 (corrupt → rename aside, start fresh) | 2 |
| F3 (persist failure → keep in-memory) | 4 (silently logged) |
| F4 (replay → revoke chain) | 7, 11 |
| F5 (concurrent refresh false positive) | accepted, no test |
| Bounded growth (self-pruning) | 2 (load), 4 (mint) |
| Tokens at rest hashed | 4 |
| Refresh-token rotation as leak detection | 6, 7, 11 |
| Constant-time comparison | not needed: SHA-256 hash equality is on a fixed-length digest, and `HashMap::get` lookup of a hex string is not a token-disclosure side channel because the comparison happens against a *hash of* the secret, not the secret itself |
| Logging hygiene (no raw tokens) | enforced by code in tasks 4, 6, 11 — tokens never logged, only chain_ids |
| Atomic file write | 4 |
| Migration path | 2 (load handles missing file), 13 (docs explain) |

---

## Final notes

- **No new dependencies.** Chain IDs use the same `format!("{:032x}", rand::random::<u128>())` pattern as existing token generation.
- **Rust 2018-style submodule.** `oauth.rs` keeps its name; `oauth/token_store.rs` is declared via `mod token_store;` at the top of `oauth.rs`. No file rename.
- **`client_credentials` grant retained** for headless scripting and tests. It returns access-only (no refresh), per RFC 6749 §4.4.3.
- **`OAuthState::with_tokens_path` is the test entry point**, `OAuthState::from_default_path` is the production entry point. Production code in `main.rs` uses the latter.
- **The integration test in Task 12 is the load-bearing proof** that the launchd-restart bug is fixed. Other tests verify behavior; this one verifies the user-visible scenario.
