# zotero-mcp

A local-first MCP server that gives Claude fast, safe access to your Zotero
library — over stdio (Claude Desktop, Claude Code) or HTTP with OAuth 2.1
(Claude.ai web, Claude Cowork).

Browse your collections, search by tag, read PDF text and annotations, look
up DOIs / ISBNs / arXiv IDs, propose metadata enrichment from CrossRef /
Semantic Scholar / OpenLibrary, format citations and bibliographies in any
CSL style, create items, attach files and links, write notes, manage tags.
Everything runs against your local Zotero — no library data is shipped to a
third party.

## Why this exists

Most serious readers split their thinking across two systems: a
**reference manager** (Zotero) that owns the citation graph, PDFs and
canonical metadata, and a **wiki-style notes app** (Obsidian, Logseq, Roam,
Foam, …) that owns the long-form writing, backlinks and synthesis. The
friction between them is real: you read a paper in Zotero, then context-
switch to write about it in Markdown, retyping the citation, hunting for
the DOI, copying a quote with the wrong page number.

`zotero-mcp` removes the seam by exposing Zotero as MCP tools Claude can
call mid-conversation. While you're drafting a note in Obsidian (or
talking to Claude in any context), Claude can search your library, pull
PDF text, format an APA citation, propose missing metadata from CrossRef,
even create a new Zotero item and attach a PDF — all without leaving the
chat. The library stays the source of truth; Claude becomes the bridge
between the structured citation world and the free-form notes world.

## What you can ask Claude to do

Concrete examples (these are prompts you'd type to Claude with the server
wired in):

**Research and reading**

- *"Find every paper in my **ENERGY BOOK** collection that mentions 'EROI'
  and give me a 1-paragraph summary of each."*
- *"Read the first 3 pages of `<item-key>` and pull out the methodology
  section."*
- *"Show me all PDF annotations I made on items tagged `climate-feedback`
  this year."*

**Writing and citation**

- *"Format these 5 items as an APA bibliography."*
- *"Convert Zotero item `<key>` into an Obsidian note: front-matter with
  title/authors/DOI, key claims as bullets, a markdown link to the PDF."*
- *"Give me the BibTeX entry for the Hall et al. 2014 paper on EROI."*

**Library admin**

- *"Find items in my library missing a DOI or abstract, enrich them from
  CrossRef, and only auto-apply when confidence > 0.9."*
- *"Create a Zotero item for arXiv 2401.12345, file it under my **AI**
  collection, tag it `to-read`."*
- *"Attach `~/Downloads/Smith2023.pdf` to the item I just created."*

**Cross-system PKM**

- *"For each item tagged `thesis-chapter-3`, generate a one-paragraph
  Obsidian note linking back to the Zotero key."*
- *"I'm writing about peak oil. Search my library, give me the 5 most
  cited papers, and produce an annotated bibliography."*

## Install

```bash
cargo install zotero-mcp
```

Requires Rust 1.75+ (2021 edition).

You also need:

- **Zotero desktop** (running), with
  *Preferences → Advanced → Allow other applications to communicate with Zotero* enabled.
- **BetterBibTeX** (optional; without it, `citation_key` fields are `null` but
  everything else works).
- A **Zotero Web API key** if you want to write to the library (add notes,
  tags, metadata patches, create items, attach files). Generate one at
  <https://www.zotero.org/settings/keys> with the *library:write* permission,
  then add it to `config.toml`:

  ```toml
  [zotero]
  api_key = "<paste the key>"
  ```

  Reads do not need a key — Zotero's local server serves them with no auth.
  Writes have to go through `api.zotero.org` because the local server
  returns 501 on `PATCH`/`POST` (see [How it works](#how-it-works)).

- **Poppler's `pdftotext`** (optional, recommended): a small set of PDFs
  use features the pure-Rust `pdf-extract` crate doesn't handle (e.g.
  PostScript Calculator functions). When `pdftotext` is on `PATH`,
  `zotero-mcp` automatically falls back to it and caches the recovered
  text alongside Zotero's own full-text index.

  ```bash
  brew install poppler            # macOS
  sudo apt install poppler-utils  # Debian/Ubuntu
  ```

- **A [Docling](https://github.com/docling-project/docling-serve) service**
  (optional, strongly recommended): enables the layout-aware primary
  extraction route — markdown output with real tables, reading order,
  page anchors, and equations decoded to LaTeX. Point `zotero-mcp` at it
  with the `DOCLING_URL` environment variable or `docling_url` in
  `config.toml`. Without it, extraction uses the flat-text chain only
  (and says so in the completeness report).

- **`ocrmypdf`** (optional): enables the OCR pre-step for scanned /
  image-only PDFs on the Docling route. Without it, scanned PDFs are
  not OCR'd and the gap is declared in the completeness report.

  ```bash
  brew install ocrmypdf           # macOS
  sudo apt install ocrmypdf       # Debian/Ubuntu
  ```

## How it works

### Reads vs writes (the two-faced HTTP client)

Zotero exposes two APIs and `zotero-mcp` uses both:

- **Local server** at `http://localhost:23119` — serves reads (search,
  PDF paths, annotations, collections, tags). No auth needed; data never
  leaves your machine. Cheap and fast.
- **Web API** at `https://api.zotero.org` — handles writes (notes, tag
  edits, metadata patches, item creation, file uploads). Authenticated
  with your Zotero Web API key. Writes propagate back to your local
  Zotero through Zotero's own sync.

This split is forced by Zotero itself: the local server is read-only
and returns `501 Not Implemented` on `PATCH`/`POST`. The trade-off is
that without an API key you get full read access but writes are
disabled.

### PDF extraction: routes and the completeness contract

`get_pdf_text` / `get_pdf_first_pages` share one extraction stack, tried
in order:

1. **Docling** (layout-aware primary route, when configured) — HTTP call
   to a [docling-serve](https://github.com/docling-project/docling-serve)
   instance with formula enrichment enabled. Output is markdown with real
   tables, reading order, `--- p.N ---` page anchors, and equations
   decoded to LaTeX. When the PDF has no usable text layer (a scan), an
   **OCR pre-step** runs `ocrmypdf --skip-text` on a temp copy first —
   the original file is never modified — and the source is labelled
   `ocr_then_docling`.
2. **`.zotero-ft-cache`** — Zotero's own flat-text extraction, kept as a
   fast path when the Docling route is unavailable.
3. **`pdf-extract`** — in-process pure-Rust extraction.
4. **`pdftotext`** — Poppler subprocess fallback.

If every route fails or yields sub-floor text, the tools return a loud
error naming the remedy — never empty text as success. Passing
`plain=true` skips Docling entirely and forces the old flat-text output.

Every result carries a **machine-readable completeness report**
(`completeness`): the engine used, page count, per-page character
counts, and the page locations of undecoded formulas, untranscribed
figures/charts, OCR-recovered pages, and suspiciously low-text pages,
plus a boolean `complete`. The contract for consumers (especially LLM
pipelines fact-checking notes against this text):

- **Presence is trustworthy** — text in the output really is in the
  document.
- **Absence is only trustworthy when `complete: true`.** Where the
  report declares drops (`undecoded_formulas`, `untranscribed_images`,
  a flat-text note), treat those regions as *unknown*, never as "not in
  the document".
- Flat-text engines (cache, `pdf-extract`, `pdftotext`) cannot detect
  tables/formulas/images, so their results are always `complete: false`
  with an explicit note — degraded extraction is a declared, queryable
  state, not a silent one.

The Docling endpoint is read from the `DOCLING_URL` environment
variable, which takes precedence over `docling_url` in `config.toml`;
when neither is set the route is disabled. On this project's home setup
the service is a GPU box on the tailnet at `http://100.79.12.8:5001` —
substitute your own docling-serve instance.

### Two transport modes

**Stdio** is the default and the simplest. Claude Desktop and Claude
Code spawn `zotero-mcp` as a subprocess and talk to it over stdin/stdout
using the MCP protocol. No network, no daemon, no auth gate. Wire it
in once via `claude_desktop_config.json` and forget about it.

**HTTP with OAuth 2.1** is what Claude.ai and Claude Cowork require.
The Cowork sandbox runs in an isolated Linux VM that cannot launch local
stdio subprocesses on your Mac, so the stdio config doesn't reach it.
Instead, `zotero-mcp` runs as a long-lived launchd job that exposes an
HTTPS endpoint over **Tailscale Funnel**, gated by **OAuth 2.1
(authorization_code + PKCE)**:

```
Cowork sandbox  →  https://<your-host>.<tailnet>.ts.net/mcp   (HTTPS, Tailscale Funnel)
                →  http://127.0.0.1:8765                       (loopback, on your Mac)
                →  zotero-mcp                                  (streamable-HTTP transport, per MCP 2025-06-18)
                →  http://localhost:23119                      (Zotero local API)
```

Why this design:

- **Tailscale Funnel** gives a stable HTTPS URL on a trusted certificate
  without you having to run an NGINX/Caddy setup or punch firewall holes.
  The local zotero-mcp listens only on 127.0.0.1; Funnel terminates TLS
  at the edge and proxies to loopback.
- **OAuth 2.1 + PKCE** means anyone hitting the public URL without a
  valid bearer token gets a 401. The Claude.ai connector flow is the
  only way in. Credentials are minted once during `zotero-mcp setup` and
  stored in a local config file.
- **Your Zotero library stays local**: even in HTTP mode, the server is
  the same binary doing the same local API reads. The HTTP layer is a
  transport, not a data shipper. No library content is ever uploaded to
  anywhere except `api.zotero.org` (for writes you explicitly request).

The HTTP mode is macOS-only out of the box because the bootstrapping uses
launchd; manual setup for Linux/Windows is in
[`docs/CLAUDE_COWORK_SETUP.md`](docs/CLAUDE_COWORK_SETUP.md).

### OAuth configuration

The HTTP transport is driven by two environment variables (the `zotero-mcp
setup` command writes them into a launchd plist for you; manual deployments
set them in whatever init system you use):

| Env var | Meaning |
|---------|---------|
| `ZOTERO_MCP_HTTP` | Bind address for the HTTP listener, e.g. `127.0.0.1:8765`. When set, the server runs in HTTP mode instead of stdio. |
| `ZOTERO_MCP_OAUTH_ISSUER` | Public URL the OAuth surface advertises in discovery and 401 challenges — e.g. `https://laptop.tailnet.ts.net`. Must match what Claude.ai believes the canonical URI is. When set and `oauth.toml` is missing, the server generates a fresh credential pair on first start. When unset and no `oauth.toml` exists, OAuth is disabled (the HTTP server runs without an auth gate; security then comes from the transport — e.g. a private Funnel URL). |

Generated credentials persist at:

- macOS: `~/Library/Application Support/dev.zotero-mcp.zotero-mcp/oauth.toml`
- Linux: `~/.config/zotero-mcp/oauth.toml`

…with mode `0600` so the secret never lands in a world-readable location.
The file is plain TOML:

```toml
client_id = "zotero-mcp-<8-hex>"
client_secret = "<32-hex>"
issuer = "https://laptop.tailnet.ts.net"
```

The OAuth flow itself follows the spec: `authorization_code` with PKCE
(SHA-256, base64url, no pad), discovery via
`/.well-known/oauth-protected-resource` and
`/.well-known/oauth-authorization-server`, 401 challenges per RFC 9728.
Access tokens are opaque 32-byte hex strings; authorization codes have a
5-minute TTL and are single-use.

**Redirect URI allowlist** is hardcoded to `https://claude.ai/api/mcp/*`
and `https://claude.com/api/mcp/*`. If you're integrating with a different
MCP client, you'll need to add its origin to `ALLOWED_REDIRECT_URI_PREFIXES`
in `crates/zotero-mcp/src/oauth.rs` and rebuild.

**To rotate credentials**: delete `oauth.toml` and restart the server
(`launchctl bootout … && launchctl bootstrap …`, or `zotero-mcp setup`
again). A fresh pair is generated on first start; re-paste the new
credentials into Claude.ai's connector config.

### Token durability

Access and refresh tokens are persisted to `<config_dir>/tokens.json` (mode 0600,
hashed at rest with SHA-256). This means OAuth sessions survive `launchd`
restarts, system sleep, log out/in, and `zotero-mcp setup` re-bootstrap — the
connector keeps working without re-authenticating in the browser.

Default TTLs:

| Token | Default TTL | Override field in `oauth.toml` |
|---|---|---|
| Access token | 7 days | `access_token_ttl_secs` |
| Refresh token | 90 days | `refresh_token_ttl_secs` |

The 7-day access TTL is a workaround for the [open Anthropic bug](https://github.com/anthropics/claude-ai-mcp/issues/228) where
`mcp-proxy.anthropic.com` ignores refresh tokens. Once Anthropic ships their proxy fix, you can lower this back to 1 hour:

```toml
access_token_ttl_secs = 3600
```

Refresh tokens follow OAuth 2.1 §4.3.1: one-time-use with rotation. If a refresh
token is replayed (a leak signal), the entire token chain is revoked and you're
forced through one fresh browser auth.

To revoke all tokens manually:

```bash
rm "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/tokens.json"
launchctl bootout gui/$UID/com.zotero-mcp.http
launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
```

## Use it

### Claude Desktop / Claude Code (stdio)

Wire it into Claude Desktop's `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "zotero": { "command": "zotero-mcp" }
  }
}
```

For Claude Code: see [`docs/CLAUDE_CODE_SETUP.md`](docs/CLAUDE_CODE_SETUP.md).

### Claude.ai / Claude Cowork (HTTP + OAuth 2.1)

```bash
zotero-mcp setup
```

macOS only. The setup command auto-detects your Tailscale Funnel hostname,
writes the launchd plist, enables Funnel on port 8765, generates the OAuth
credentials, and prints a paste-ready block:

```
=== Paste these into Claude.ai → Settings → Connectors → Add custom ===

  Server URL          https://<host>.<tailnet>.ts.net/mcp
  Advanced ▸ Client ID     zotero-mcp-<8-hex>
  Advanced ▸ Client Secret <32-hex>
```

Two companion subcommands:

- `zotero-mcp status` — health check across launchd, the HTTP listener,
  Tailscale Funnel, Zotero's local API, and the OAuth config file.
- `zotero-mcp show-credentials` — re-print the paste-ready block.

Full setup notes including manual / non-macOS deployment in
[`docs/CLAUDE_COWORK_SETUP.md`](docs/CLAUDE_COWORK_SETUP.md).

## Tools

All 34 tools the server exposes, grouped by purpose. Tool descriptions
are paraphrased from each tool's `#[tool(description = …)]` declaration
— what Claude itself sees when deciding which tool to call.

### Discover and browse

| Tool | What it does |
|------|--------------|
| `search_items` | Library search (metadata + optional fulltext) |
| `list_recent_items` | Items sorted by `dateAdded` or `dateModified` |
| `get_item` | Single item by key, with `citation_key` if BBT is available |
| `list_collections` | All collections in your library |
| `list_tags` | Tags, optionally prefix-filtered |
| `list_attachments` | File attachments and snapshots for an item, with resolved absolute paths |
| `list_annotations` | PDF highlights and comments for an item |
| `find_weak_metadata_items` | Items with missing DOI/abstract or stub titles (enrichment candidates) |

### Read content

| Tool | What it does |
|------|--------------|
| `get_pdf_text` | Full PDF text — layout-aware markdown via Docling by default (tables, `--- p.N ---` page anchors, LaTeX equations, OCR pre-step for scans) with a machine-readable completeness report; falls back to the flat-text chain (`.zotero-ft-cache` → `pdf-extract` → `pdftotext`) when the service is unreachable; `plain=true` forces the old flat output |
| `get_pdf_first_pages` | First N pages of a PDF (default 2) — same extraction contract as `get_pdf_text`, cheaper |
| `get_pdf_path` | Absolute filesystem path to an attachment (raw-bytes use cases; prefer `get_pdf_text` for text) |
| `get_webpage_content` | Webpage content for an item via stored snapshot or live fetch (mode: `snapshot`/`live`/`auto`) |
| `refetch_url` | Re-fetch a webpage item live, optionally saving a fresh HTML snapshot |

### Lookup external metadata

| Tool | What it does |
|------|--------------|
| `lookup_doi` | DOI → flat Zotero-shaped JSON via CrossRef. `format="zotero"` (default) returns an item ready to pass straight to `create_item`; `format="candidate"` returns an envelope for use with `propose_metadata_update`/`enrich_item` |
| `lookup_isbn` | ISBN → flat Zotero-shaped JSON via OpenLibrary (same `format` choice as above; freeform `publish_date` normalised to ISO 8601) |
| `lookup_arxiv` | arXiv ID → flat Zotero-shaped JSON (same `format` choice as above) |
| `search_crossref` | Free-text CrossRef search; normalized candidates |
| `search_semantic_scholar` | Free-text Semantic Scholar search; normalized candidates |

When emitted as flat Zotero items (the default), the lookup tools stash
provenance in Zotero's `extra` field as newline-separated `key: value`
lines (`source: openlibrary` / `sourceURL: …`) so the origin of each
record survives into the library.

### Format

| Tool | What it does |
|------|--------------|
| `format_citation` | One item as a citation (`style` = CSL name; `format` = `bib`/`biblatex`/`bibtex`/`ris`) |
| `format_bibliography` | Multiple items as a combined bibliography (same options) |

### Enrich (propose → review → apply)

| Tool | What it does |
|------|--------------|
| `propose_metadata_update` | Score candidate metadata, produce an `EnrichmentProposal` — does **not** apply |
| `apply_metadata_update` | Apply a previously generated `EnrichmentProposal` |
| `enrich_item` | Compose propose+apply; auto-applies only when confidence ≥ threshold *and* multi-source agreement |

The split exists so that Claude (or you) can review a proposal before
writes happen. `enrich_item` is the bulk-safe convenience: it will pass
on items where confidence isn't high enough rather than guessing.

`propose_metadata_update` and `enrich_item` require their `candidates`
to be lookup results obtained with `format="candidate"` — they need
the envelope's `source` field for scoring. Items obtained with the
default `format="zotero"` will fail validation.

### Write

| Tool | What it does |
|------|--------------|
| `create_item` | Create a new Zotero item from a JSON metadata object. The default-format output of `lookup_doi` / `lookup_isbn` / `lookup_arxiv` drops straight in with no transform |
| `attach_file` | Attach a local file as a child of an item; supports `imported_file` (copies bytes into Zotero's managed `storage/<key>/` dir; desktop client syncs to your configured backend — cloud or WebDAV) and `linked_file` (path reference for BYO-storage setups like Resilio/Syncthing) |
| `attach_link` | Attach a URL as a `linked_url` child (no bytes transfer) |
| `add_note` | Markdown/HTML note attached to an item |
| `update_item_fields` | Patch arbitrary fields (auto-detects version for `If-Unmodified-Since-Version`) |
| `add_tags` / `remove_tags` | Tag CRUD (`add_tags` deduplicates against existing tags) |
| `add_to_collection` / `remove_from_collection` | Move items between collections |
| `delete_item` | Move item/note/attachment to Zotero's trash (recoverable) |

### Diagnostic

| Tool | What it does |
|------|--------------|
| `ping` | Liveness check; returns `"pong (v<version>, <git-sha>)"` so callers can confirm which build is responding |

Run `zotero-mcp` in stdio mode with an MCP inspector to see the full
schemas and argument types.

## Configuration

Optional TOML at the platform config dir:

- macOS: `~/Library/Application Support/dev.zotero-mcp.zotero-mcp/config.toml`
- Linux: `~/.config/zotero-mcp/config.toml`

Defaults work out of the box for a stock Zotero install except for
`zotero.api_key`, which you must set for writes to work. Every field is
optional; values shown below are the defaults.

```toml
[zotero]
# Zotero data directory (where the SQLite + attachment storage live).
data_dir = "~/Zotero"

# Local Zotero HTTP server (reads). Default is Zotero's documented port.
local_api_base = "http://localhost:23119"

# Zotero Web API endpoint (writes only; reads use local_api_base).
web_api_base = "https://api.zotero.org"

# Web API key from https://www.zotero.org/settings/keys with library:write.
# Required for any write operation. Leave unset for read-only.
# api_key = "<paste>"

# Your Zotero user ID (from the same Settings page). 0 = auto-detect from
# the local API at startup.
user_id = 0

# Include items from group libraries you have access to.
include_group_libraries = true

# Zotero schema-version range this build is tested against. The server
# refuses to start if your library is outside the supported window
# (Zotero's schema does evolve; we hold a snapshot).
min_schema_userdata = 120
max_schema_userdata = 135

# PDF text extraction fallback. When pdf-extract fails on a tricky PDF
# (e.g. PostScript Calculator functions), fall back to Poppler's pdftotext
# and cache the result. Set to false to disable.
pdftotext_fallback = true

# Explicit path to pdftotext binary. When set and the file exists, used
# instead of PATH lookup. Useful for non-standard installs.
# pdftotext_path = "/opt/homebrew/bin/pdftotext"

# Base URL of the Docling conversion service (layout-aware primary
# extraction route). The DOCLING_URL environment variable takes
# precedence over this value. When neither is set, the Docling route is
# disabled and extraction uses the flat-text chain only.
# docling_url = "http://100.79.12.8:5001"

# Wall-clock timeout for one Docling convert request, in seconds.
# Formula enrichment on large PDFs is slow — keep this generous.
docling_convert_timeout_secs = 300

# Timeout for the Docling /health probe, in seconds.
docling_health_timeout_secs = 5

# Explicit path to the ocrmypdf binary used by the OCR pre-step for
# scanned (image-only) PDFs. When set and the file exists, used instead
# of PATH lookup. When ocrmypdf cannot be resolved at all, the OCR
# pre-step is skipped and the gap is recorded in the completeness report.
# ocrmypdf_path = "/opt/homebrew/bin/ocrmypdf"

# attach_file storage mode. "imported_file" copies bytes into
# <data_dir>/storage/<key>/<filename> — the same on-disk layout Zotero's
# UI produces. Zotero desktop's sync engine then pushes the file to
# whichever backend you have configured (Zotero cloud, WebDAV, or none).
# "linked_file" stores only a path reference (BYO storage, e.g. Resilio
# Sync, Syncthing, NAS-backed Zotero).
attachment_mode = "imported_file"

# Required when attachment_mode = "linked_file". Files attached via
# attach_file must live inside this directory.
# linked_attachment_base_dir = "/Users/you/Resilio/Zotero-Attachments"

# Per-file size ceiling for attach_file. Default: 50 MB.
max_attachment_bytes = 52428800

[enrichment]
# Confidence threshold for enrich_item auto-apply. Below this, the tool
# produces a proposal without applying. Range 0.0 - 1.0.
auto_apply_threshold = 0.9

# Sources consulted by find_weak_metadata_items and enrich_item.
sources = ["crossref", "openlibrary", "arxiv", "semantic_scholar"]

# How long to cache external lookup results.
cache_ttl_days = 30

[web]
# How long to keep cached webpage snapshots before considering them stale.
snapshot_cache_ttl_hours = 24

# User-Agent sent when refetching URLs.
user_agent = "zotero-mcp/0.1"

[paths]
# Override the platform default cache and log dirs.
# cache_dir = "~/.cache/zotero-mcp"
# log_dir = "~/.local/state/zotero-mcp"

[logging]
# tracing-subscriber level: error | warn | info | debug | trace
level = "info"
```

## Troubleshooting

**`zotero-mcp` returns errors like "cannot connect to local API"**
Zotero desktop isn't running, or *Preferences → Advanced → Allow other
applications to communicate with Zotero* is unchecked. Open Zotero and
verify both.

**Writes fail with `WriteApiKeyMissing`**
You haven't set `[zotero] api_key` in `config.toml`. Generate one at
<https://www.zotero.org/settings/keys> with *library:write* permission
and paste it in. Reads don't need a key.

**`412 Precondition Failed` on writes**
Zotero's optimistic concurrency control fired — the item you're trying
to patch has been modified since the last read. Re-read the item and
retry. `update_item_fields` does this auto-detection internally;
direct API users may need to handle it.

**PDF text extraction returns empty / errors out**
On the flat-text chain: the PDF uses features the pure-Rust `pdf-extract`
crate doesn't support. Install Poppler (`brew install poppler` /
`sudo apt install poppler-utils`) so `zotero-mcp` can fall back to
`pdftotext`. The fallback is automatic when `pdftotext` is on `PATH`.
Confirm with `which pdftotext`. For scanned (image-only) PDFs, install
`ocrmypdf` and configure a Docling service (`DOCLING_URL` or
`docling_url`) so the OCR pre-step can recover the text.

**`get_pdf_text` reports `complete: false`**
Not an error — the completeness report declaring a gap. Check
`completeness.notes` and the drop vectors: a flat-text note means the
Docling route wasn't reachable (check `DOCLING_URL` / `docling_url` and
the service's `/health`); `undecoded_formulas` / `untranscribed_images`
name pages whose content is present in the PDF but not in the text.
Treat those regions as unknown, not absent.

**`attach_file` returns `AttachmentOutsideBaseDir`**
You set `attachment_mode = "linked_file"` and tried to attach a file
that isn't under `linked_attachment_base_dir`. Either move the file
in, or pass `mode = "imported_file"` for that specific call.

**HTTP mode: 401 from the public URL after `zotero-mcp setup`**
That's the OAuth gate working as intended. Use the paste-ready block
from `zotero-mcp show-credentials` in *Claude.ai → Settings →
Connectors → Add custom*. The connector flow is the only way in.

**`zotero-mcp status` reports Tailscale Funnel not enabled**
Funnel is a one-time admin toggle on your tailnet. Enable it at
<https://login.tailscale.com/admin/settings/features>, then re-run
`zotero-mcp setup`.

**I want to rotate / revoke OAuth credentials**
Delete `<config_dir>/oauth.toml` and restart the server (`launchctl
bootout && launchctl bootstrap …`, or simpler: re-run `zotero-mcp
setup`). The server generates a fresh `client_id` / `client_secret`
on next start. Re-paste the new pair into Claude.ai's connector config
— the old credentials stop working immediately.

**OAuth seems disabled — server accepts unauthenticated requests on the
public URL**
`ZOTERO_MCP_OAUTH_ISSUER` is unset and no `oauth.toml` exists. In that
state, OAuth is off by design and security relies on the transport (a
private Funnel URL, a VPN, etc.). Run `zotero-mcp setup` to bootstrap
the OAuth gate, or set `ZOTERO_MCP_OAUTH_ISSUER` in your daemon's
environment and restart.

**`attach_file imported_file` succeeded but the file never appears on
my Zotero cloud / WebDAV server**
Bytes land at `<data_dir>/storage/<attachment_key>/<filename>` first;
Zotero desktop's sync engine then pushes them to your configured
backend on the next sync pass. Make sure Zotero desktop is running,
"Sync attachment files" is enabled in Preferences → Sync, and trigger
sync manually (⇧⌘S on macOS) if you're not seeing activity. Builds
before 0.3.1 had a bug where the row landed at `syncState = IN_SYNC`
and the sync engine never queued the upload — `cargo install
zotero-mcp --force` to upgrade.

## Upgrading

```bash
cargo install zotero-mcp --force
```

After upgrading:

- **Stdio mode**: nothing else to do — Claude Desktop/Code will spawn
  the new binary on next launch.
- **HTTP mode**: re-run `zotero-mcp setup` if the launchd plist is
  older than the new binary (the plist references the install path).
  `zotero-mcp status` will flag a mismatch.
- **Upgrading from `0.2.x`**: v0.3.0 replaced the legacy SSE transport
  (`GET /sse` + `POST /message`) with the new streamable-HTTP transport
  per MCP 2025-06-18 (`POST /mcp`). If you have an existing Claude.ai /
  Cowork connector pointing at `…/sse`, you must add a new connector
  whose URL ends in `/mcp` (Cowork currently has no UI to edit an
  existing connector's URL — see
  [anthropics/claude-ai-mcp#73](https://github.com/anthropics/claude-ai-mcp/issues/73)).
  The new connector reuses the same `client_id` / `client_secret`, so
  `zotero-mcp show-credentials` still applies.

If your Zotero library is outside the supported schema window
(`min_schema_userdata`/`max_schema_userdata`) after an update, the server
will refuse to start with a clear error pointing at the schema range.
Bump those config knobs once you've verified the new schema works.

Release notes and changelog: see git tags and commit history on
<https://github.com/richardjlyon/zotero-mcp>.

## Integration test against your real Zotero library

The unit tests use mocked HTTP servers and don't touch your library. A
separate gated test exercises the write tools against the real Zotero
Web API end-to-end. Useful when:

- You're about to depend on `create_item` / `attach_file` / `attach_link`
  in a workflow.
- You've upgraded `zotero-mcp` and want to verify writes still work.
- You're contributing changes that touch the write tools.

Setup (one-time):

1. Generate a Zotero Web API key with `library:write` permission at
   <https://www.zotero.org/settings/keys>.
2. In Zotero desktop, create a collection named `_zotero-mcp-test`. The
   test scopes everything to this collection so a failure can't pollute
   real data. Note its key (right-click → "Generate Report" or via the
   Zotero connector — any way to get the 8-char collection key).
3. Find your Zotero user ID at the same Settings page.

Run:

```bash
ZOTERO_MCP_LIVE_API_KEY=<key> \
ZOTERO_MCP_LIVE_USER_ID=<user-id> \
ZOTERO_MCP_TEST_COLLECTION_KEY=<collection-key> \
ZOTERO_MCP_TEST_PAUSE=1 \
cargo test -p zotero-mcp --test writer_live_zotero -- --nocapture --ignored
```

`ZOTERO_MCP_TEST_PAUSE` triggers a manual-verification pause before
teardown — open Zotero, navigate to the test collection, eyeball the
created item and its two children, then press ENTER to let the test
clean up.

For environments where stdin isn't a TTY (CI, non-interactive shells),
set `ZOTERO_MCP_TEST_KEEP=1` instead: the test skips teardown entirely
and prints a ready-to-paste DELETE command so you can verify and clean
up on your own timeline.

## License

MIT OR Apache-2.0.
