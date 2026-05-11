use crate::core::error::{Error, Result};
use crate::core::enrichment::{NormalizedRecord, pdf_signals::PdfSignals, scoring};
use crate::core::pdf::get_pdf_first_pages;
use crate::core::reader::pool::ReadOnlyPool;
use crate::core::reader::items::get_item_by_key;
use crate::core::types::{Diff, EnrichmentProposal, FieldChange, SourceBreakdown};
use crate::core::writer::client::LocalApi;
use crate::core::writer::items::update_item_fields;
use serde_json::{Map, Value};
use std::path::Path;

pub fn compute_diff(current: &Value, proposed: &Value) -> Diff {
    let mut changes = vec![];
    if let Value::Object(pm) = proposed {
        for (k, pv) in pm {
            let cv = current.get(k).cloned();
            let differs = match &cv {
                Some(x) => x != pv,
                None => !pv.is_null(),
            };
            if differs {
                changes.push(FieldChange { field: k.clone(), current: cv, proposed: pv.clone() });
            }
        }
    }
    Diff { changes }
}

/// Find items whose metadata looks stubby. Heuristics:
///   - missing DOI on a journalArticle
///   - missing abstractNote
///   - title equal to attached filename
///   - very short title
pub async fn find_weak_metadata_items(pool: &ReadOnlyPool, library_id: i64, limit: i64) -> Result<Vec<(String, Vec<String>)>> {
    pool.with_conn(move |c| {
        let mut out = vec![];
        let mut stmt = c.prepare(
            "SELECT i.key, it.typeName,
                (SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = i.itemID AND d.fieldID = (SELECT fieldID FROM fieldsCombined WHERE fieldName='title')) AS title,
                (SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = i.itemID AND d.fieldID = (SELECT fieldID FROM fieldsCombined WHERE fieldName='DOI')) AS doi,
                (SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = i.itemID AND d.fieldID = (SELECT fieldID FROM fieldsCombined WHERE fieldName='abstractNote')) AS abs
            FROM items i JOIN itemTypes it ON it.itemTypeID = i.itemTypeID
            WHERE i.libraryID = ? AND it.typeName NOT IN ('attachment','note','annotation')
            LIMIT ?"
        )?;
        let mut rows = stmt.query(rusqlite::params![library_id, limit])?;
        while let Some(r) = rows.next()? {
            let mut reasons: Vec<String> = vec![];
            let key: String = r.get(0)?;
            let typ: String = r.get(1)?;
            let title: Option<String> = r.get(2)?;
            let doi: Option<String> = r.get(3)?;
            let abs: Option<String> = r.get(4)?;
            if typ == "journalArticle" && doi.as_deref().unwrap_or("").is_empty() {
                reasons.push("missing DOI on journalArticle".into());
            }
            if abs.as_deref().unwrap_or("").is_empty() {
                reasons.push("missing abstractNote".into());
            }
            if let Some(t) = &title {
                if t.len() < 8 { reasons.push("very short title".into()); }
                if t.ends_with(".pdf") || t.ends_with(".html") { reasons.push("title looks like a filename".into()); }
            } else {
                reasons.push("missing title".into());
            }
            if !reasons.is_empty() { out.push((key, reasons)); }
        }
        Ok(out)
    }).await
}

pub struct ProposeInput<'a> {
    pub item_key: &'a str,
    pub library_id: i64,
    pub storage_dir: &'a Path,
    pub candidates: Vec<NormalizedRecord>,
}

pub async fn propose_metadata_update(pool: &ReadOnlyPool, inp: ProposeInput<'_>) -> Result<EnrichmentProposal> {
    let item = get_item_by_key(pool, inp.item_key, inp.library_id).await?;

    // Pull PDF first-page signals if we have a PDF attachment
    let signals = match get_pdf_first_pages(pool, inp.item_key, inp.library_id, inp.storage_dir, 1).await {
        Ok(p) => crate::core::enrichment::pdf_signals::extract_signals(&p.text),
        Err(_) => PdfSignals::default(),
    };

    // Score each candidate; pick best
    let mut best: Option<(f64, usize, Vec<String>)> = None;
    let mut source_breakdown: Vec<SourceBreakdown> = vec![];
    for (idx, c) in inp.candidates.iter().enumerate() {
        let scoring::ScoreBreakdown { score: s, reasons } = scoring::score(&scoring::ScoringInput {
            current_fields: &item.fields, signals: &signals, candidate: c,
        });
        source_breakdown.push(SourceBreakdown {
            source: c.source.clone(),
            matched: s > 0.5,
            fields_contributed: c.fields.iter().map(|(k, _)| k.clone()).collect(),
            raw_response_cached: true,
        });
        if best.as_ref().map(|(b, _, _)| s > *b).unwrap_or(true) {
            best = Some((s, idx, reasons));
        }
    }
    let (confidence, best_idx, _reasons) = best.ok_or_else(|| Error::Lookup { r#source: "any".into(), message: "no candidates".into() })?;
    let candidate = &inp.candidates[best_idx];

    // Build proposed fields, merging only when current is empty/null
    let mut proposed = Map::new();
    if let Value::Object(cur) = &item.fields {
        for (k, v) in cur { proposed.insert(k.clone(), v.clone()); }
    }
    for (k, v) in &candidate.fields {
        let cur_empty = proposed.get(k).map(|x| matches!(x, Value::Null) || x.as_str().map(|s| s.is_empty()).unwrap_or(false)).unwrap_or(true);
        if cur_empty { proposed.insert(k.clone(), v.clone()); }
    }
    let proposed_v = Value::Object(proposed);
    let diff = compute_diff(&item.fields, &proposed_v);

    let needs_review = confidence < 0.9 || source_breakdown.iter().filter(|s| s.matched).count() < 2;

    Ok(EnrichmentProposal {
        item_key: inp.item_key.into(),
        diff,
        confidence,
        source_breakdown,
        needs_review,
    })
}

pub async fn apply_metadata_update(api: &LocalApi, pool: &ReadOnlyPool, library_id: i64, proposal: &EnrichmentProposal) -> Result<()> {
    let item = get_item_by_key(pool, &proposal.item_key, library_id).await?;
    let mut patch = Map::new();
    for ch in &proposal.diff.changes {
        patch.insert(ch.field.clone(), ch.proposed.clone());
    }
    update_item_fields(api, &proposal.item_key, item.version, Value::Object(patch)).await
}

pub struct EnrichInput<'a> {
    pub item_key: &'a str,
    pub library_id: i64,
    pub storage_dir: &'a Path,
    pub candidates: Vec<NormalizedRecord>,
    pub auto_apply_threshold: f64,
}

pub async fn enrich_item(api: &LocalApi, pool: &ReadOnlyPool, inp: EnrichInput<'_>) -> Result<EnrichmentProposal> {
    let auto = inp.auto_apply_threshold;
    let proposal = propose_metadata_update(pool, ProposeInput {
        item_key: inp.item_key,
        library_id: inp.library_id,
        storage_dir: inp.storage_dir,
        candidates: inp.candidates,
    }).await?;
    if proposal.confidence >= auto && !proposal.needs_review {
        apply_metadata_update(api, pool, inp.library_id, &proposal).await?;
    }
    Ok(proposal)
}
