use crate::core::error::{Error, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

pub fn open_read_only(db: &Path) -> Result<Connection> {
    // Zotero keeps an open write transaction while running, which makes a plain
    // SQLITE_OPEN_READ_ONLY connection fail with "database is locked". Open via
    // URI with `mode=ro&nolock=1&immutable=1` so we get a stable read snapshot
    // without competing for filesystem locks. Each connection is short-lived
    // (one query per `with_conn` call), so re-opening picks up Zotero's latest
    // committed state.
    let uri = format!(
        "file:{}?mode=ro&nolock=1&immutable=1",
        db.to_string_lossy()
    );
    let conn = Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI,
    )?;
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
