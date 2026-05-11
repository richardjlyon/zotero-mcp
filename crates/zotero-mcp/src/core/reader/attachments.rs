use crate::core::error::{Error, Result};
use crate::core::reader::pool::ReadOnlyPool;
use crate::core::types::{Attachment, AttachmentLinkMode};
use std::path::{Path, PathBuf};

/// Lists attachments for an item identified by its parent's Zotero key.
/// Returns child attachments (PDFs, snapshots), each with resolved filesystem path
/// when possible.
pub async fn list_attachments(
    pool: &ReadOnlyPool,
    parent_key: &str,
    library_id: i64,
    storage_dir: &Path,
) -> Result<Vec<Attachment>> {
    let parent_key = parent_key.to_string();
    let storage_dir = storage_dir.to_path_buf();

    pool.with_conn(move |c| {
        let parent_id: Option<i64> = c.query_row(
            "SELECT itemID FROM items WHERE libraryID = ? AND key = ?",
            rusqlite::params![library_id, &parent_key],
            |r| r.get(0),
        ).ok();
        let parent_id = match parent_id {
            Some(id) => id,
            None => return Ok(vec![]),
        };

        let mut out = vec![];
        let mut stmt = c.prepare(
            "SELECT i.key, a.linkMode, a.contentType, a.path
             FROM itemAttachments a JOIN items i ON i.itemID = a.itemID
             WHERE a.parentItemID = ? AND i.libraryID = ?"
        )?;
        let mut rows = stmt.query(rusqlite::params![parent_id, library_id])?;
        while let Some(r) = rows.next()? {
            let key: String = r.get(0)?;
            let link_mode = AttachmentLinkMode::from_i64(r.get(1)?);
            let content_type: Option<String> = r.get(2)?;
            let path_raw: Option<String> = r.get(3)?;
            let (filename, absolute_path) = resolve_filename(&storage_dir, &key, path_raw.as_deref(), link_mode);
            out.push(Attachment {
                key,
                parent_key: Some(parent_key.clone()),
                content_type,
                filename,
                absolute_path,
                link_mode,
            });
        }
        Ok(out)
    }).await
}

/// Returns the absolute path to the (first, preferred) attachment of an item.
pub async fn resolve_path(
    pool: &ReadOnlyPool,
    parent_key: &str,
    library_id: i64,
    storage_dir: &Path,
) -> Result<PathBuf> {
    let atts = list_attachments(pool, parent_key, library_id, storage_dir).await?;
    // Prefer PDFs first, then HTML snapshots
    let chosen = atts.iter().find(|a| a.content_type.as_deref() == Some("application/pdf"))
        .or_else(|| atts.iter().find(|a| a.content_type.as_deref() == Some("text/html")))
        .ok_or_else(|| Error::AttachmentNotFound(parent_key.to_string()))?;
    chosen.absolute_path.as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| Error::AttachmentNotFound(parent_key.to_string()))
}

fn resolve_filename(
    storage_dir: &Path,
    key: &str,
    path_raw: Option<&str>,
    link_mode: AttachmentLinkMode,
) -> (Option<String>, Option<String>) {
    // Zotero `path` formats:
    //   "storage:foo.pdf"    -> imported file, in storage/<key>/foo.pdf
    //   "attachments:foo.pdf" -> linked file, base-dir relative (out of scope for v1)
    //   absolute path        -> linked file
    //   null                 -> unknown
    let raw = match path_raw {
        Some(s) => s,
        None => return (None, None),
    };
    if let Some(name) = raw.strip_prefix("storage:") {
        let abs = storage_dir.join(key).join(name);
        let abs_str = abs.to_string_lossy().to_string();
        return (Some(name.to_string()), Some(abs_str));
    }
    if matches!(link_mode, AttachmentLinkMode::LinkedFile) {
        let p = std::path::Path::new(raw);
        let abs = if p.is_absolute() {
            Some(raw.to_string())
        } else {
            None // base-dir-relative linked files not supported in v1
        };
        let fname = p.file_name().map(|f| f.to_string_lossy().to_string());
        return (fname, abs);
    }
    (Some(raw.to_string()), None)
}
