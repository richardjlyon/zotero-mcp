use crate::state::AppState;
use crate::tools::search::map_err;
use rmcp::Error;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use zotero_core::pdf::{get_pdf_first_pages, get_pdf_text};
use zotero_core::reader::annotations::list_annotations;
use zotero_core::reader::attachments::{list_attachments, resolve_path};
use zotero_core::web::{get_webpage_content, refetch_url, WebMode};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ItemKeyArgs {
    pub item_key: String,
}

pub async fn list_attachments_t(s: &AppState, a: ItemKeyArgs) -> Result<CallToolResult, Error> {
    let r = list_attachments(&s.pool, &a.item_key, 1, &s.cfg.storage_dir())
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}

pub async fn get_pdf_path(s: &AppState, a: ItemKeyArgs) -> Result<CallToolResult, Error> {
    let p = resolve_path(&s.pool, &a.item_key, 1, &s.cfg.storage_dir())
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::text(
        p.to_string_lossy().into_owned(),
    )]))
}

pub async fn get_pdf_text_t(s: &AppState, a: ItemKeyArgs) -> Result<CallToolResult, Error> {
    let r = get_pdf_text(&s.pool, &a.item_key, 1, &s.cfg.storage_dir())
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FirstPagesArgs {
    pub item_key: String,
    #[serde(default = "two")]
    pub n: usize,
}
fn two() -> usize {
    2
}

pub async fn get_pdf_first_pages_t(s: &AppState, a: FirstPagesArgs) -> Result<CallToolResult, Error> {
    let r = get_pdf_first_pages(&s.pool, &a.item_key, 1, &s.cfg.storage_dir(), a.n)
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}

pub async fn list_annotations_t(s: &AppState, a: ItemKeyArgs) -> Result<CallToolResult, Error> {
    let r = list_annotations(&s.pool, &a.item_key, 1)
        .await
        .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct WebArgs {
    pub item_key: String,
    #[serde(default = "default_auto")]
    pub mode: String,
}
fn default_auto() -> String {
    "auto".into()
}

pub async fn get_webpage_content_t(s: &AppState, a: WebArgs) -> Result<CallToolResult, Error> {
    let mode = match a.mode.as_str() {
        "snapshot" => WebMode::Snapshot,
        "live" => WebMode::Live,
        _ => WebMode::Auto,
    };
    let r = get_webpage_content(
        &s.pool,
        &a.item_key,
        1,
        &s.cfg.storage_dir(),
        mode,
        &s.cfg.web.user_agent,
    )
    .await
    .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RefetchArgs {
    pub item_key: String,
    #[serde(default)]
    pub save_as_snapshot: bool,
}

pub async fn refetch_url_t(s: &AppState, a: RefetchArgs) -> Result<CallToolResult, Error> {
    let r = refetch_url(
        &s.pool,
        Some(&s.api),
        &a.item_key,
        1,
        a.save_as_snapshot,
        &s.cfg.web.user_agent,
    )
    .await
    .map_err(map_err)?;
    Ok(CallToolResult::success(vec![Content::json(
        serde_json::to_value(&r).unwrap(),
    )?]))
}
