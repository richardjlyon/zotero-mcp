use crate::error::{Error, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

pub fn open_read_only(db: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // We rely on Zotero's WAL mode; do not change journaling.
    conn.busy_timeout(std::time::Duration::from_millis(500))?;
    Ok(conn)
}

pub fn check_schema(conn: &Connection, min_inclusive: i64, max_inclusive: i64) -> Result<i64> {
    let v: i64 = conn.query_row(
        "SELECT version FROM version WHERE schema = 'userdata'",
        [],
        |r| r.get(0),
    )?;
    if v < min_inclusive || v > max_inclusive {
        return Err(Error::SchemaMismatch {
            expected: format!("{}..={}", min_inclusive, max_inclusive),
            found: v,
        });
    }
    Ok(v)
}
