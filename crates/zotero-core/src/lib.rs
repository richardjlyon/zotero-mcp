//! Library: read/write access to a local Zotero installation.

pub mod error;
pub mod types;
pub mod config;

pub use error::{Error, Result};
pub use config::Config;
