// Date format audit (2026-05-13): all three sources emit ISO 8601 dates.
// - openlibrary: normalised via parse_date (handles freeform publish_date).
// - crossref: extract_date pads {YYYY, MM, DD} parts to 2-digit width.
// - arxiv: published timestamps split at 'T' (arXiv always sends ISO 8601).
pub mod arxiv;
pub mod crossref;
pub mod openlibrary;
pub mod pdf_signals;
pub mod propose;
pub mod scoring;
pub mod semantic_scholar;

use crate::core::types::Creator;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Result from any enrichment source, already mapped to Zotero's schema vocabulary.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

/// Flatten a `NormalizedRecord` into a Zotero-shaped item JSON suitable for
/// `core::writer::items::create_item`.
///
/// Reads `itemType` from `record.fields` (every source populates it).
/// Rewrites creators inline using Zotero's wire vocabulary
/// (`creatorType` / `firstName` / `lastName`) — the internal `Creator` struct
/// keeps snake_case names because it is the canonical type used by readers,
/// scoring, and diffing. The wire-shape rename lives only here.
///
/// Stashes provenance (`source`, `source_url`) into Zotero's `extra` field
/// as newline-separated `key: value` lines, appending to any pre-existing
/// `extra` content.
pub fn normalized_to_item(record: &NormalizedRecord) -> Value {
    let mut obj = record.fields.clone();

    if !record.creators.is_empty() {
        let creators: Vec<Value> = record
            .creators
            .iter()
            .map(|c| {
                let mut m = serde_json::Map::new();
                m.insert("creatorType".into(), Value::String(c.creator_type.clone()));
                if let Some(ref first) = c.first_name {
                    m.insert("firstName".into(), Value::String(first.clone()));
                }
                if let Some(ref last) = c.last_name {
                    m.insert("lastName".into(), Value::String(last.clone()));
                }
                Value::Object(m)
            })
            .collect();
        obj.insert("creators".into(), Value::Array(creators));
    }

    let mut extra_lines: Vec<String> = Vec::new();
    if let Some(existing) = obj.get("extra").and_then(|v| v.as_str()) {
        if !existing.is_empty() {
            extra_lines.push(existing.to_string());
        }
    }
    extra_lines.push(format!("source: {}", record.source));
    if let Some(ref url) = record.source_url {
        extra_lines.push(format!("sourceURL: {}", url));
    }
    obj.insert("extra".into(), Value::String(extra_lines.join("\n")));

    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::Creator;
    use serde_json::{Map, Value};

    fn record_with(
        source: &str,
        item_type: &str,
        title: &str,
        source_url: Option<&str>,
    ) -> NormalizedRecord {
        let mut fields = Map::new();
        fields.insert("itemType".into(), Value::String(item_type.into()));
        fields.insert("title".into(), Value::String(title.into()));
        NormalizedRecord {
            source: source.into(),
            fields,
            creators: vec![Creator {
                first_name: Some("Jane".into()),
                last_name: Some("Doe".into()),
                creator_type: "author".into(),
                order_index: 0,
            }],
            source_url: source_url.map(String::from),
        }
    }

    #[test]
    fn flat_output_is_object_with_item_type_from_fields() {
        let r = record_with("openlibrary", "book", "Some Book", None);
        let v = normalized_to_item(&r);
        let obj = v.as_object().expect("top-level object");
        assert_eq!(obj["itemType"], "book");
        assert_eq!(obj["title"], "Some Book");
        assert!(!obj.contains_key("source"));
        assert!(!obj.contains_key("source_url"));
        assert!(!obj.contains_key("fields"));
    }

    #[test]
    fn creators_use_zotero_camel_case() {
        let r = record_with("openlibrary", "book", "x", None);
        let v = normalized_to_item(&r);
        let creators = v["creators"].as_array().expect("creators array");
        let c0 = creators[0].as_object().expect("creator object");
        assert_eq!(c0["creatorType"], "author");
        assert_eq!(c0["firstName"], "Jane");
        assert_eq!(c0["lastName"], "Doe");
        assert!(!c0.contains_key("creator_type"));
        assert!(!c0.contains_key("first_name"));
        assert!(!c0.contains_key("last_name"));
        assert!(!c0.contains_key("orderIndex"));
        assert!(!c0.contains_key("order_index"));
    }

    #[test]
    fn extra_field_stashes_source_and_source_url() {
        let r = record_with(
            "openlibrary",
            "book",
            "x",
            Some("https://openlibrary.org/isbn/9780000000000"),
        );
        let v = normalized_to_item(&r);
        let extra = v["extra"].as_str().expect("extra string");
        assert!(extra.contains("source: openlibrary"), "got: {extra:?}");
        assert!(
            extra.contains("sourceURL: https://openlibrary.org/isbn/9780000000000"),
            "got: {extra:?}"
        );
    }

    #[test]
    fn extra_omits_source_url_line_when_none() {
        let r = record_with("arxiv", "preprint", "x", None);
        let v = normalized_to_item(&r);
        let extra = v["extra"].as_str().expect("extra string");
        assert!(extra.contains("source: arxiv"));
        assert!(!extra.contains("sourceURL"), "got: {extra:?}");
    }

    #[test]
    fn extra_appends_to_existing_extra_field() {
        let mut r = record_with(
            "crossref",
            "journalArticle",
            "x",
            Some("https://doi.org/10.1/x"),
        );
        r.fields.insert(
            "extra".into(),
            Value::String("Citation Key: foo2024".into()),
        );
        let v = normalized_to_item(&r);
        let extra = v["extra"].as_str().expect("extra string");
        assert!(extra.starts_with("Citation Key: foo2024"));
        assert!(extra.contains("source: crossref"));
    }

    #[test]
    fn creator_with_only_last_name_omits_first_name_key() {
        let mut r = record_with("openlibrary", "book", "x", None);
        r.creators[0].first_name = None;
        let v = normalized_to_item(&r);
        let c0 = &v["creators"][0];
        assert!(c0.get("firstName").is_none());
        assert_eq!(c0["lastName"], "Doe");
    }
}
