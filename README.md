# zotero-mcp

A local-first MCP server that gives Claude fast, safe access to your Zotero
library — over stdio (Claude Desktop, Claude Code) or HTTP/SSE with OAuth 2.1
(Claude.ai web, Claude Cowork).

Browse your collections, search by tag, read PDF text, look up DOIs and
arXiv IDs, propose metadata enrichment from CrossRef / Semantic Scholar /
OpenLibrary, format citations and bibliographies in any CSL style, write
notes, manage tags. Everything runs against your local Zotero instance —
no library data is shipped to a third party.

## Install

```bash
cargo install zotero-mcp
```

You also need:

- **Zotero desktop** (running), with
  *Preferences → Advanced → Allow other applications to communicate with Zotero* enabled.
- **BetterBibTeX** (optional; without it, `citation_key` fields are `null` but
  everything else works).
- A **Zotero Web API key** if you want to write to the library (add notes,
  tags, metadata patches, …). Generate one at
  <https://www.zotero.org/settings/keys> with the *library:write* permission,
  then add it to `config.toml`:

  ```toml
  [zotero]
  api_key = "<paste the key>"
  ```

  Reads do not need a key — Zotero's local server serves them with no auth.
  Writes have to go through `api.zotero.org` because the local server
  returns 501 on `PATCH`/`POST`.

- **Poppler's `pdftotext`** (optional, recommended): a small set of PDFs
  use features the pure-Rust `pdf-extract` crate doesn't handle (e.g.
  PostScript Calculator functions). When `pdftotext` is on `PATH`,
  `zotero-mcp` automatically falls back to it and caches the recovered
  text alongside Zotero's own full-text index. Install with:

  ```bash
  brew install poppler          # macOS
  sudo apt install poppler-utils  # Debian/Ubuntu
  ```

  Or set an explicit path in `config.toml`:

  ```toml
  [zotero]
  pdftotext_path = "/opt/homebrew/bin/pdftotext"
  pdftotext_fallback = true   # default; set false to disable
  ```

## Use it

### Claude Desktop / Claude Code (stdio)

The server speaks MCP over stdio by default. Wire it into Claude Desktop's
`claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "zotero": { "command": "zotero-mcp" }
  }
}
```

For Claude Code see [`docs/CLAUDE_CODE_SETUP.md`](docs/CLAUDE_CODE_SETUP.md).

### Claude.ai / Claude Cowork (HTTP/SSE + OAuth 2.1)

Cowork runs in a Linux sandbox that can't launch local stdio subprocesses, so
the server publishes an HTTP/SSE endpoint over Tailscale Funnel and gates it
with OAuth 2.1 (authorization_code + PKCE).

```bash
zotero-mcp setup
```

macOS only. The setup command auto-detects your Tailscale Funnel hostname,
writes the launchd plist, enables Funnel on port 8765, generates the OAuth
credentials, and prints a paste-ready block:

```
=== Paste these into Claude.ai → Settings → Connectors → Add custom ===

  Server URL          https://<host>.<tailnet>.ts.net/sse
  Advanced ▸ Client ID     zotero-mcp-<8-hex>
  Advanced ▸ Client Secret <32-hex>
```

Two companion subcommands:

- `zotero-mcp status` — health check across launchd, the HTTP listener,
  Tailscale Funnel, Zotero's local API, and the OAuth config file.
- `zotero-mcp show-credentials` — re-print the paste-ready block.

Full setup notes including manual / non-macOS deployment in
[`docs/CLAUDE_COWORK_SETUP.md`](docs/CLAUDE_COWORK_SETUP.md).

## What Claude can do

Tools (excerpt):

| Tool | What it does |
|------|--------------|
| `search_items` | Library search (metadata + optional fulltext) |
| `list_recent_items` | Newest by `dateAdded` / `dateModified` |
| `get_item` | Single item, with `citation_key` hydrated when BBT is available |
| `list_collections` / `list_tags` | Browse the structure |
| `get_pdf_text` / `get_pdf_first_pages` | Read attachment text |
| `list_annotations` | Highlights and comments |
| `format_citation` / `format_bibliography` | Render in any CSL style |
| `lookup_doi` / `lookup_arxiv` / `lookup_isbn` | External metadata fetch |
| `search_crossref` / `search_semantic_scholar` | Free-text academic search |
| `propose_metadata_update` / `apply_metadata_update` / `enrich_item` | Score + apply enrichment |
| `add_note`, `add_tags`, `remove_tags`, `add_to_collection`, `remove_from_collection` | Library writes |
| `update_item_fields` | Patch arbitrary fields (version-aware) |
| `delete_item` | Move item, note, or attachment to trash (recoverable) |

Run `zotero-mcp` in stdio mode + an MCP inspector to see the full list.

## Configuration

Optional TOML at the platform config dir:

- macOS: `~/Library/Application Support/dev.zotero-mcp.zotero-mcp/config.toml`
- Linux: `~/.config/zotero-mcp/config.toml`

See `crates/zotero-mcp/src/core/config.rs` for fields and defaults. The
defaults work out of the box for a stock Zotero install except for
`zotero.api_key`, which you must set explicitly if you want writes to work
(see Install section above).

## License

MIT OR Apache-2.0.
