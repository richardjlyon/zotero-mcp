use crate::core::error::Result;
use crate::core::reader::pool::ReadOnlyPool;

/// Lowercased-word match. Returns parent-item keys (i.e. the actual library
/// items, not the attachment items) for hits.
pub async fn fulltext_match_items(pool: &ReadOnlyPool, library_id: i64, word: &str) -> Result<Vec<String>> {
    let needle = word.to_lowercase();
    pool.with_conn(move |c| {
        let mut out = vec![];
        let mut stmt = c.prepare(
            "SELECT DISTINCT parent.key
             FROM fulltextWords fw
             JOIN fulltextItemWords fiw ON fiw.wordID = fw.wordID
             JOIN itemAttachments a ON a.itemID = fiw.itemID
             JOIN items parent ON parent.itemID = a.parentItemID
             WHERE parent.libraryID = ? AND fw.word = ?"
        )?;
        let mut rows = stmt.query(rusqlite::params![library_id, needle])?;
        while let Some(r) = rows.next()? { out.push(r.get::<_, String>(0)?); }
        Ok(out)
    }).await
}
