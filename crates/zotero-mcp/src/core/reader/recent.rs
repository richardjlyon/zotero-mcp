use crate::core::error::{Error, Result};
use crate::core::reader::pool::ReadOnlyPool;
use crate::core::types::SearchHit;

pub async fn list(pool: &ReadOnlyPool, library_id: i64, sort_by: &str, limit: i64) -> Result<Vec<SearchHit>> {
    let col = match sort_by {
        "dateAdded" => "i.dateAdded",
        "dateModified" => "i.dateModified",
        other => return Err(Error::Config(format!("sort_by must be dateAdded or dateModified, got {}", other))),
    };
    let sql = format!(
        "SELECT i.key, it.typeName,
                (SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = i.itemID AND d.fieldID = (SELECT fieldID FROM fieldsCombined WHERE fieldName='title')),
                (SELECT v.value FROM itemData d JOIN itemDataValues v ON v.valueID = d.valueID
                 WHERE d.itemID = i.itemID AND d.fieldID = (SELECT fieldID FROM fieldsCombined WHERE fieldName='date'))
         FROM items i JOIN itemTypes it ON it.itemTypeID = i.itemTypeID
         WHERE i.libraryID = ?
           AND it.typeName NOT IN ('attachment', 'note', 'annotation')
         ORDER BY {} DESC LIMIT ?",
        col
    );
    pool.with_conn(move |c| {
        let mut out = vec![];
        let mut stmt = c.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params![library_id, limit])?;
        while let Some(r) = rows.next()? {
            let year = r.get::<_, Option<String>>(3)?.and_then(|s| s.split('-').next().map(str::to_string));
            out.push(SearchHit {
                key: r.get(0)?,
                citation_key: None,
                item_type: r.get(1)?,
                title: r.get::<_, Option<String>>(2)?,
                creators_short: None,
                year,
                match_excerpt: None,
            });
        }
        Ok(out)
    }).await
}
