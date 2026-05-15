//! Attachment-creation primitives.
//!
//! - [`attach_link`]: single POST that creates a `linked_url` child attachment
//!   (URL only, no bytes).
//! - [`attach_file`]: file-on-disk attachment, supporting both `imported_file`
//!   (row + local storage write; desktop sync engine handles the file backend)
//!   and `linked_file` (path reference only).

use crate::core::error::{Error, Result};
use crate::core::writer::client::LocalApi;
use reqwest::Method;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn md5_hex(bytes: &[u8]) -> String {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(bytes);
    let d = h.finalize();
    let mut s = String::with_capacity(32);
    for b in d {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

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
    /// Required for `ImportedFile` mode: Zotero's resolved `storage/` directory
    /// (typically `~/Zotero/storage`). `attach_file` drops bytes at
    /// `<storage_dir>/<attach_key>/<filename>` and lets the desktop client's
    /// sync engine push them to whichever file backend the user has configured
    /// (Zotero cloud, WebDAV, or none). Unused for `LinkedFile` mode.
    pub storage_dir: PathBuf,
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
            let bytes = tokio::fs::read(file_path)
                .await
                .map_err(|e| Error::UploadFailed {
                    stage: "read",
                    detail: format!("reading {}: {}", file_path.display(), e),
                })?;
            attach_file_imported(
                api,
                parent_key,
                &bytes,
                &filename,
                &content_type,
                &opts.storage_dir,
            )
            .await
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
            let rel =
                file_path
                    .strip_prefix(base)
                    .map_err(|_| Error::AttachmentOutsideBaseDir {
                        file_path: file_path.to_path_buf(),
                        base_dir: base.clone(),
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

async fn attach_file_imported(
    api: &LocalApi,
    parent_key: &str,
    bytes: &[u8],
    filename: &str,
    content_type: &str,
    storage_dir: &Path,
) -> Result<String> {
    let md5 = md5_hex(bytes);
    let mtime = unix_ms_now();

    // Create the attachment row with md5 + mtime pre-populated so Zotero
    // treats the file as already-linked once bytes appear under storage_dir.
    // The desktop client's normal sync engine then pushes them to whichever
    // file backend the user has configured (Zotero cloud, WebDAV, or none).
    let attach_key =
        create_imported_attachment_row(api, parent_key, filename, content_type, &md5, mtime)
            .await?;

    let dest_dir = storage_dir.join(&attach_key);
    tokio::fs::create_dir_all(&dest_dir)
        .await
        .map_err(|e| Error::UploadFailed {
            stage: "create_storage_dir",
            detail: format!("creating {}: {}", dest_dir.display(), e),
        })?;
    let dest_file = dest_dir.join(filename);
    tokio::fs::write(&dest_file, bytes)
        .await
        .map_err(|e| Error::UploadFailed {
            stage: "write_bytes",
            detail: format!("writing {}: {}", dest_file.display(), e),
        })?;

    Ok(attach_key)
}

async fn create_imported_attachment_row(
    api: &LocalApi,
    parent_key: &str,
    filename: &str,
    content_type: &str,
    md5: &str,
    mtime: u64,
) -> Result<String> {
    let body = json!([{
        "itemType": "attachment",
        "parentItem": parent_key,
        "linkMode": "imported_file",
        "title": filename,
        "filename": filename,
        "contentType": content_type,
        "charset": "",
        "md5": md5,
        "mtime": mtime,
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
