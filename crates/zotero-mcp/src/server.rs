use crate::state::AppState;
use rmcp::{
    Error as McpError, ServerHandler,
    model::{
        CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
    },
    tool,
};

#[derive(Clone)]
pub struct ZoteroServer {
    pub state: AppState,
}

#[tool(tool_box)]
impl ZoteroServer {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    #[tool(description = "Liveness check; returns 'pong'.")]
    pub async fn ping(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text("pong")]))
    }
}

#[tool(tool_box)]
impl ServerHandler for ZoteroServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            server_info: Implementation {
                name: "zotero-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            instructions: Some(
                "Local Zotero library bridge (read + write via Local API)".into(),
            ),
        }
    }
}
