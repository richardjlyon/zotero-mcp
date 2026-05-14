use serde_json::Map;
use zotero_mcp::core::enrichment::pdf_signals::PdfSignals;
use zotero_mcp::core::enrichment::scoring::{score, ScoreBreakdown, ScoringInput};
use zotero_mcp::core::enrichment::NormalizedRecord;
use zotero_mcp::core::types::Creator;

fn rec(title: &str, year: &str, doi: Option<&str>, surname: &str) -> NormalizedRecord {
    let mut fields = Map::new();
    fields.insert("title".into(), title.into());
    fields.insert("date".into(), year.into());
    if let Some(d) = doi {
        fields.insert("DOI".into(), d.into());
    }
    NormalizedRecord {
        source: "test".into(),
        fields,
        source_url: None,
        creators: vec![Creator {
            first_name: None,
            last_name: Some(surname.into()),
            creator_type: "author".into(),
            order_index: 0,
        }],
    }
}

#[test]
fn matches_doi_yields_high_score() {
    let signals = PdfSignals {
        doi_candidates: vec!["10.1234/abcd".into()],
        title_candidate: Some("A Paper on Things".into()),
        ..Default::default()
    };
    let current = serde_json::json!({ "title": "paper on things", "date": "2024" });
    let r = rec(
        "A Paper on Things",
        "2024",
        Some("10.1234/abcd"),
        "Aardvark",
    );
    let ScoreBreakdown { score: s, .. } = score(&ScoringInput {
        current_fields: &current,
        signals: &signals,
        candidate: &r,
    });
    assert!(s >= 0.9);
}

#[test]
fn weak_title_match_yields_low_score() {
    let signals = PdfSignals::default();
    let current = serde_json::json!({ "title": "Completely unrelated" });
    let r = rec("Other Paper", "1999", None, "Zilch");
    let ScoreBreakdown { score: s, .. } = score(&ScoringInput {
        current_fields: &current,
        signals: &signals,
        candidate: &r,
    });
    assert!(s < 0.5);
}
