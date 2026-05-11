use crate::core::error::Result;
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
        if !p.exists() {
            return Ok(None);
        }
        let bytes = tokio::fs::read(&p).await?;
        let env: Envelope<serde_json::Value> = serde_json::from_slice(&bytes)?;
        let age = now_secs().saturating_sub(env.stored_at);
        if age >= self.ttl.as_secs() {
            return Ok(None);
        }
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
struct Envelope<T> {
    stored_at: u64,
    value: T,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn simple_hash(s: &str) -> String {
    // Stable, deterministic file naming via FNV-1a 64-bit. Avoid adding sha2.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}
