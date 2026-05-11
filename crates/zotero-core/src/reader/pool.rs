use crate::error::{Error, Result};
use crate::reader::conn::open_read_only;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct ReadOnlyPool {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    sem: Semaphore,
}

impl ReadOnlyPool {
    pub async fn new(path: PathBuf, max: usize) -> Result<Self> {
        // Quick open to validate path / permissions.
        let _probe = open_read_only(&path)?;
        Ok(Self {
            inner: Arc::new(Inner {
                path,
                sem: Semaphore::new(max),
            }),
        })
    }

    pub async fn with_conn<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let _permit = self
            .inner
            .sem
            .acquire()
            .await
            .map_err(|e| Error::Pool(e.to_string()))?;
        let path = self.inner.path.clone();
        let r = tokio::task::spawn_blocking(move || {
            let conn = open_read_only(&path)?;
            f(&conn).map_err(Error::from)
        })
        .await
        .map_err(|e| Error::Pool(e.to_string()))?;
        r
    }
}
