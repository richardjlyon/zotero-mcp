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

    #[error(
        "writes require a Zotero Web API key. Create one at \
         https://www.zotero.org/settings/keys (give it library:write) and set \
         `[zotero] api_key = \"...\"` in zotero-mcp's config.toml."
    )]
    WriteApiKeyMissing,

    #[error("better-bibtex JSON-RPC unavailable")]
    BbtUnavailable,

    #[error("better-bibtex error: {0}")]
    Bbt(String),

    #[error("pdf extraction failed: {0}")]
    Pdf(String),

    #[error("attachment file not found: {0}")]
    AttachmentFileNotFound(std::path::PathBuf),

    #[error(
        "attachment file {file_path} is not inside the configured \
         linked_attachment_base_dir ({base_dir}). Move it in first, or pass \
         mode = \"imported_file\" for this call.",
        file_path = file_path.display(),
        base_dir = base_dir.display(),
    )]
    AttachmentOutsideBaseDir {
        file_path: std::path::PathBuf,
        base_dir: std::path::PathBuf,
    },

    #[error("zotero file upload failed at {stage}: {detail}")]
    UploadFailed { stage: &'static str, detail: String },

    #[error(
        "attachment file {file_path} exceeds max_attachment_bytes ({limit})",
        file_path = file_path.display(),
    )]
    AttachmentTooLarge {
        file_path: std::path::PathBuf,
        limit: usize,
    },

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
    PdfAllEnginesFailed {
        pdf_extract: String,
        pdftotext: String,
    },

    #[error(
        "pdf extraction found no usable text in {path} ({detail}). The PDF is \
         likely image-only (scanned). Remedy: install ocrmypdf \
         (`brew install ocrmypdf` on macOS, `apt install ocrmypdf` on Linux) \
         and/or configure the Docling route (`DOCLING_URL` or `docling_url` \
         in config.toml) so the OCR pre-step can recover the text."
    )]
    PdfNothingExtractable { path: String, detail: String },

    #[error(
        "pdf {path} has {pages} pages, over the {threshold}-page whole-document \
         extraction limit. Extracting the whole document at once (OCR + layout \
         conversion) would exceed the time budget and the response size. Remedy: \
         request page windows instead — call the extraction tool with a page \
         range (e.g. from_page/to_page) and walk the {pages} pages a window at a \
         time; each result reports the total page count so you know when you are done."
    )]
    PdfDocumentTooLarge {
        path: String,
        pages: u32,
        threshold: u32,
    },

    #[error("html extraction failed: {0}")]
    Html(String),

    #[error("external lookup failed for source {source}: {message}")]
    Lookup { r#source: String, message: String },

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
        let e = Error::SchemaMismatch {
            expected: "125..130".into(),
            found: 99,
        };
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
    fn pdf_nothing_extractable_names_the_ocr_remedy() {
        let e = Error::PdfNothingExtractable {
            path: "/tmp/scan.pdf".into(),
            detail: "all routes yielded sub-floor text".into(),
        };
        let s = e.to_string();
        assert!(s.contains("/tmp/scan.pdf"));
        assert!(s.contains("sub-floor"));
        assert!(s.contains("ocrmypdf"));
        assert!(s.contains("DOCLING_URL"));
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

    use std::path::PathBuf;

    #[test]
    fn attachment_file_not_found_message_includes_path() {
        let e = Error::AttachmentFileNotFound(PathBuf::from("/tmp/missing.pdf"));
        let s = e.to_string();
        assert!(s.contains("/tmp/missing.pdf"));
    }

    #[test]
    fn attachment_outside_base_dir_message_includes_hint() {
        let e = Error::AttachmentOutsideBaseDir {
            file_path: PathBuf::from("/var/tmp/x.pdf"),
            base_dir: PathBuf::from("/Users/rjl/Resilio/Zotero-Attachments"),
        };
        let s = e.to_string();
        assert!(s.contains("/var/tmp/x.pdf"));
        assert!(s.contains("/Users/rjl/Resilio/Zotero-Attachments"));
        assert!(s.contains("imported_file"));
    }

    #[test]
    fn upload_failed_carries_stage_and_detail() {
        let e = Error::UploadFailed {
            stage: "s3_put",
            detail: "connection reset".into(),
        };
        let s = e.to_string();
        assert!(s.contains("s3_put"));
        assert!(s.contains("connection reset"));
    }

    #[test]
    fn attachment_too_large_includes_limit() {
        let e = Error::AttachmentTooLarge {
            file_path: PathBuf::from("/tmp/big.pdf"),
            limit: 50 * 1024 * 1024,
        };
        let s = e.to_string();
        assert!(s.contains("/tmp/big.pdf"));
        assert!(s.contains(&(50 * 1024 * 1024).to_string()));
    }
}
