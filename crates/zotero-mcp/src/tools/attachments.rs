use crate::core::pdf::{get_pdf_first_pages, get_pdf_text, PdfTextResult};
use crate::core::reader::annotations::list_annotations;
use crate::core::reader::attachments::{list_attachments, resolve_path};
use crate::core::web::{
    get_webpage_content, refetch_url, RefetchResult, WebContentResult, WebMode,
};
use crate::core::writer::attachments::{
    attach_file, attach_link, AttachFileOptions, AttachmentMode,
};
use crate::core::writer::items::create_item;
use crate::state::AppState;
use crate::tools::search::map_err;
use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData as Error;
use rmcp::Json;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CreateItemResult {
    pub item_key: String,
    pub version: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AttachmentResult {
    pub attachment_key: String,
}

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

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PdfTextArgs {
    pub item_key: String,
    /// Force the old flat-text extraction path (format `plain`, no page
    /// anchors), skipping the layout-aware Docling route.
    #[serde(default)]
    pub plain: bool,
}

pub async fn get_pdf_text_t(s: &AppState, a: PdfTextArgs) -> Result<Json<PdfTextResult>, Error> {
    let r = get_pdf_text(
        &s.pool,
        &a.item_key,
        1,
        &s.cfg.storage_dir(),
        &s.pdf_engines,
        a.plain,
    )
    .await
    .map_err(map_err)?;
    Ok(Json(r))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FirstPagesArgs {
    pub item_key: String,
    #[serde(default = "two")]
    pub n: usize,
    /// Force the old flat-text extraction path (format `plain`, no page
    /// anchors), skipping the layout-aware Docling route.
    #[serde(default)]
    pub plain: bool,
}
fn two() -> usize {
    2
}

pub async fn get_pdf_first_pages_t(
    s: &AppState,
    a: FirstPagesArgs,
) -> Result<Json<PdfTextResult>, Error> {
    let r = get_pdf_first_pages(
        &s.pool,
        &a.item_key,
        1,
        &s.cfg.storage_dir(),
        a.n,
        &s.pdf_engines,
        a.plain,
    )
    .await
    .map_err(map_err)?;
    Ok(Json(r))
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

pub async fn get_webpage_content_t(
    s: &AppState,
    a: WebArgs,
) -> Result<Json<WebContentResult>, Error> {
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
    Ok(Json(r))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RefetchArgs {
    pub item_key: String,
    #[serde(default)]
    pub save_as_snapshot: bool,
}

pub async fn refetch_url_t(s: &AppState, a: RefetchArgs) -> Result<Json<RefetchResult>, Error> {
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
    Ok(Json(r))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateItemArgs {
    /// Zotero-shaped item JSON object. Required key: `itemType` (string).
    /// Other keys pass through to the Zotero Web API. The output of
    /// `lookup_doi`/`lookup_isbn`/`lookup_arxiv` with the default
    /// `format='zotero'` is directly compatible.
    pub item: Map<String, Value>,
    /// Optional collection keys to file the new item under. Equivalent to
    /// setting `collections` inside `item`; the two are unioned.
    #[serde(default)]
    pub collection_keys: Vec<String>,
}

pub async fn create_item_t(
    s: &AppState,
    a: CreateItemArgs,
) -> Result<Json<CreateItemResult>, Error> {
    let item_value = Value::Object(a.item);
    let (key, version) = create_item(&s.api, &item_value, &a.collection_keys)
        .await
        .map_err(map_err)?;
    Ok(Json(CreateItemResult {
        item_key: key,
        version,
    }))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AttachLinkArgs {
    pub parent_key: String,
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
}

pub async fn attach_link_t(
    s: &AppState,
    a: AttachLinkArgs,
) -> Result<Json<AttachmentResult>, Error> {
    let key = attach_link(&s.api, &a.parent_key, &a.url, a.title.as_deref())
        .await
        .map_err(map_err)?;
    Ok(Json(AttachmentResult {
        attachment_key: key,
    }))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AttachFileArgs {
    pub parent_key: String,
    /// Absolute path to a local file.
    pub file_path: String,
    /// Override the config-default attachment mode. "imported_file" uploads
    /// bytes to Zotero cloud storage; "linked_file" stores a path reference
    /// (BYO storage). Omit to use cfg.zotero.attachment_mode.
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
}

pub async fn attach_file_t(
    s: &AppState,
    a: AttachFileArgs,
) -> Result<Json<AttachmentResult>, Error> {
    let cfg = &s.cfg.zotero;
    let mode_str = a.mode.as_deref().unwrap_or(&cfg.attachment_mode);
    let mode = AttachmentMode::from_config(mode_str);
    let opts = AttachFileOptions {
        mode,
        linked_attachment_base_dir: cfg
            .linked_attachment_base_dir
            .as_deref()
            .map(crate::core::config::expand_tilde)
            .map(PathBuf::from),
        storage_dir: s.cfg.storage_dir(),
        max_attachment_bytes: cfg.max_attachment_bytes,
        filename: a.filename,
        content_type: a.content_type,
    };
    let path = PathBuf::from(crate::core::config::expand_tilde(&a.file_path));
    let key = attach_file(&s.api, &a.parent_key, &path, &opts)
        .await
        .map_err(map_err)?;
    Ok(Json(AttachmentResult {
        attachment_key: key,
    }))
}
