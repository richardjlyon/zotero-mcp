use crate::core::derivatives::{DerivativeStatus, DerivativeStore};
use crate::core::pdf::{
    attachment_key_for, get_pdf_first_pages_stored, get_pdf_text_stored, is_layout_faithful,
    ExtractPolicy, PdfTextResult,
};
use crate::core::reader::annotations::list_annotations;
use crate::core::reader::attachments::{list_attachments, resolve_path};
use crate::core::types::{Annotation, Attachment};
use crate::core::web::{
    get_webpage_content, refetch_url, RefetchResult, WebContentResult, WebMode,
};
use crate::core::writer::attachments::{
    attach_file, attach_link, AttachFileOptions, AttachmentMode,
};
use crate::core::writer::items::create_item;
use crate::state::AppState;
use crate::tools::search::map_err;
use crate::tools::wire::ListResult;
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

pub async fn list_attachments_t(
    s: &AppState,
    a: ItemKeyArgs,
) -> Result<Json<ListResult<Attachment>>, Error> {
    let r = list_attachments(&s.pool, &a.item_key, 1, &s.cfg.storage_dir())
        .await
        .map_err(map_err)?;
    Ok(Json(ListResult::complete(r)))
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
    /// First page to extract (1-indexed, inclusive). Together with `to_page`
    /// this requests a bounded page *window* instead of the whole document,
    /// so large and scanned PDFs stay tractable — extraction work is bounded
    /// by the window, not the document. Omit both for the whole document;
    /// large whole-document requests are refused with a page-count hint, so
    /// walk big documents in windows (e.g. ~20 pages at a time). The result's
    /// `completeness.total_pages` tells you how many pages exist.
    #[serde(default)]
    pub from_page: Option<u32>,
    /// Last page to extract (1-indexed, inclusive). See `from_page`.
    #[serde(default)]
    pub to_page: Option<u32>,
    /// Accept flat-text output instead of an error when the layout-aware
    /// route is configured on this server but unavailable (cold or down).
    /// Default false: a table-free substitute is refused rather than returned
    /// as an ordinary success, because a flat engine drops tables while
    /// returning a similar volume of prose, and a caller reading only the text
    /// cannot tell. Set true when partial text now beats no text — the result
    /// is still labelled and marked incomplete. Not the same as `plain`, which
    /// asks for flat output on purpose and is never gated. On a host with no
    /// layout route configured at all, this argument is irrelevant: flat
    /// extraction succeeds as it always did.
    #[serde(default)]
    pub allow_degraded: bool,
    /// Re-extract even when a stored derivative is current, replacing it.
    /// Normally unnecessary: a changed PDF or a bumped extraction profile
    /// invalidates the stored copy automatically. Use when you suspect the
    /// stored text is wrong rather than merely old.
    #[serde(default)]
    pub refresh: bool,
}

/// Build an optional `(from, to)` window from the two optional page args.
/// Any window with a bound present is honoured (a missing `from` defaults to
/// page 1, a missing `to` defaults to `from`); both absent means whole
/// document.
fn page_window(from: Option<u32>, to: Option<u32>) -> Option<(u32, u32)> {
    match (from, to) {
        (None, None) => None,
        (f, t) => {
            let from = f.unwrap_or(1).max(1);
            let to = t.unwrap_or(from).max(from);
            Some((from, to))
        }
    }
}

pub async fn get_pdf_text_t(s: &AppState, a: PdfTextArgs) -> Result<Json<PdfTextResult>, Error> {
    let r = get_pdf_text_stored(
        &s.pool,
        &a.item_key,
        1,
        &s.cfg.storage_dir(),
        &s.pdf_engines,
        &s.derivatives,
        ExtractPolicy {
            plain: a.plain,
            allow_degraded: a.allow_degraded,
        },
        page_window(a.from_page, a.to_page),
        a.refresh,
    )
    .await
    .map_err(map_err)?;
    Ok(Json(r))
}

/// Where a stored derivative lives and what state it is in — without the
/// document's text.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DerivativePathResult {
    /// `present`, `absent`, `failed`, or `no_pdf`.
    pub status: String,
    /// Path to the markdown **on the machine running this server**. Present
    /// whenever the item has a PDF, whether or not the derivative is built
    /// yet — pair it with `status`. When you reach this server over HTTP this
    /// path is not on your own filesystem.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Path to the source PDF, likewise server-local.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_path: Option<String>,
    /// Engine that produced the stored bytes (not the engine that would run
    /// now — a derivative may be days old).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// Extraction profile the stored bytes were produced under.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_pages: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_count: Option<usize>,
    /// Why the last build failed, when `status` is `failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Return the path to an item's stored text derivative without returning the
/// text. The point is cost: a 69-page report is ~250,000 characters, and
/// handing a path to a tool or a person should not cost the document.
pub async fn get_derivative_path_t(
    s: &AppState,
    a: ItemKeyArgs,
) -> Result<Json<DerivativePathResult>, Error> {
    let pdf_path = match resolve_path(&s.pool, &a.item_key, 1, &s.cfg.storage_dir()).await {
        Ok(p) => p,
        // No PDF is a state, not an error: it is the difference between "not
        // extracted yet" and "nothing to extract".
        Err(_) => {
            return Ok(Json(DerivativePathResult {
                status: "no_pdf".into(),
                path: None,
                pdf_path: None,
                engine: None,
                profile: None,
                total_pages: None,
                character_count: None,
                detail: Some(format!("item {} has no PDF attachment", a.item_key)),
            }))
        }
    };
    let Some(att_key) = attachment_key_for(&pdf_path) else {
        return Err(map_err(crate::core::error::Error::AttachmentNotFound(
            a.item_key.clone(),
        )));
    };
    let hash = DerivativeStore::content_hash(&pdf_path)
        .await
        .map_err(map_err)?;
    let path = Some(
        s.derivatives
            .path_for(&att_key, &hash)
            .to_string_lossy()
            .into_owned(),
    );
    let pdf = Some(pdf_path.to_string_lossy().into_owned());

    Ok(Json(match s.derivatives.status(&att_key, &hash).await {
        DerivativeStatus::Present => {
            let hit = s.derivatives.get(&att_key, &hash).await;
            let meta = hit.map(|h| h.meta);
            DerivativePathResult {
                status: "present".into(),
                path,
                pdf_path: pdf,
                engine: meta.as_ref().map(|m| format!("{:?}", m.engine)),
                profile: meta.as_ref().map(|m| m.profile.clone()),
                total_pages: meta.as_ref().map(|m| m.completeness.total_pages),
                character_count: meta.as_ref().map(|m| m.character_count),
                detail: None,
            }
        }
        DerivativeStatus::Failed(reason) => DerivativePathResult {
            status: "failed".into(),
            path,
            pdf_path: pdf,
            engine: None,
            profile: None,
            total_pages: None,
            character_count: None,
            detail: Some(reason),
        },
        DerivativeStatus::Absent => DerivativePathResult {
            status: "absent".into(),
            path,
            pdf_path: pdf,
            engine: None,
            profile: None,
            total_pages: None,
            character_count: None,
            detail: Some(
                "no derivative built yet — read the item's text once, or run \
                 build_derivatives, to create it"
                    .into(),
            ),
        },
    }))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BuildDerivativesArgs {
    /// Item keys to build derivatives for.
    pub item_keys: Vec<String>,
    /// Rebuild even when a current derivative exists.
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DerivativeOutcome {
    pub item_key: String,
    /// `stored`, `already_present`, `no_pdf`, `not_layout_faithful`, or `failed`.
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BuildDerivativesResult {
    pub stored: usize,
    pub already_present: usize,
    pub skipped_no_pdf: usize,
    pub failed: usize,
    /// One entry per item, in the order given. Statuses only — never text, so
    /// the response stays inside the transport's size ceiling however many
    /// items were processed.
    pub outcomes: Vec<DerivativeOutcome>,
}

/// Backfill: give a set of items durable text derivatives.
///
/// Issues **no** Zotero writes — the store is server-owned — so item metadata,
/// hand-written `extra` fields, tags, collections and notes cannot be touched
/// by this. Resumable: items that already have a current derivative are
/// reported and skipped rather than re-extracted.
pub async fn build_derivatives_t(
    s: &AppState,
    a: BuildDerivativesArgs,
) -> Result<Json<BuildDerivativesResult>, Error> {
    let mut r = BuildDerivativesResult {
        stored: 0,
        already_present: 0,
        skipped_no_pdf: 0,
        failed: 0,
        outcomes: Vec::with_capacity(a.item_keys.len()),
    };
    for key in &a.item_keys {
        let pdf_path = match resolve_path(&s.pool, key, 1, &s.cfg.storage_dir()).await {
            Ok(p) => p,
            Err(_) => {
                r.skipped_no_pdf += 1;
                r.outcomes.push(DerivativeOutcome {
                    item_key: key.clone(),
                    outcome: "no_pdf".into(),
                    detail: Some("item has no PDF attachment".into()),
                });
                continue;
            }
        };
        let hash = match DerivativeStore::content_hash(&pdf_path).await {
            Ok(h) => h,
            Err(e) => {
                r.failed += 1;
                r.outcomes.push(DerivativeOutcome {
                    item_key: key.clone(),
                    outcome: "failed".into(),
                    detail: Some(e.to_string()),
                });
                continue;
            }
        };
        let att_key = attachment_key_for(&pdf_path).unwrap_or_default();
        if !a.refresh
            && matches!(
                s.derivatives.status(&att_key, &hash).await,
                DerivativeStatus::Present
            )
        {
            r.already_present += 1;
            r.outcomes.push(DerivativeOutcome {
                item_key: key.clone(),
                outcome: "already_present".into(),
                detail: None,
            });
            continue;
        }
        match get_pdf_text_stored(
            &s.pool,
            key,
            1,
            &s.cfg.storage_dir(),
            &s.pdf_engines,
            &s.derivatives,
            ExtractPolicy::default(),
            None,
            a.refresh,
        )
        .await
        {
            Ok(res) if is_layout_faithful(&res) => {
                r.stored += 1;
                r.outcomes.push(DerivativeOutcome {
                    item_key: key.clone(),
                    outcome: "stored".into(),
                    detail: None,
                });
            }
            Ok(res) => {
                // Extraction worked but on a flat engine: nothing is stored,
                // because a lossy artefact must never become the permanent one.
                r.failed += 1;
                r.outcomes.push(DerivativeOutcome {
                    item_key: key.clone(),
                    outcome: "not_layout_faithful".into(),
                    detail: Some(format!(
                        "extracted via {:?}, which cannot express tables; nothing stored",
                        res.source
                    )),
                });
            }
            Err(e) => {
                r.failed += 1;
                r.outcomes.push(DerivativeOutcome {
                    item_key: key.clone(),
                    outcome: "failed".into(),
                    detail: Some(e.to_string()),
                });
            }
        }
    }
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
    /// Accept flat-text output instead of an error when the layout-aware
    /// route is configured on this server but unavailable (cold or down).
    /// Default false: a table-free substitute is refused rather than returned
    /// as an ordinary success, because a flat engine drops tables while
    /// returning a similar volume of prose, and a caller reading only the text
    /// cannot tell. Set true when partial text now beats no text — the result
    /// is still labelled and marked incomplete. Not the same as `plain`, which
    /// asks for flat output on purpose and is never gated. On a host with no
    /// layout route configured at all, this argument is irrelevant: flat
    /// extraction succeeds as it always did.
    #[serde(default)]
    pub allow_degraded: bool,
}
fn two() -> usize {
    2
}

pub async fn get_pdf_first_pages_t(
    s: &AppState,
    a: FirstPagesArgs,
) -> Result<Json<PdfTextResult>, Error> {
    let r = get_pdf_first_pages_stored(
        &s.pool,
        &a.item_key,
        1,
        &s.cfg.storage_dir(),
        a.n,
        &s.pdf_engines,
        &s.derivatives,
        ExtractPolicy {
            plain: a.plain,
            allow_degraded: a.allow_degraded,
        },
    )
    .await
    .map_err(map_err)?;
    Ok(Json(r))
}

pub async fn list_annotations_t(
    s: &AppState,
    a: ItemKeyArgs,
) -> Result<Json<ListResult<Annotation>>, Error> {
    let r = list_annotations(&s.pool, &a.item_key, 1)
        .await
        .map_err(map_err)?;
    Ok(Json(ListResult::complete(r)))
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
    /// Advanced escape hatch — omit it. Omitted means the file is stored the
    /// way Zotero's own UI would store it, which is what you almost always
    /// want. "linked_file" makes Zotero store only a path reference to the
    /// file instead of the file itself, for BYO-storage setups (a Calibre
    /// mirror, a shared NAS); "imported_file" names the default explicitly.
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
}

/// Resolve the storage mode for one `attach_file` call.
///
/// Config does not participate. An omitted `mode` means "store it the way
/// Zotero's own UI would", which is the imported-file route; where the bytes
/// travel from there is Zotero's own file-sync preference, not this server's
/// decision. An unrecognised `mode` string falls back to the same route
/// (with a WARN, via [`AttachmentMode::from_config`]) rather than erroring.
fn resolve_mode(mode: Option<&str>) -> AttachmentMode {
    mode.map(AttachmentMode::from_config)
        .unwrap_or(AttachmentMode::ImportedFile)
}

pub async fn attach_file_t(
    s: &AppState,
    a: AttachFileArgs,
) -> Result<Json<AttachmentResult>, Error> {
    let cfg = &s.cfg.zotero;
    let opts = AttachFileOptions {
        mode: resolve_mode(a.mode.as_deref()),
        // Never config-derived: an explicit `mode: "linked_file"` call stores
        // the file's absolute path. Only direct core-API callers set a base dir.
        linked_attachment_base_dir: None,
        storage_dir: s.cfg.storage_dir(),
        max_attachment_bytes: cfg.max_attachment_bytes,
        filename: a.filename,
        content_type: a.content_type,
    };
    let path = PathBuf::from(crate::core::config::remap_path(&cfg.path_map, &a.file_path));
    let key = attach_file(&s.api, &a.parent_key, &path, &opts)
        .await
        .map_err(map_err)?;
    Ok(Json(AttachmentResult {
        attachment_key: key,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The storage-mode simplification guard (v0.4.0): an omitted `mode` is not
    // "look up the config default" any more — it is "the way Zotero's UI would".
    #[test]
    fn omitted_mode_routes_to_imported_file() {
        assert_eq!(resolve_mode(None), AttachmentMode::ImportedFile);
    }

    #[test]
    fn explicit_linked_file_still_honoured() {
        assert_eq!(
            resolve_mode(Some("linked_file")),
            AttachmentMode::LinkedFile
        );
    }

    #[test]
    fn explicit_imported_file_still_honoured() {
        assert_eq!(
            resolve_mode(Some("imported_file")),
            AttachmentMode::ImportedFile
        );
    }

    #[test]
    fn unknown_mode_falls_back_to_imported_file() {
        assert_eq!(resolve_mode(Some("nonsense")), AttachmentMode::ImportedFile);
    }
}
