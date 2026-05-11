use crate::state::AppState;
use crate::tools::search::{self, EmptyArgs, GetItemArgs, ListTagsArgs, RecentArgs, SearchArgs};
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

    #[tool(description = "Search the local Zotero library (metadata + optional fulltext).")]
    pub async fn search_items(
        &self,
        #[tool(aggr)] args: SearchArgs,
    ) -> Result<CallToolResult, McpError> {
        search::search_items(&self.state, args).await
    }

    #[tool(
        description = "Get a single Zotero item by key, with citation_key hydrated when BBT is available."
    )]
    pub async fn get_item(
        &self,
        #[tool(aggr)] args: GetItemArgs,
    ) -> Result<CallToolResult, McpError> {
        search::get_item(&self.state, args).await
    }

    #[tool(description = "List all collections in the user's library.")]
    pub async fn list_collections(
        &self,
        #[tool(aggr)] args: EmptyArgs,
    ) -> Result<CallToolResult, McpError> {
        search::list_collections(&self.state, args).await
    }

    #[tool(description = "List tags, optionally filtered by prefix.")]
    pub async fn list_tags(
        &self,
        #[tool(aggr)] args: ListTagsArgs,
    ) -> Result<CallToolResult, McpError> {
        search::list_tags(&self.state, args).await
    }

    #[tool(
        description = "List recently added or modified items, sorted by 'dateAdded' or 'dateModified'."
    )]
    pub async fn list_recent_items(
        &self,
        #[tool(aggr)] args: RecentArgs,
    ) -> Result<CallToolResult, McpError> {
        search::list_recent_items(&self.state, args).await
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
