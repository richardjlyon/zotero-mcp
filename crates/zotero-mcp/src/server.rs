use crate::state::AppState;
use crate::tools::attachments::{
    self as att, FirstPagesArgs, ItemKeyArgs as AttachItemKey, RefetchArgs, WebArgs,
};
use crate::tools::citations::{self as cit, FormatBibArgs, FormatCitationArgs};
use crate::tools::enrichment::{
    self as en, ApplyArgs, ArxivArgs, DoiArgs, EnrichArgs, IsbnArgs, ProposeArgs, SearchSourceArgs,
    WeakArgs,
};
use crate::tools::search::{self, EmptyArgs, GetItemArgs, ListTagsArgs, RecentArgs, SearchArgs};
use crate::tools::writes::{
    self as wr, AddNoteArgs, CollectionArgs, DeleteItemArgs, TagArgs, UpdateFieldsArgs,
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{
        AnnotateAble, CallToolResult, Content, Implementation, ListResourcesResult,
        PaginatedRequestParams, RawResource, ReadResourceRequestParams, ReadResourceResult,
        ResourceContents, ServerCapabilities, ServerInfo,
    },
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};

#[derive(Clone)]
pub struct ZoteroServer {
    pub state: AppState,
}

#[tool_router]
impl ZoteroServer {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    #[tool(
        description = "Liveness check; returns 'pong (v<version>, <git-sha>)' so callers can confirm which build is responding.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn ping(&self) -> Result<CallToolResult, McpError> {
        let msg = format!(
            "pong (v{}, {})",
            env!("CARGO_PKG_VERSION"),
            env!("ZOTERO_MCP_GIT_SHA"),
        );
        Ok(CallToolResult::success(vec![Content::text(msg)]))
    }

    #[tool(
        description = "Search the local Zotero library (metadata + optional fulltext).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn search_items(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        search::search_items(&self.state, args).await
    }

    #[tool(
        description = "Get a single Zotero item by key, with citation_key hydrated when BBT is available.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn get_item(
        &self,
        Parameters(args): Parameters<GetItemArgs>,
    ) -> Result<CallToolResult, McpError> {
        search::get_item(&self.state, args).await
    }

    #[tool(
        description = "List all collections in the user's library.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn list_collections(
        &self,
        Parameters(args): Parameters<EmptyArgs>,
    ) -> Result<CallToolResult, McpError> {
        search::list_collections(&self.state, args).await
    }

    #[tool(
        description = "List tags, optionally filtered by prefix.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn list_tags(
        &self,
        Parameters(args): Parameters<ListTagsArgs>,
    ) -> Result<CallToolResult, McpError> {
        search::list_tags(&self.state, args).await
    }

    #[tool(
        description = "List recently added or modified items, sorted by 'dateAdded' or 'dateModified'.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn list_recent_items(
        &self,
        Parameters(args): Parameters<RecentArgs>,
    ) -> Result<CallToolResult, McpError> {
        search::list_recent_items(&self.state, args).await
    }

    #[tool(
        description = "List file attachments and snapshots for an item, with resolved absolute paths.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn list_attachments(
        &self,
        Parameters(args): Parameters<AttachItemKey>,
    ) -> Result<CallToolResult, McpError> {
        att::list_attachments_t(&self.state, args).await
    }

    #[tool(
        description = "Get the absolute filesystem path to a Zotero attachment. For text extraction prefer get_pdf_text — it has built-in fallback to pdftotext on PDFs that trip pdf-extract. Use this path only when you need raw bytes (e.g. binary handling, OCR pipelines).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn get_pdf_path(
        &self,
        Parameters(args): Parameters<AttachItemKey>,
    ) -> Result<CallToolResult, McpError> {
        att::get_pdf_path(&self.state, args).await
    }

    #[tool(
        description = "Read full extracted PDF text for an item. Resolution order: .zotero-ft-cache → in-process pdf-extract → pdftotext fallback (automatic, when Poppler is on PATH). The `source` field on the response identifies which engine succeeded (zotero_cache | live_extract | pdftotext_fallback). Callers do not need to handle fallback manually — extraction is resilient to PDFs that trip pdf-extract.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn get_pdf_text(
        &self,
        Parameters(args): Parameters<AttachItemKey>,
    ) -> Result<CallToolResult, McpError> {
        att::get_pdf_text_t(&self.state, args).await
    }

    #[tool(
        description = "Read the first N pages of a PDF (default 2). Useful for cheap context grabs. Uses the same resilient extraction chain as get_pdf_text (cache → pdf-extract → pdftotext fallback).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn get_pdf_first_pages(
        &self,
        Parameters(args): Parameters<FirstPagesArgs>,
    ) -> Result<CallToolResult, McpError> {
        att::get_pdf_first_pages_t(&self.state, args).await
    }

    #[tool(
        description = "List PDF annotations (highlights, comments) for an item.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn list_annotations(
        &self,
        Parameters(args): Parameters<AttachItemKey>,
    ) -> Result<CallToolResult, McpError> {
        att::list_annotations_t(&self.state, args).await
    }

    #[tool(
        description = "Read webpage content for an item via stored snapshot or live fetch (mode = snapshot|live|auto).",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub async fn get_webpage_content(
        &self,
        Parameters(args): Parameters<WebArgs>,
    ) -> Result<CallToolResult, McpError> {
        att::get_webpage_content_t(&self.state, args).await
    }

    #[tool(
        description = "Re-fetch a webpage item live, optionally saving a fresh HTML snapshot as an attachment.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true,
        )
    )]
    pub async fn refetch_url(
        &self,
        Parameters(args): Parameters<RefetchArgs>,
    ) -> Result<CallToolResult, McpError> {
        att::refetch_url_t(&self.state, args).await
    }

    #[tool(
        description = "Format a single Zotero item as a citation (style = CSL name, e.g. 'apa', 'chicago-author-date'; format = 'bib'|'biblatex'|'bibtex'|'ris').",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn format_citation(
        &self,
        Parameters(args): Parameters<FormatCitationArgs>,
    ) -> Result<CallToolResult, McpError> {
        cit::format_citation_t(&self.state, args).await
    }

    #[tool(
        description = "Format multiple Zotero items as a combined bibliography (same style/format options as format_citation).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn format_bibliography(
        &self,
        Parameters(args): Parameters<FormatBibArgs>,
    ) -> Result<CallToolResult, McpError> {
        cit::format_bibliography_t(&self.state, args).await
    }

    #[tool(
        description = "Attach a markdown/HTML note to a Zotero item (markdown converted to simple HTML).",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false,
        )
    )]
    pub async fn add_note(
        &self,
        Parameters(args): Parameters<AddNoteArgs>,
    ) -> Result<CallToolResult, McpError> {
        wr::add_note_t(&self.state, args).await
    }

    #[tool(
        description = "Patch arbitrary fields on an item (auto-detects current version for If-Unmodified-Since-Version).",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false,
        )
    )]
    pub async fn update_item_fields(
        &self,
        Parameters(args): Parameters<UpdateFieldsArgs>,
    ) -> Result<CallToolResult, McpError> {
        wr::update_item_fields_t(&self.state, args).await
    }

    #[tool(
        description = "Add tags to an item (deduplicates against existing tags).",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false,
        )
    )]
    pub async fn add_tags(
        &self,
        Parameters(args): Parameters<TagArgs>,
    ) -> Result<CallToolResult, McpError> {
        wr::add_tags_t(&self.state, args).await
    }

    #[tool(
        description = "Remove tags from an item.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false,
        )
    )]
    pub async fn remove_tags(
        &self,
        Parameters(args): Parameters<TagArgs>,
    ) -> Result<CallToolResult, McpError> {
        wr::remove_tags_t(&self.state, args).await
    }

    #[tool(
        description = "Add an item to a collection (by collection key).",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false,
        )
    )]
    pub async fn add_to_collection(
        &self,
        Parameters(args): Parameters<CollectionArgs>,
    ) -> Result<CallToolResult, McpError> {
        wr::add_to_collection_t(&self.state, args).await
    }

    #[tool(
        description = "Remove an item from a collection (by collection key).",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false,
        )
    )]
    pub async fn remove_from_collection(
        &self,
        Parameters(args): Parameters<CollectionArgs>,
    ) -> Result<CallToolResult, McpError> {
        wr::remove_from_collection_t(&self.state, args).await
    }

    #[tool(
        description = "Move an item (regular item, note, or attachment) to Zotero's trash. \
                       Recoverable: items remain in the library until the trash is emptied. \
                       Use this when the user explicitly asks to delete something.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false,
        )
    )]
    pub async fn delete_item(
        &self,
        Parameters(args): Parameters<DeleteItemArgs>,
    ) -> Result<CallToolResult, McpError> {
        wr::delete_item_t(&self.state, args).await
    }

    #[tool(
        description = "Create a new Zotero item. Input: { item: <Zotero-shaped JSON object with required itemType field>, collection_keys?: [string] }. Returns { item_key, version }. Tags are an array of objects: [{\"tag\": \"x\"}]. Creators use Zotero's creatorType vocabulary (author/editor/translator/etc). For metadata-discovery flows, lookup_doi / search_crossref return the JSON shape directly compatible with this tool.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false,
        )
    )]
    pub async fn create_item(
        &self,
        Parameters(args): Parameters<att::CreateItemArgs>,
    ) -> Result<CallToolResult, McpError> {
        att::create_item_t(&self.state, args).await
    }

    #[tool(
        description = "Attach a URL as a child of a Zotero item (linkMode: linked_url). No bytes transfer; Zotero stores just the URL. Use this for online resources you want listed alongside an item without downloading them. Input: { parent_key, url, title? }. Returns { attachment_key }.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false,
        )
    )]
    pub async fn attach_link(
        &self,
        Parameters(args): Parameters<att::AttachLinkArgs>,
    ) -> Result<CallToolResult, McpError> {
        att::attach_link_t(&self.state, args).await
    }

    #[tool(
        description = "Attach a local file to a Zotero parent item. Two storage modes: \"imported_file\" (bytes uploaded to Zotero's cloud and downloaded locally on each device — Zotero's default) or \"linked_file\" (Zotero stores only a path reference; the file lives wherever you put it — useful for BYO-storage setups like Resilio/Syncthing). Default mode comes from cfg.zotero.attachment_mode; per-call override allowed. For linked_file, the file must be under cfg.zotero.linked_attachment_base_dir. Input: { parent_key, file_path (absolute), mode?, filename?, content_type? }. Returns { attachment_key }.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false,
        )
    )]
    pub async fn attach_file(
        &self,
        Parameters(args): Parameters<att::AttachFileArgs>,
    ) -> Result<CallToolResult, McpError> {
        att::attach_file_t(&self.state, args).await
    }

    #[tool(
        description = "Find items with weak metadata (missing DOI/abstract, stub titles).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn find_weak_metadata_items(
        &self,
        Parameters(args): Parameters<WeakArgs>,
    ) -> Result<CallToolResult, McpError> {
        en::find_weak_metadata_items_t(&self.state, args).await
    }

    #[tool(
        description = "Look up a DOI via CrossRef. \
                          `format='zotero'` (default) returns a flat Zotero item ready to pass straight to `create_item`; \
                          `format='candidate'` returns an envelope `{source, fields, creators, source_url}` for use with `propose_metadata_update` and `enrich_item`.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub async fn lookup_doi(
        &self,
        Parameters(args): Parameters<DoiArgs>,
    ) -> Result<CallToolResult, McpError> {
        en::lookup_doi_t(&self.state, args).await
    }

    #[tool(
        description = "Look up an ISBN via OpenLibrary. \
                          `format='zotero'` (default) returns a flat Zotero item ready to pass straight to `create_item`; \
                          `format='candidate'` returns an envelope `{source, fields, creators, source_url}` for use with `propose_metadata_update` and `enrich_item`.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub async fn lookup_isbn(
        &self,
        Parameters(args): Parameters<IsbnArgs>,
    ) -> Result<CallToolResult, McpError> {
        en::lookup_isbn_t(&self.state, args).await
    }

    #[tool(
        description = "Look up an arXiv preprint by ID. \
                          `format='zotero'` (default) returns a flat Zotero item ready to pass straight to `create_item`; \
                          `format='candidate'` returns an envelope `{source, fields, creators, source_url}` for use with `propose_metadata_update` and `enrich_item`.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub async fn lookup_arxiv(
        &self,
        Parameters(args): Parameters<ArxivArgs>,
    ) -> Result<CallToolResult, McpError> {
        en::lookup_arxiv_t(&self.state, args).await
    }

    #[tool(
        description = "Search CrossRef by free-text query; returns normalized candidates.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub async fn search_crossref(
        &self,
        Parameters(args): Parameters<SearchSourceArgs>,
    ) -> Result<CallToolResult, McpError> {
        en::search_crossref_t(&self.state, args).await
    }

    #[tool(
        description = "Search Semantic Scholar by free-text query; returns normalized candidates.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub async fn search_semantic_scholar(
        &self,
        Parameters(args): Parameters<SearchSourceArgs>,
    ) -> Result<CallToolResult, McpError> {
        en::search_semantic_scholar_t(&self.state, args).await
    }

    #[tool(
        description = "Score candidate metadata and produce an EnrichmentProposal (does not apply). \
                          Candidates must be lookup results obtained with `format='candidate'`. \
                          Items obtained with the default `format='zotero'` will fail validation because the scoring logic requires the envelope's `source` field.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn propose_metadata_update(
        &self,
        Parameters(args): Parameters<ProposeArgs>,
    ) -> Result<CallToolResult, McpError> {
        en::propose_metadata_update_t(&self.state, args).await
    }

    #[tool(
        description = "Apply a previously generated EnrichmentProposal to Zotero.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false,
        )
    )]
    pub async fn apply_metadata_update(
        &self,
        Parameters(args): Parameters<ApplyArgs>,
    ) -> Result<CallToolResult, McpError> {
        en::apply_metadata_update_t(&self.state, args).await
    }

    #[tool(
        description = "Compose propose+apply: only auto-applies when confidence >= threshold AND multi-source agreement. \
                          Candidates must be lookup results obtained with `format='candidate'`. \
                          Items obtained with the default `format='zotero'` will fail validation because the scoring logic requires the envelope's `source` field.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true,
        )
    )]
    pub async fn enrich_item(
        &self,
        Parameters(args): Parameters<EnrichArgs>,
    ) -> Result<CallToolResult, McpError> {
        en::enrich_item_t(&self.state, args).await
    }
}

#[tool_handler]
impl ServerHandler for ZoteroServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("zotero-mcp", env!("CARGO_PKG_VERSION")))
        .with_instructions("Local Zotero library bridge (read + write via Local API)")
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let make = |uri: &str, name: &str, desc: &str| {
            let mut raw = RawResource::new(uri, name);
            raw.description = Some(desc.to_string());
            raw.mime_type = Some("application/json".to_string());
            raw.no_annotation()
        };
        Ok(ListResourcesResult::with_all_items(vec![
            make(
                "zotero://collections",
                "Zotero collections",
                "All collections in the user's library",
            ),
            make(
                "zotero://tags",
                "Zotero tags",
                "All tags in the user's library with counts",
            ),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let body = match request.uri.as_str() {
            "zotero://collections" => crate::resources::collections::read_all(&self.state).await,
            "zotero://tags" => crate::resources::tags::read_all(&self.state).await,
            other => {
                return Err(McpError::invalid_params(
                    format!("unknown resource uri: {}", other),
                    None,
                ))
            }
        };
        let text = body.map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            text,
            request.uri,
        )]))
    }
}

#[cfg(test)]
mod tests {
    use super::ZoteroServer;

    #[test]
    fn tool_annotations_present_on_representative_tools() {
        let ann = ZoteroServer::ping_tool_attr()
            .annotations
            .expect("ping should carry annotations");
        assert_eq!(ann.read_only_hint, Some(true));

        let ann = ZoteroServer::delete_item_tool_attr()
            .annotations
            .expect("delete_item should carry annotations");
        assert_eq!(ann.destructive_hint, Some(true));
        assert_eq!(ann.idempotent_hint, Some(true));

        let ann = ZoteroServer::lookup_doi_tool_attr()
            .annotations
            .expect("lookup_doi should carry annotations");
        assert_eq!(ann.open_world_hint, Some(true));

        let ann = ZoteroServer::add_tags_tool_attr()
            .annotations
            .expect("add_tags should carry annotations");
        assert_eq!(ann.idempotent_hint, Some(true));
    }
}
