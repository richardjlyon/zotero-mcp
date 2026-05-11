use crate::state::AppState;
use crate::tools::attachments::{self as att, FirstPagesArgs, ItemKeyArgs as AttachItemKey, RefetchArgs, WebArgs};
use crate::tools::citations::{self as cit, FormatBibArgs, FormatCitationArgs};
use crate::tools::enrichment::{
    self as en, ApplyArgs, ArxivArgs, DoiArgs, EnrichArgs, IsbnArgs, ProposeArgs,
    SearchSourceArgs, WeakArgs,
};
use crate::tools::search::{self, EmptyArgs, GetItemArgs, ListTagsArgs, RecentArgs, SearchArgs};
use crate::tools::writes::{self as wr, AddNoteArgs, CollectionArgs, TagArgs, UpdateFieldsArgs};
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

    #[tool(description = "List file attachments and snapshots for an item, with resolved absolute paths.")]
    pub async fn list_attachments(
        &self,
        #[tool(aggr)] args: AttachItemKey,
    ) -> Result<CallToolResult, McpError> {
        att::list_attachments_t(&self.state, args).await
    }

    #[tool(description = "Get the absolute filesystem path to a Zotero attachment.")]
    pub async fn get_pdf_path(
        &self,
        #[tool(aggr)] args: AttachItemKey,
    ) -> Result<CallToolResult, McpError> {
        att::get_pdf_path(&self.state, args).await
    }

    #[tool(description = "Read full extracted PDF text for an item (uses Zotero's .zotero-ft-cache when present).")]
    pub async fn get_pdf_text(
        &self,
        #[tool(aggr)] args: AttachItemKey,
    ) -> Result<CallToolResult, McpError> {
        att::get_pdf_text_t(&self.state, args).await
    }

    #[tool(description = "Read the first N pages of a PDF (default 2). Useful for cheap context grabs.")]
    pub async fn get_pdf_first_pages(
        &self,
        #[tool(aggr)] args: FirstPagesArgs,
    ) -> Result<CallToolResult, McpError> {
        att::get_pdf_first_pages_t(&self.state, args).await
    }

    #[tool(description = "List PDF annotations (highlights, comments) for an item.")]
    pub async fn list_annotations(
        &self,
        #[tool(aggr)] args: AttachItemKey,
    ) -> Result<CallToolResult, McpError> {
        att::list_annotations_t(&self.state, args).await
    }

    #[tool(description = "Read webpage content for an item via stored snapshot or live fetch (mode = snapshot|live|auto).")]
    pub async fn get_webpage_content(
        &self,
        #[tool(aggr)] args: WebArgs,
    ) -> Result<CallToolResult, McpError> {
        att::get_webpage_content_t(&self.state, args).await
    }

    #[tool(description = "Re-fetch a webpage item live, optionally saving a fresh HTML snapshot as an attachment.")]
    pub async fn refetch_url(
        &self,
        #[tool(aggr)] args: RefetchArgs,
    ) -> Result<CallToolResult, McpError> {
        att::refetch_url_t(&self.state, args).await
    }

    #[tool(description = "Format a single Zotero item as a citation (style = CSL name, e.g. 'apa', 'chicago-author-date'; format = 'bib'|'biblatex'|'bibtex'|'ris').")]
    pub async fn format_citation(
        &self,
        #[tool(aggr)] args: FormatCitationArgs,
    ) -> Result<CallToolResult, McpError> {
        cit::format_citation_t(&self.state, args).await
    }

    #[tool(description = "Format multiple Zotero items as a combined bibliography (same style/format options as format_citation).")]
    pub async fn format_bibliography(
        &self,
        #[tool(aggr)] args: FormatBibArgs,
    ) -> Result<CallToolResult, McpError> {
        cit::format_bibliography_t(&self.state, args).await
    }

    #[tool(description = "Attach a markdown/HTML note to a Zotero item (markdown converted to simple HTML).")]
    pub async fn add_note(&self, #[tool(aggr)] args: AddNoteArgs) -> Result<CallToolResult, McpError> {
        wr::add_note_t(&self.state, args).await
    }

    #[tool(description = "Patch arbitrary fields on an item (auto-detects current version for If-Unmodified-Since-Version).")]
    pub async fn update_item_fields(&self, #[tool(aggr)] args: UpdateFieldsArgs) -> Result<CallToolResult, McpError> {
        wr::update_item_fields_t(&self.state, args).await
    }

    #[tool(description = "Add tags to an item (deduplicates against existing tags).")]
    pub async fn add_tags(&self, #[tool(aggr)] args: TagArgs) -> Result<CallToolResult, McpError> {
        wr::add_tags_t(&self.state, args).await
    }

    #[tool(description = "Remove tags from an item.")]
    pub async fn remove_tags(&self, #[tool(aggr)] args: TagArgs) -> Result<CallToolResult, McpError> {
        wr::remove_tags_t(&self.state, args).await
    }

    #[tool(description = "Add an item to a collection (by collection key).")]
    pub async fn add_to_collection(&self, #[tool(aggr)] args: CollectionArgs) -> Result<CallToolResult, McpError> {
        wr::add_to_collection_t(&self.state, args).await
    }

    #[tool(description = "Remove an item from a collection (by collection key).")]
    pub async fn remove_from_collection(&self, #[tool(aggr)] args: CollectionArgs) -> Result<CallToolResult, McpError> {
        wr::remove_from_collection_t(&self.state, args).await
    }

    #[tool(description = "Find items with weak metadata (missing DOI/abstract, stub titles).")]
    pub async fn find_weak_metadata_items(&self, #[tool(aggr)] args: WeakArgs) -> Result<CallToolResult, McpError> {
        en::find_weak_metadata_items_t(&self.state, args).await
    }

    #[tool(description = "Look up a DOI via CrossRef and return Zotero-shaped metadata.")]
    pub async fn lookup_doi(&self, #[tool(aggr)] args: DoiArgs) -> Result<CallToolResult, McpError> {
        en::lookup_doi_t(&self.state, args).await
    }

    #[tool(description = "Look up an ISBN via OpenLibrary and return Zotero-shaped metadata.")]
    pub async fn lookup_isbn(&self, #[tool(aggr)] args: IsbnArgs) -> Result<CallToolResult, McpError> {
        en::lookup_isbn_t(&self.state, args).await
    }

    #[tool(description = "Look up an arXiv preprint by ID and return Zotero-shaped metadata.")]
    pub async fn lookup_arxiv(&self, #[tool(aggr)] args: ArxivArgs) -> Result<CallToolResult, McpError> {
        en::lookup_arxiv_t(&self.state, args).await
    }

    #[tool(description = "Search CrossRef by free-text query; returns normalized candidates.")]
    pub async fn search_crossref(&self, #[tool(aggr)] args: SearchSourceArgs) -> Result<CallToolResult, McpError> {
        en::search_crossref_t(&self.state, args).await
    }

    #[tool(description = "Search Semantic Scholar by free-text query; returns normalized candidates.")]
    pub async fn search_semantic_scholar(&self, #[tool(aggr)] args: SearchSourceArgs) -> Result<CallToolResult, McpError> {
        en::search_semantic_scholar_t(&self.state, args).await
    }

    #[tool(description = "Score candidate metadata and produce an EnrichmentProposal (does not apply).")]
    pub async fn propose_metadata_update(&self, #[tool(aggr)] args: ProposeArgs) -> Result<CallToolResult, McpError> {
        en::propose_metadata_update_t(&self.state, args).await
    }

    #[tool(description = "Apply a previously generated EnrichmentProposal to Zotero.")]
    pub async fn apply_metadata_update(&self, #[tool(aggr)] args: ApplyArgs) -> Result<CallToolResult, McpError> {
        en::apply_metadata_update_t(&self.state, args).await
    }

    #[tool(description = "Compose propose+apply: only auto-applies when confidence >= threshold AND multi-source agreement.")]
    pub async fn enrich_item(&self, #[tool(aggr)] args: EnrichArgs) -> Result<CallToolResult, McpError> {
        en::enrich_item_t(&self.state, args).await
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
