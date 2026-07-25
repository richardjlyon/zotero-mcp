//! Trash awareness for the reader layer.
//!
//! Zotero keeps trashed items in `items` and records the deletion in a separate
//! `deletedItems` table, so an ordinary query returns them. Nothing in the
//! reader filtered them before `find_duplicates`, which meant a trashed record
//! could be offered as a duplicate and trigger a spurious abort.

use crate::core::error::Result;
use crate::core::reader::pool::ReadOnlyPool;
use std::collections::HashSet;

/// Which of `keys` are in the trash. Unknown keys are simply absent from the
/// returned set.
///
/// A library whose schema predates `deletedItems`, or a trimmed test fixture
/// without the table, yields an empty set rather than an error: "we could not
/// tell" degrades to "nothing is trashed", which is the same answer the reader
/// gave before this function existed.
pub async fn trashed_keys(
    pool: &ReadOnlyPool,
    library_id: i64,
    keys: &[String],
) -> Result<HashSet<String>> {
    if keys.is_empty() {
        return Ok(HashSet::new());
    }
    let wanted: HashSet<String> = keys.iter().cloned().collect();
    pool.with_conn(move |c| {
        let mut stmt = match c.prepare(
            "SELECT i.key FROM deletedItems d
             JOIN items i ON i.itemID = d.itemID
             WHERE i.libraryID = ?",
        ) {
            Ok(s) => s,
            Err(_) => return Ok(HashSet::new()),
        };
        let mut rows = stmt.query(rusqlite::params![library_id])?;
        let mut out = HashSet::new();
        while let Some(r) = rows.next()? {
            let k: String = r.get(0)?;
            if wanted.contains(&k) {
                out.insert(k);
            }
        }
        Ok(out)
    })
    .await
}
