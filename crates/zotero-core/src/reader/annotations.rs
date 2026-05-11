use crate::error::Result;
use crate::reader::pool::ReadOnlyPool;
use crate::types::Annotation;

fn annotation_kind(t: i64) -> &'static str {
    // Zotero annotation types: 1=highlight 2=note 3=image 4=ink 5=underline
    match t {
        1 => "highlight",
        2 => "note",
        3 => "image",
        4 => "ink",
        5 => "underline",
        _ => "unknown",
    }
}

pub async fn list_annotations(pool: &ReadOnlyPool, parent_item_key: &str, library_id: i64) -> Result<Vec<Annotation>> {
    let key = parent_item_key.to_string();
    pool.with_conn(move |c| {
        let parent_id: Option<i64> = c.query_row(
            "SELECT itemID FROM items WHERE libraryID = ? AND key = ?",
            rusqlite::params![library_id, &key], |r| r.get(0)).ok();
        let Some(parent_id) = parent_id else { return Ok(vec![]) };

        // Find attachment items for the parent
        let mut attachment_ids = vec![];
        let mut stmt = c.prepare("SELECT itemID FROM itemAttachments WHERE parentItemID = ?")?;
        let mut rows = stmt.query([parent_id])?;
        while let Some(r) = rows.next()? { attachment_ids.push(r.get::<_, i64>(0)?); }

        let mut out = vec![];
        for aid in attachment_ids {
            let attachment_key: String = c.query_row(
                "SELECT key FROM items WHERE itemID = ?", [aid], |r| r.get(0))?;

            let mut stmt = c.prepare(
                "SELECT i.key, a.type, a.text, a.comment, a.color, a.pageLabel, a.sortIndex
                 FROM itemAnnotations a JOIN items i ON i.itemID = a.itemID
                 WHERE a.parentItemID = ? ORDER BY a.sortIndex")?;
            let mut rows = stmt.query([aid])?;
            while let Some(r) = rows.next()? {
                let kind = annotation_kind(r.get::<_, i64>(1)?).to_string();
                out.push(Annotation {
                    key: r.get(0)?,
                    parent_attachment_key: attachment_key.clone(),
                    kind,
                    text: r.get::<_, Option<String>>(2)?,
                    comment: r.get::<_, Option<String>>(3)?,
                    color: r.get::<_, Option<String>>(4)?,
                    page_label: r.get::<_, Option<String>>(5)?,
                    sort_index: r.get::<_, String>(6)?,
                });
            }
        }
        Ok(out)
    }).await
}
