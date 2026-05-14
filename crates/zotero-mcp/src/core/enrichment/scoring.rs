use crate::core::enrichment::pdf_signals::PdfSignals;
use crate::core::enrichment::NormalizedRecord;
use serde_json::Value;

pub struct ScoringInput<'a> {
    pub current_fields: &'a Value,
    pub signals: &'a PdfSignals,
    pub candidate: &'a NormalizedRecord,
}

pub struct ScoreBreakdown {
    pub score: f64,
    pub reasons: Vec<String>,
}

pub fn score(inp: &ScoringInput<'_>) -> ScoreBreakdown {
    let mut s: f64 = 0.0;
    let mut reasons = vec![];

    // DOI direct: if the candidate's DOI matches a signal DOI, big positive
    if let Some(doi_c) = inp.candidate.fields.get("DOI").and_then(|x| x.as_str()) {
        if inp
            .signals
            .doi_candidates
            .iter()
            .any(|d| d.eq_ignore_ascii_case(doi_c))
        {
            s += 0.5;
            reasons.push("DOI found in PDF first page".into());
        }
    }

    // Title fuzzy
    let cand_title = inp
        .candidate
        .fields
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let cur_title = inp
        .current_fields
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let signal_title = inp.signals.title_candidate.as_deref().unwrap_or("");
    let title_score = token_overlap(cand_title, &[cur_title, signal_title].join(" "));
    if title_score >= 0.9 {
        s += 0.35;
        reasons.push("title token overlap >= 0.9".into());
    } else if title_score >= 0.7 {
        s += 0.15;
        reasons.push("title token overlap 0.7..0.9".into());
    } else if !cur_title.is_empty() || !signal_title.is_empty() {
        s -= 0.15;
        reasons.push("title overlap < 0.7".into());
    }

    // First-author surname match
    let cand_surname = inp
        .candidate
        .creators
        .first()
        .and_then(|c| c.last_name.as_deref())
        .unwrap_or("")
        .to_lowercase();
    if !cand_surname.is_empty()
        && inp
            .signals
            .author_candidates
            .iter()
            .any(|a| a.to_lowercase().contains(&cand_surname))
    {
        s += 0.1;
        reasons.push("first-author surname appears in PDF authors line".into());
    }

    // Year ±1
    let cand_year = inp
        .candidate
        .fields
        .get("date")
        .and_then(|x| x.as_str())
        .and_then(year_of);
    let cur_year = inp
        .current_fields
        .get("date")
        .and_then(|x| x.as_str())
        .and_then(year_of);
    if let (Some(c), Some(u)) = (cand_year, cur_year) {
        if (c - u).abs() <= 1 {
            s += 0.05;
            reasons.push("year within ±1".into());
        } else {
            s -= 0.05;
            reasons.push("year mismatch > 1".into());
        }
    }

    let clamped = s.clamp(0.0, 1.0);
    ScoreBreakdown {
        score: clamped,
        reasons,
    }
}

fn token_overlap(a: &str, b: &str) -> f64 {
    let an: std::collections::HashSet<String> = tokens(a);
    let bn: std::collections::HashSet<String> = tokens(b);
    if an.is_empty() || bn.is_empty() {
        return 0.0;
    }
    let inter = an.intersection(&bn).count() as f64;
    let denom = an.len().min(bn.len()) as f64;
    inter / denom
}

fn tokens(s: &str) -> std::collections::HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .collect()
}

fn year_of(s: &str) -> Option<i64> {
    s.split('-').next().and_then(|y| y.parse::<i64>().ok())
}
