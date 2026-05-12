//! Attachment-creation primitives.
//!
//! - [`attach_link`]: single POST that creates a `linked_url` child attachment
//!   (URL only, no bytes).
//! - [`attach_file`]: file-on-disk attachment, supporting both `imported_file`
//!   (3-step upload to Zotero's cloud) and `linked_file` (path reference only).

use crate::core::error::{Error, Result};
use crate::core::writer::client::LocalApi;
use reqwest::Method;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Attach a URL as a `linked_url` child to an existing parent item.
///
/// One POST; no bytes transfer. Returns the new attachment item key.
pub async fn attach_link(
    api: &LocalApi,
    parent_key: &str,
    url: &str,
    title: Option<&str>,
) -> Result<String> {
    let title = title.unwrap_or(url);
    let body = json!([{
        "itemType": "attachment",
        "parentItem": parent_key,
        "linkMode": "linked_url",
        "url": url,
        "title": title,
        "tags": [],
        "relations": {}
    }]);
    let resp = api
        .write_request(Method::POST, "/items")?
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(Error::LocalApi {
            status: status.as_u16(),
            body: body_text,
        });
    }
    let v: Value = resp.json().await?;
    v.get("successful")
        .and_then(|s| s.get("0"))
        .and_then(|i| i.get("key"))
        .and_then(|k| k.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::LocalApi {
            status: status.as_u16(),
            body: v.to_string(),
        })
}

// ---------------------------------------------------------------------------
// attach_file — linked_file and imported_file modes
// ---------------------------------------------------------------------------

/// Which Zotero attachment storage mode to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentMode {
    ImportedFile,
    LinkedFile,
}

impl AttachmentMode {
    /// Parse from the config string. Returns `ImportedFile` for unknown values
    /// with a warn-level log; this matches the "graceful default" stance of
    /// the rest of the config layer.
    pub fn from_config(s: &str) -> Self {
        match s {
            "linked_file" => AttachmentMode::LinkedFile,
            "imported_file" => AttachmentMode::ImportedFile,
            other => {
                tracing::warn!(
                    value = other,
                    "unknown attachment_mode in config; falling back to imported_file"
                );
                AttachmentMode::ImportedFile
            }
        }
    }
}

/// Options for [`attach_file`].
#[derive(Debug, Clone)]
pub struct AttachFileOptions {
    pub mode: AttachmentMode,
    /// When `mode` is `LinkedFile`, files must be inside this directory.
    /// The stored path uses Zotero's `attachments:<relative>` prefix so it
    /// can be resolved on any device that has the same base dir configured.
    pub linked_attachment_base_dir: Option<PathBuf>,
    /// Pre-flight size cap (bytes). Requests exceeding this return
    /// [`Error::AttachmentTooLarge`] before any network call.
    pub max_attachment_bytes: usize,
    /// Override the attachment title / filename stored in Zotero metadata.
    /// Defaults to the file's own name.
    pub filename: Option<String>,
    /// Override the MIME content-type. Defaults to `mime_guess` result.
    pub content_type: Option<String>,
}

/// Attach a local file to a Zotero parent item.
///
/// `mode` selects between Zotero's `imported_file` (bytes uploaded to
/// Zotero's cloud) and `linked_file` (path reference only). Pre-flight
/// validation (file exists, size ≤ max_attachment_bytes, base-dir
/// relativity for linked_file) happens before any network call.
pub async fn attach_file(
    api: &LocalApi,
    parent_key: &str,
    file_path: &Path,
    opts: &AttachFileOptions,
) -> Result<String> {
    // Pre-flight: existence + size cap (cheap, no network).
    let meta = tokio::fs::metadata(file_path)
        .await
        .map_err(|_| Error::AttachmentFileNotFound(file_path.to_path_buf()))?;
    let size = meta.len() as usize;
    if size > opts.max_attachment_bytes {
        return Err(Error::AttachmentTooLarge {
            file_path: file_path.to_path_buf(),
            limit: opts.max_attachment_bytes,
        });
    }

    let filename = opts.filename.clone().unwrap_or_else(|| {
        file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment")
            .to_string()
    });
    let content_type = opts.content_type.clone().unwrap_or_else(|| {
        mime_guess::from_path(file_path)
            .first_or_octet_stream()
            .to_string()
    });

    match opts.mode {
        AttachmentMode::LinkedFile => {
            attach_file_linked(api, parent_key, file_path, &filename, &content_type, opts).await
        }
        AttachmentMode::ImportedFile => {
            // Implemented in Task 9.
            Err(Error::UploadFailed {
                stage: "init",
                detail: "imported_file mode not yet implemented (Task 9)".into(),
            })
        }
    }
}

async fn attach_file_linked(
    api: &LocalApi,
    parent_key: &str,
    file_path: &Path,
    filename: &str,
    content_type: &str,
    opts: &AttachFileOptions,
) -> Result<String> {
    let path_value = match opts.linked_attachment_base_dir.as_ref() {
        Some(base) => {
            let rel = file_path.strip_prefix(base).map_err(|_| {
                Error::AttachmentOutsideBaseDir {
                    file_path: file_path.to_path_buf(),
                    base_dir: base.clone(),
                }
            })?;
            format!("attachments:{}", rel.display())
        }
        None => {
            tracing::warn!(
                file = %file_path.display(),
                "linked_attachment_base_dir not configured; storing absolute path. \
                 File will not replicate to other Zotero clients."
            );
            file_path.display().to_string()
        }
    };

    let body = json!([{
        "itemType": "attachment",
        "parentItem": parent_key,
        "linkMode": "linked_file",
        "title": filename,
        "path": path_value,
        "contentType": content_type,
        "tags": [],
        "relations": {}
    }]);
    let resp = api
        .write_request(Method::POST, "/items")?
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(Error::LocalApi {
            status: status.as_u16(),
            body: body_text,
        });
    }
    let v: Value = resp.json().await?;
    v.get("successful")
        .and_then(|s| s.get("0"))
        .and_then(|i| i.get("key"))
        .and_then(|k| k.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::LocalApi {
            status: status.as_u16(),
            body: v.to_string(),
        })
}
