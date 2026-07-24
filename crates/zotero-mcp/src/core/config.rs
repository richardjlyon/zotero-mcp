use crate::core::error::{Error, Result};
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
    /// Base URL of the Zotero Web API used for writes. Zotero's local HTTP
    /// server is read-only (returns 501 on PATCH/POST), so writes must go
    /// through the cloud API and propagate back via sync.
    pub web_api_base: String,
    /// User Zotero API key from <https://www.zotero.org/settings/keys>.
    /// Required for any write operation. Reads do not need it. Sensitive;
    /// treat like a password.
    pub api_key: Option<String>,
    pub user_id: i64,
    pub include_group_libraries: bool,
    pub min_schema_userdata: i64,
    pub max_schema_userdata: i64,

    /// Optional explicit path to the `pdftotext` binary. When set and the file
    /// exists, used instead of PATH lookup. Useful for non-standard installs.
    #[serde(default)]
    pub pdftotext_path: Option<String>,

    /// Whether to fall back to `pdftotext` (Poppler) when the in-process
    /// `pdf-extract` engine fails. Default: true.
    #[serde(default = "default_true")]
    pub pdftotext_fallback: bool,

    /// Storage model for attachments created via `attach_file`. Default
    /// mirrors Zotero's own default behaviour. Set to `"linked_file"` for
    /// BYO-storage users (Resilio Sync, Syncthing, NAS-backed Zotero data dirs).
    #[serde(default = "default_attachment_mode")]
    pub attachment_mode: String,

    /// Required when `attachment_mode = "linked_file"`. Absolute path to the
    /// Zotero "Linked Attachment Base Directory" (Zotero Preferences →
    /// Advanced → Files & Folders). Files attached via `attach_file` must
    /// live inside this directory.
    #[serde(default)]
    pub linked_attachment_base_dir: Option<String>,

    /// Per-file size ceiling for `attach_file`. Anything larger is rejected
    /// pre-flight. Default: 50 MB.
    #[serde(default = "default_max_attachment_bytes")]
    pub max_attachment_bytes: usize,

    /// Prefix rewrites applied to incoming `file_path` arguments before the
    /// file is read. Lets a remote MCP client that shares a synced folder
    /// with this machine pass its own absolute paths (e.g. a VM whose
    /// `/vault/brain` is this machine's `~/Resilio/second-brain`). The
    /// longest matching key prefix is replaced by its value; values may use
    /// `~`. Paths matching no key pass through unchanged.
    #[serde(default)]
    pub path_map: std::collections::BTreeMap<String, String>,
}

fn default_true() -> bool {
    true
}

fn default_attachment_mode() -> String {
    "imported_file".into()
}

fn default_max_attachment_bytes() -> usize {
    50 * 1024 * 1024
}

impl Default for ZoteroConfig {
    fn default() -> Self {
        Self {
            data_dir: "~/Zotero".into(),
            local_api_base: "http://localhost:23119".into(),
            web_api_base: "https://api.zotero.org".into(),
            api_key: None,
            user_id: 0,
            include_group_libraries: true,
            min_schema_userdata: 120,
            max_schema_userdata: 135,
            pdftotext_path: None,
            pdftotext_fallback: true,
            attachment_mode: "imported_file".into(),
            linked_attachment_base_dir: None,
            max_attachment_bytes: 50 * 1024 * 1024,
            path_map: std::collections::BTreeMap::new(),
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
        Self {
            level: "info".into(),
        }
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

/// Rewrite `path` through a `path_map`: the longest key that is a prefix of
/// `path` is replaced by its (tilde-expanded) value. Paths matching no key
/// are returned tilde-expanded but otherwise unchanged.
pub fn remap_path(map: &std::collections::BTreeMap<String, String>, path: &str) -> String {
    let mut best: Option<(&str, &str)> = None;
    for (k, v) in map {
        if path.starts_with(k.as_str()) && best.is_none_or(|(bk, _)| k.len() > bk.len()) {
            best = Some((k.as_str(), v.as_str()));
        }
    }
    match best {
        Some((k, v)) => format!("{}{}", expand_tilde(v), &path[k.len()..]),
        None => expand_tilde(path),
    }
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
    directories::UserDirs::new().map(|u| u.home_dir().to_string_lossy().into_owned())
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Config(s.into())
    }
}

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
        assert_eq!(
            c.zotero.pdftotext_path.as_deref(),
            Some("/opt/homebrew/bin/pdftotext")
        );
        assert!(!c.zotero.pdftotext_fallback);
    }

    #[test]
    fn attachment_mode_defaults_to_imported_file() {
        let c = Config::default();
        assert_eq!(c.zotero.attachment_mode, "imported_file");
        assert!(c.zotero.linked_attachment_base_dir.is_none());
        assert_eq!(c.zotero.max_attachment_bytes, 50 * 1024 * 1024);
    }

    #[test]
    fn attachment_mode_parses_from_toml() {
        let toml = r#"
[zotero]
attachment_mode = "linked_file"
linked_attachment_base_dir = "/Users/rjl/Resilio/Zotero-Attachments"
max_attachment_bytes = 104857600
"#;
        let c: Config = toml::from_str(toml).unwrap();
        assert_eq!(c.zotero.attachment_mode, "linked_file");
        assert_eq!(
            c.zotero.linked_attachment_base_dir.as_deref(),
            Some("/Users/rjl/Resilio/Zotero-Attachments")
        );
        assert_eq!(c.zotero.max_attachment_bytes, 104857600);
    }

    #[test]
    fn path_map_parses_and_remaps_longest_prefix() {
        let toml = r#"
[zotero.path_map]
"/vault/" = "/local/vault/"
"/vault/brain/" = "/local/brain/"
"#;
        let c: Config = toml::from_str(toml).unwrap();
        // longest matching prefix wins
        assert_eq!(
            remap_path(&c.zotero.path_map, "/vault/brain/_inbox/a.pdf"),
            "/local/brain/_inbox/a.pdf"
        );
        assert_eq!(
            remap_path(&c.zotero.path_map, "/vault/code/x.pdf"),
            "/local/vault/code/x.pdf"
        );
        // no match → unchanged
        assert_eq!(
            remap_path(&c.zotero.path_map, "/tmp/other.pdf"),
            "/tmp/other.pdf"
        );
        // empty map → unchanged
        let empty = std::collections::BTreeMap::new();
        assert_eq!(remap_path(&empty, "/tmp/other.pdf"), "/tmp/other.pdf");
    }
}
