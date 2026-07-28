use crate::core::dedup::FindDuplicatesResult;
use crate::core::enrichment::propose::WeakMetadataItem;
use crate::core::enrichment::NormalizedRecord;
use crate::core::pdf::PdfTextResult;
use crate::core::types::{
    Annotation, Attachment, Collection, EnrichmentProposal, Item, SearchHit, Tag,
};
use crate::core::web::{RefetchResult, WebContentResult};
use crate::state::AppState;
use crate::tools::attachments::{
    self as att, FirstPagesArgs, ItemKeyArgs as AttachItemKey, PdfTextArgs, RefetchArgs, WebArgs,
};
use crate::tools::citations::{self as cit, FormatBibArgs, FormatCitationArgs};
use crate::tools::dedup;
use crate::tools::enrichment::{
    self as en, ApplyArgs, ArxivArgs, DoiArgs, EnrichArgs, IsbnArgs, ProposeArgs, SearchSourceArgs,
    WeakArgs,
};
use crate::tools::search::{self, EmptyArgs, GetItemArgs, ListTagsArgs, RecentArgs, SearchArgs};
use crate::tools::wire::ListResult;
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
    tool, tool_handler, tool_router, ErrorData as McpError, Json, RoleServer, ServerHandler,
};

#[derive(Clone)]
pub struct ZoteroServer {
    pub state: AppState,
}

// `vis = "pub"` so tests can build the router and walk every tool's schema.
// Building it is what validates output schemas — rmcp panics here on a schema
// without an object root, so this is the guard against shipping a tool list
// that a client will reject wholesale (see tests/tool_surface.rs).
#[tool_router(vis = "pub")]
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
        description = "Search the local Zotero library (metadata + optional fulltext). FULL-TEXT COVERAGE — read this before treating a miss as evidence of absence. The index searched is Zotero's own, built by Zotero's indexer; this server never writes it, and it does NOT cover the layout-aware markdown derivatives that get_pdf_text stores, so a term that appears only inside a table may be in the document and not in the index. The full-text clause applies to single-word queries only: a query containing whitespace is matched against metadata alone, with full-text matching dropped entirely rather than degraded. Matching resolves through parent items, so a top-level attachment with no parent is never returned by a full-text match. A negative result therefore means 'not found in Zotero's flat word index for a single-word query on a parented item' — not 'not in the document'. To search a document's real content, read its text with get_pdf_text.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn search_items(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<Json<ListResult<SearchHit>>, McpError> {
        search::search_items(&self.state, args).await
    }

    #[tool(
        description = "Check whether a work is ALREADY in the library, before creating an item. \
                       Call this first whenever you are about to add a reference. Runs three \
                       passes over the local library — individual title words, author surname, \
                       and identifier (DOI/ISBN/arXiv, in every plausible form) — unions the \
                       results, excludes trashed items, and returns a triage per candidate. \
                       Input: { title?, author_surname?, identifier?, input_kind: \
                       \"pdf\"|\"url\"|\"name\", limit? }; at least one of title / \
                       author_surname / identifier is required, and supplying both a title and \
                       a surname catches more than either alone. The `recommendation` is one \
                       of: \"abort\" (already there with the same kind of attachment), \
                       \"attach_to_existing\" (the record exists but lacks your file — see \
                       metadata_diff for what else it is missing), \"ask\" (weak match: put it \
                       to the user), \"create_new\" (nothing found). `queries_run` reports \
                       every query and its row count, so you can show your working.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn find_duplicates(
        &self,
        Parameters(args): Parameters<dedup::FindDuplicatesArgs>,
    ) -> Result<Json<FindDuplicatesResult>, McpError> {
        dedup::find_duplicates_t(&self.state, args).await
    }

    #[tool(
        description = "Get a single Zotero item by key, with citation_key hydrated when BBT is available.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn get_item(
        &self,
        Parameters(args): Parameters<GetItemArgs>,
    ) -> Result<Json<Item>, McpError> {
        search::get_item(&self.state, args).await
    }

    #[tool(
        description = "List all collections in the user's library.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn list_collections(
        &self,
        Parameters(args): Parameters<EmptyArgs>,
    ) -> Result<Json<ListResult<Collection>>, McpError> {
        search::list_collections(&self.state, args).await
    }

    #[tool(
        description = "List tags, optionally filtered by prefix.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn list_tags(
        &self,
        Parameters(args): Parameters<ListTagsArgs>,
    ) -> Result<Json<ListResult<Tag>>, McpError> {
        search::list_tags(&self.state, args).await
    }

    #[tool(
        description = "List recently added or modified items, sorted by 'dateAdded' or 'dateModified'.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn list_recent_items(
        &self,
        Parameters(args): Parameters<RecentArgs>,
    ) -> Result<Json<ListResult<SearchHit>>, McpError> {
        search::list_recent_items(&self.state, args).await
    }

    #[tool(
        description = "List file attachments and snapshots for an item, with resolved absolute paths.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn list_attachments(
        &self,
        Parameters(args): Parameters<AttachItemKey>,
    ) -> Result<Json<ListResult<Attachment>>, McpError> {
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
        description = "Read full extracted PDF text for an item. Primary route: the layout-aware Docling service — markdown output with real tables, `--- p.N ---` page anchors, and formulas decoded to LaTeX; scanned (image-only) PDFs get an ocrmypdf OCR pre-step. Falls back to the flat-text chain (.zotero-ft-cache → pdf-extract → pdftotext) when the service is unreachable; `source` identifies which engine produced the text. Layout-faithful extractions are stored durably and served back on later reads, so the expensive work happens once per PDF rather than once per reader: `served_from` says `store` or `fresh`, and a stored copy is reused until the PDF's content or the extraction profile changes (`refresh=true` forces a rebuild). Flat-text output is never stored. Every result carries a machine-readable `completeness` report (pages, per-page chars, undecoded formulas, untranscribed images, OCR'd pages, low-text pages): trust presence in the text, but treat absence on pages with declared drops as unknown — never as 'not in the document'. Set `plain=true` to force the old flat-text output. IMPORTANT — when a layout route is configured on the server but is unavailable (cold or down), this call FAILS rather than silently returning flat text: a flat engine drops every table while returning a similar volume of prose, and a caller reading only the text cannot tell. Pass `allow_degraded=true` to accept table-free text for that call (still labelled, still marked incomplete). On a server with no layout route configured at all, flat extraction succeeds as it always did and `allow_degraded` is irrelevant. Fails loudly when no route can extract text; never returns empty text as success.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub async fn get_pdf_text(
        &self,
        Parameters(args): Parameters<PdfTextArgs>,
    ) -> Result<Json<PdfTextResult>, McpError> {
        att::get_pdf_text_t(&self.state, args).await
    }

    #[tool(
        description = "Read the first N pages of a PDF (default 2). Useful for cheap context grabs. Same extraction contract as get_pdf_text: layout-aware markdown by default (Docling route, completeness report, OCR pre-step for scans), served from the durable derivative store when one exists, `plain=true` for the old flat-text output, `allow_degraded=true` to accept flat text when a configured layout route is down (without it, that case is an error rather than a quiet downgrade), loud failure when nothing extracts.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub async fn get_pdf_first_pages(
        &self,
        Parameters(args): Parameters<FirstPagesArgs>,
    ) -> Result<Json<PdfTextResult>, McpError> {
        att::get_pdf_first_pages_t(&self.state, args).await
    }

    #[tool(
        description = "Get the filesystem path of an item's stored PDF text derivative — the markdown produced by the layout-aware route — WITHOUT returning the text. Use this to hand a whole document to another tool, a script or a person: the response size is the same for a 2-page paper and a 69-page report, so it does not cost the document's tokens. `status` is `present`, `absent` (nothing built yet — read the text once, or run build_derivatives), `failed` (a build was attempted and the reason is in `detail`), or `no_pdf`. IMPORTANT: paths are on the machine running this server. If you are connected over HTTP rather than stdio, the path will not exist on your own filesystem.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn get_derivative_path(
        &self,
        Parameters(args): Parameters<AttachItemKey>,
    ) -> Result<Json<att::DerivativePathResult>, McpError> {
        att::get_derivative_path_t(&self.state, args).await
    }

    #[tool(
        description = "Build durable text derivatives for a set of items (backfill). Extracts each item's PDF once via the layout-aware route and stores the markdown, so later reads of those items are served from storage instead of re-extracting. Writes NOTHING to your Zotero library — item metadata, `extra` fields, tags, collections and notes are untouched — because the store is server-owned and lives outside the Zotero tree. Resumable: items that already have a current derivative are reported as `already_present` and skipped. Returns per-item outcomes (`stored`, `already_present`, `no_pdf`, `not_layout_faithful`, `failed`) and counts only, never text. Items whose extraction falls back to a flat-text engine are reported `not_layout_faithful` and nothing is stored for them, since flat text cannot express tables.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    pub async fn build_derivatives(
        &self,
        Parameters(args): Parameters<att::BuildDerivativesArgs>,
    ) -> Result<Json<att::BuildDerivativesResult>, McpError> {
        att::build_derivatives_t(&self.state, args).await
    }

    #[tool(
        description = "List PDF annotations (highlights, comments) for an item.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn list_annotations(
        &self,
        Parameters(args): Parameters<AttachItemKey>,
    ) -> Result<Json<ListResult<Annotation>>, McpError> {
        att::list_annotations_t(&self.state, args).await
    }

    #[tool(
        description = "Read webpage content for an item via stored snapshot or live fetch (mode = snapshot|live|auto).",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub async fn get_webpage_content(
        &self,
        Parameters(args): Parameters<WebArgs>,
    ) -> Result<Json<WebContentResult>, McpError> {
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
    ) -> Result<Json<RefetchResult>, McpError> {
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
    ) -> Result<Json<att::CreateItemResult>, McpError> {
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
    ) -> Result<Json<att::AttachmentResult>, McpError> {
        att::attach_link_t(&self.state, args).await
    }

    #[tool(
        description = "Attach a local file to a Zotero parent item. The file is stored the way Zotero's own UI would store it, and the user's Zotero file-sync preference (cloud, WebDAV, or none) decides where the bytes travel from there — that is not this server's decision to make. Input: { parent_key, file_path (absolute), mode?, filename?, content_type? }. `mode` is an advanced escape hatch: omit it. Pass \"linked_file\" only if the user specifically wants Zotero to store a path reference instead of the file (BYO-storage setups like a Calibre mirror or a shared NAS). Returns { attachment_key }.",
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
    ) -> Result<Json<att::AttachmentResult>, McpError> {
        att::attach_file_t(&self.state, args).await
    }

    #[tool(
        description = "Find items with weak metadata (missing DOI/abstract, stub titles).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn find_weak_metadata_items(
        &self,
        Parameters(args): Parameters<WeakArgs>,
    ) -> Result<Json<ListResult<WeakMetadataItem>>, McpError> {
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
    ) -> Result<Json<ListResult<NormalizedRecord>>, McpError> {
        en::search_crossref_t(&self.state, args).await
    }

    #[tool(
        description = "Search Semantic Scholar by free-text query; returns normalized candidates.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub async fn search_semantic_scholar(
        &self,
        Parameters(args): Parameters<SearchSourceArgs>,
    ) -> Result<Json<ListResult<NormalizedRecord>>, McpError> {
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
    ) -> Result<Json<EnrichmentProposal>, McpError> {
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
    ) -> Result<Json<EnrichmentProposal>, McpError> {
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

        // Both PDF tools now reach the external Docling service.
        let ann = ZoteroServer::get_pdf_text_tool_attr()
            .annotations
            .expect("get_pdf_text should carry annotations");
        assert_eq!(ann.read_only_hint, Some(true));
        assert_eq!(ann.open_world_hint, Some(true));

        let ann = ZoteroServer::get_pdf_first_pages_tool_attr()
            .annotations
            .expect("get_pdf_first_pages should carry annotations");
        assert_eq!(ann.open_world_hint, Some(true));
    }

    #[test]
    fn output_schemas_emitted_for_json_returning_tools() {
        // Json<T> tools get an output schema auto-generated by the #[tool] macro.
        assert!(ZoteroServer::get_item_tool_attr().output_schema.is_some());
        assert!(ZoteroServer::get_pdf_text_tool_attr()
            .output_schema
            .is_some());
        assert!(ZoteroServer::create_item_tool_attr()
            .output_schema
            .is_some());
        assert!(ZoteroServer::propose_metadata_update_tool_attr()
            .output_schema
            .is_some());

        // The nine list-returning tools now carry schemas too, via the
        // ListResult<T> envelope. Their bare-Vec shape could not be advertised:
        // MCP requires an object at the root of an output schema, and schemars
        // renders Vec<T> as type:array. This assertion replaces the inverse one
        // that guarded them while the wire-format question was open — Richard
        // settled it on 2026-07-25 (docs/superpowers/specs/
        // 2026-07-25-slice-g-wire-format-decision.md).
        assert!(ZoteroServer::search_items_tool_attr()
            .output_schema
            .is_some());
        assert!(ZoteroServer::list_collections_tool_attr()
            .output_schema
            .is_some());
        assert!(ZoteroServer::find_weak_metadata_items_tool_attr()
            .output_schema
            .is_some());

        // Text-returning tools still have none — deliberately bare strings.
        assert!(ZoteroServer::format_citation_tool_attr()
            .output_schema
            .is_none());
        assert!(ZoteroServer::add_tags_tool_attr().output_schema.is_none());
        assert!(ZoteroServer::delete_item_tool_attr()
            .output_schema
            .is_none());

        // The three lookup tools stay untyped by decision, not by deferral: the
        // schema they would gain says only "an object", while migrating would
        // push their structured lookup_failed body out of tool content.
        assert!(ZoteroServer::lookup_doi_tool_attr().output_schema.is_none());
        assert!(ZoteroServer::lookup_isbn_tool_attr()
            .output_schema
            .is_none());
        assert!(ZoteroServer::lookup_arxiv_tool_attr()
            .output_schema
            .is_none());
    }
}
