use crate::core::error::Result;
use crate::core::reader::pool::ReadOnlyPool;
use crate::core::types::Tag;

pub async fn list(pool: &ReadOnlyPool, library_id: i64, prefix: Option<String>) -> Result<Vec<Tag>> {
    pool.with_conn(move |c| {
        let like = prefix.as_ref().map(|p| format!("{}%", p));
        let mut out = vec![];
        let sql = if like.is_some() {
            "SELECT t.name, COUNT(it.itemID) AS n
             FROM tags t JOIN itemTags it ON it.tagID = t.tagID JOIN items i ON i.itemID = it.itemID
             WHERE i.libraryID = ? AND t.name LIKE ?
             GROUP BY t.name ORDER BY n DESC, t.name"
        } else {
            "SELECT t.name, COUNT(it.itemID) AS n
             FROM tags t JOIN itemTags it ON it.tagID = t.tagID JOIN items i ON i.itemID = it.itemID
             WHERE i.libraryID = ?
             GROUP BY t.name ORDER BY n DESC, t.name"
        };
        let mut stmt = c.prepare(sql)?;
        let mut rows = if let Some(l) = like.as_deref() {
            stmt.query(rusqlite::params![library_id, l])?
        } else {
            stmt.query([library_id])?
        };
        while let Some(r) = rows.next()? {
            out.push(Tag { name: r.get(0)?, item_count: r.get(1)? });
        }
        Ok(out)
    }).await
}
