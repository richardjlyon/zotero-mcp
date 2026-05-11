//! Library: read/write access to a local Zotero installation.

pub mod error;
pub mod types;
pub mod config;
pub mod reader;
pub mod pdf;
pub mod web;
pub mod bbt;
pub mod writer;
pub mod citations;

pub use error::{Error, Result};
pub use config::Config;
