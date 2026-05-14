//! Library: read/write access to a local Zotero installation.

pub mod bbt;
pub mod cache;
pub mod citations;
pub mod config;
pub mod enrichment;
pub mod error;
pub mod pdf;
pub mod reader;
pub mod types;
pub mod web;
pub mod writer;

pub use config::Config;
pub use error::{Error, Result};
