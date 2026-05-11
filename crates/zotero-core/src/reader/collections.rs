use crate::error::Result;
use crate::reader::pool::ReadOnlyPool;
use crate::types::Collection;

pub async fn list(pool: &ReadOnlyPool, library_id: i64, parent: Option<String>) -> Result<Vec<Collection>> {
    pool.with_conn(move |c| {
        let mut out = vec![];
        let mut sql = String::from(
            "SELECT c.key, c.libraryID, c.collectionName, p.key
             FROM collections c
             LEFT JOIN collections p ON p.collectionID = c.parentCollectionID
             WHERE c.libraryID = ?"
        );
        if parent.is_some() {
            sql.push_str(" AND p.key = ?");
        }
        sql.push_str(" ORDER BY c.collectionName");
        let mut stmt = c.prepare(&sql)?;
        let mut rows = if let Some(p) = parent.as_deref() {
            stmt.query(rusqlite::params![library_id, p])?
        } else {
            stmt.query([library_id])?
        };
        while let Some(r) = rows.next()? {
            out.push(Collection {
                key: r.get(0)?,
                library_id: r.get(1)?,
                name: r.get(2)?,
                parent_key: r.get::<_, Option<String>>(3)?,
            });
        }
        Ok(out)
    }).await
}
