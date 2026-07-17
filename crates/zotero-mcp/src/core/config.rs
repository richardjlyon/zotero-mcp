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

    /// Base URL of the Docling conversion service used as the layout-aware
    /// primary extraction route (e.g. "http://100.79.12.8:5001"). The
    /// `DOCLING_URL` environment variable takes precedence over this value.
    /// When neither is set, the Docling route is disabled and extraction
    /// uses the flat-text chain only.
    #[serde(default)]
    pub docling_url: Option<String>,

    /// Wall-clock timeout for a Docling convert request, in seconds.
    /// Default: 300 (formula enrichment on large PDFs is slow).
    #[serde(default = "default_docling_convert_timeout_secs")]
    pub docling_convert_timeout_secs: u64,

    /// Timeout for the Docling `/health` probe, in seconds. Default: 5.
    #[serde(default = "default_docling_health_timeout_secs")]
    pub docling_health_timeout_secs: u64,

    /// Optional explicit path to the `ocrmypdf` binary used by the OCR
    /// pre-step for image-only (scanned) PDFs. When set and the file
    /// exists, used instead of PATH lookup. When `ocrmypdf` cannot be
    /// resolved at all, the OCR pre-step is skipped and extraction
    /// degrades gracefully (recorded in the completeness report).
    #[serde(default)]
    pub ocrmypdf_path: Option<String>,

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
}

fn default_true() -> bool {
    true
}

fn default_docling_convert_timeout_secs() -> u64 {
    300
}

fn default_docling_health_timeout_secs() -> u64 {
    5
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
            docling_url: None,
            docling_convert_timeout_secs: 300,
            docling_health_timeout_secs: 5,
            ocrmypdf_path: None,
            attachment_mode: "imported_file".into(),
            linked_attachment_base_dir: None,
            max_attachment_bytes: 50 * 1024 * 1024,
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
    fn docling_defaults_to_disabled_with_timeouts() {
        let c = Config::default();
        assert!(c.zotero.docling_url.is_none());
        assert_eq!(c.zotero.docling_convert_timeout_secs, 300);
        assert_eq!(c.zotero.docling_health_timeout_secs, 5);
    }

    #[test]
    fn docling_config_parses_from_toml() {
        let toml = r#"
[zotero]
docling_url = "http://100.79.12.8:5001"
docling_convert_timeout_secs = 120
docling_health_timeout_secs = 3
"#;
        let c: Config = toml::from_str(toml).unwrap();
        assert_eq!(
            c.zotero.docling_url.as_deref(),
            Some("http://100.79.12.8:5001")
        );
        assert_eq!(c.zotero.docling_convert_timeout_secs, 120);
        assert_eq!(c.zotero.docling_health_timeout_secs, 3);
    }

    #[test]
    fn ocrmypdf_path_defaults_to_none() {
        let c = Config::default();
        assert!(c.zotero.ocrmypdf_path.is_none());
    }

    #[test]
    fn ocrmypdf_path_parses_from_toml() {
        let toml = r#"
[zotero]
ocrmypdf_path = "/Users/rjl/.local/bin/ocrmypdf"
"#;
        let c: Config = toml::from_str(toml).unwrap();
        assert_eq!(
            c.zotero.ocrmypdf_path.as_deref(),
            Some("/Users/rjl/.local/bin/ocrmypdf")
        );
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
}
