use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Item {
    pub key: String,
    pub library_id: i64,
    pub version: i64,
    pub item_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation_key: Option<String>,
    pub fields: Value,
    pub creators: Vec<Creator>,
    pub tags: Vec<String>,
    pub collection_keys: Vec<String>,
    pub date_added: String,
    pub date_modified: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_content_tool: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Creator {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    pub creator_type: String,
    pub order_index: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Attachment {
    pub key: String,
    pub parent_key: Option<String>,
    pub content_type: Option<String>,
    pub filename: Option<String>,
    pub absolute_path: Option<String>,
    pub link_mode: AttachmentLinkMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentLinkMode {
    ImportedFile,
    ImportedUrl,
    LinkedFile,
    LinkedUrl,
    EmbeddedImage,
    Unknown,
}

impl AttachmentLinkMode {
    pub fn from_i64(n: i64) -> Self {
        // Zotero link_mode constants:
        //   0 = imported_file, 1 = imported_url, 2 = linked_file,
        //   3 = linked_url, 4 = embedded_image
        match n {
            0 => Self::ImportedFile,
            1 => Self::ImportedUrl,
            2 => Self::LinkedFile,
            3 => Self::LinkedUrl,
            4 => Self::EmbeddedImage,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Collection {
    pub key: String,
    pub library_id: i64,
    pub name: String,
    pub parent_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Tag {
    pub name: String,
    pub item_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Annotation {
    pub key: String,
    pub parent_attachment_key: String,
    pub kind: String,
    pub text: Option<String>,
    pub comment: Option<String>,
    pub color: Option<String>,
    pub page_label: Option<String>,
    pub sort_index: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchHit {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation_key: Option<String>,
    pub item_type: String,
    pub title: Option<String>,
    pub creators_short: Option<String>,
    pub year: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_excerpt: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Diff {
    pub changes: Vec<FieldChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FieldChange {
    pub field: String,
    pub current: Option<Value>,
    pub proposed: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EnrichmentProposal {
    pub item_key: String,
    pub diff: Diff,
    pub confidence: f64,
    pub source_breakdown: Vec<SourceBreakdown>,
    pub needs_review: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SourceBreakdown {
    pub source: String,
    pub matched: bool,
    pub fields_contributed: Vec<String>,
    pub raw_response_cached: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_serialises_round_trip() {
        let item = Item {
            key: "JGF2UTMW".into(),
            library_id: 1,
            version: 10005,
            item_type: "book".into(),
            citation_key: Some("rabkinWhatModernIsrael2016".into()),
            fields: serde_json::json!({
                "title": "What is Modern Israel?",
                "date": "2016",
                "publisher": "Pluto Press"
            }),
            creators: vec![Creator {
                first_name: Some("Yakob".into()),
                last_name: Some("Rabkin".into()),
                creator_type: "author".into(),
                order_index: 0,
            }],
            tags: vec![],
            collection_keys: vec!["LU3TXR2S".into()],
            date_added: "2026-05-11T06:28:35Z".into(),
            date_modified: "2026-05-11T06:29:38Z".into(),
            parent_key: None,
            recommended_content_tool: Some("get_pdf_text".into()),
        };
        let s = serde_json::to_string(&item).unwrap();
        let back: Item = serde_json::from_str(&s).unwrap();
        assert_eq!(back.key, "JGF2UTMW");
        assert_eq!(
            back.citation_key.as_deref(),
            Some("rabkinWhatModernIsrael2016")
        );
    }

    #[test]
    fn diff_default_has_no_changes() {
        let d = Diff::default();
        assert!(d.changes.is_empty());
    }
}
