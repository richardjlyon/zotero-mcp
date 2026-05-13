//! `zotero-mcp` library surface.
//!
//! Most consumers of this crate are interacting via the bundled binary
//! (`zotero-mcp` on the command line). The library form exists so the
//! integration test suite — and any future Rust caller — can reach the
//! pieces that drive the server: the low-level Zotero readers/writers
//! (`core`), the MCP service implementation (`server`, `tools`,
//! `resources`), the HTTP/SSE transport (`http_transport`), and the
//! OAuth 2.1 surface (`oauth`).

pub mod bearer;
pub mod core;
pub mod http_transport;
pub mod logging;
pub mod oauth;
pub mod resources;
pub mod server;
pub mod setup;
pub mod state;
pub mod tools;
