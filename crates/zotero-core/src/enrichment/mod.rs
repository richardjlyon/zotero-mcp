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

/// Split a full name into (first, last) by treating the last whitespace-separated
/// token as the surname. Used by arXiv and Semantic Scholar clients.
pub(crate) fn openlibrary_like_split(full: &str) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = full.trim().rsplitn(2, ' ').collect();
    match parts.as_slice() {
        [last, first] => (Some((*first).to_string()), Some((*last).to_string())),
        [single] => (None, Some((*single).to_string())),
        _ => (None, None),
    }
}
