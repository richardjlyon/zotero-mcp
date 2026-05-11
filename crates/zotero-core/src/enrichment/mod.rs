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
