# Wiring zotero-mcp into Claude Code

Add the server to your Claude Code config (`~/.claude/mcp_settings.json` or your project's `.mcp.json`):

```json
{
  "mcpServers": {
    "zotero": {
      "command": "/absolute/path/to/target/release/zotero-mcp",
      "args": [],
      "env": {}
    }
  }
}
```

Restart Claude Code. The `zotero` server should appear in `/mcp` with tools:

- search_items, get_item, list_collections, list_tags, list_recent_items
- list_attachments, get_pdf_path, get_pdf_text, get_pdf_first_pages, list_annotations
- get_webpage_content, refetch_url
- format_citation, format_bibliography
- add_note, update_item_fields, add_tags, remove_tags, add_to_collection, remove_from_collection
- find_weak_metadata_items, lookup_doi, lookup_isbn, lookup_arxiv, search_crossref, search_semantic_scholar
- propose_metadata_update, apply_metadata_update, enrich_item

And resources:

- `zotero://collections`
- `zotero://tags`

## Troubleshooting

- "Local API is not enabled" → toggle the setting in Zotero Preferences → Advanced.
- Schema version mismatch on startup → bump `max_schema_userdata` in your config TOML after eyeballing the new schema against this repo's queries.
- Logs go to stderr; if you set `paths.log_dir`, a file at `<log_dir>/zotero-mcp.log` will also receive entries.
