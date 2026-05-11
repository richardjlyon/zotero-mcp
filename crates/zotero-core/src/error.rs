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

    #[error("external lookup failed for source {}: {}", .0, .1)]
    Lookup(String, String),

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
