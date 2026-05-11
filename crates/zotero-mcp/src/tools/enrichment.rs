use std::collections::BTreeMap;

use crate::state::AppState;
use crate::tools::search::map_err;
use rmcp::Error;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::core::enrichment::NormalizedRecord;
use crate::core::enrichment::propose::{
    EnrichInput, ProposeInput, apply_metadata_update, enrich_item, find_weak_metadata_items,
    propose_metadata_update,
};
use crate::core::types::EnrichmentProposal;

fn invalid(msg: String) -> Error {
    Error::invalid_params(msg, None)
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct WeakArgs {
    #[serde(default = "fifty")]
    pub limit: i64,
}
fn fifty() -> i64 {
    50
}

pub async fn find_weak_metadata_items_t(
    s: &AppState,
    a: WeakArgs,
) -> Result<CallToolResult, Error> {
    let r = find_weak_metadata_items(&s.pool, 1, a.limit)
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DoiArgs {
    pub doi: String,
}

pub async fn lookup_doi_t(s: &AppState, a: DoiArgs) -> Result<CallToolResult, Error> {
    let r = s.crossref.lookup_doi(&a.doi).await.map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IsbnArgs {
    pub isbn: String,
}

pub async fn lookup_isbn_t(s: &AppState, a: IsbnArgs) -> Result<CallToolResult, Error> {
    let r = s.openlibrary.lookup_isbn(&a.isbn).await.map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ArxivArgs {
    pub id: String,
}

pub async fn lookup_arxiv_t(s: &AppState, a: ArxivArgs) -> Result<CallToolResult, Error> {
    let r = s.arxiv.lookup_arxiv(&a.id).await.map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SearchSourceArgs {
    pub query: String,
    #[serde(default = "ten")]
    pub limit: usize,
}
fn ten() -> usize {
    10
}

pub async fn search_crossref_t(s: &AppState, a: SearchSourceArgs) -> Result<CallToolResult, Error> {
    let r = s
        .crossref
        .search(&a.query, a.limit)
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}

pub async fn search_semantic_scholar_t(
    s: &AppState,
    a: SearchSourceArgs,
) -> Result<CallToolResult, Error> {
    let r = s
        .semantic_scholar
        .search(&a.query, a.limit)
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ProposeArgs {
    pub item_key: String,
    /// JSON array of NormalizedRecord objects.
    pub candidates: Vec<Value>,
}

fn parse_candidates(arr: Vec<Value>) -> Result<Vec<NormalizedRecord>, Error> {
    arr.into_iter()
        .enumerate()
        .map(|(i, v)| {
            serde_json::from_value(v).map_err(|e| {
                invalid(format!("candidates[{}] invalid NormalizedRecord: {}", i, e))
            })
        })
        .collect()
}

pub async fn propose_metadata_update_t(
    s: &AppState,
    a: ProposeArgs,
) -> Result<CallToolResult, Error> {
    let candidates = parse_candidates(a.candidates)?;
    let storage_dir = s.cfg.storage_dir();
    let p = propose_metadata_update(
        &s.pool,
        ProposeInput {
            item_key: &a.item_key,
            library_id: 1,
            storage_dir: &storage_dir,
            candidates,
            engines: &s.pdf_engines,
        },
    )
    .await
    .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&p).unwrap(),
    )?]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ApplyArgs {
    /// A previously returned EnrichmentProposal JSON object.
    pub proposal: BTreeMap<String, Value>,
}

pub async fn apply_metadata_update_t(
    s: &AppState,
    a: ApplyArgs,
) -> Result<CallToolResult, Error> {
    let proposal: EnrichmentProposal =
        serde_json::from_value(serde_json::to_value(&a.proposal).unwrap())
            .map_err(|e| invalid(format!("proposal is not a valid EnrichmentProposal: {}", e)))?;
    apply_metadata_update(&s.api, &s.pool, 1, &proposal)
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::text("applied")]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EnrichArgs {
    pub item_key: String,
    pub candidates: Vec<Value>,
    #[serde(default)]
    pub auto_apply_threshold: Option<f64>,
}

pub async fn enrich_item_t(s: &AppState, a: EnrichArgs) -> Result<CallToolResult, Error> {
    let candidates = parse_candidates(a.candidates)?;
    let threshold = a
        .auto_apply_threshold
        .unwrap_or(s.cfg.enrichment.auto_apply_threshold);
    let storage_dir = s.cfg.storage_dir();
    let p = enrich_item(
        &s.api,
        &s.pool,
        EnrichInput {
            item_key: &a.item_key,
            library_id: 1,
            storage_dir: &storage_dir,
            candidates,
            auto_apply_threshold: threshold,
            engines: &s.pdf_engines,
        },
    )
    .await
    .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&p).unwrap(),
    )?]))
}
