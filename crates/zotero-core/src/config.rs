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
