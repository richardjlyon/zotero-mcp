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
}
