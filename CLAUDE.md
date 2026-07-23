# Working in this repo

`zotero-connector` is the Rust implementation of `zotero-mcp` — a local-first MCP server bridging Claude to a Zotero library over stdio or streamable-HTTP with OAuth 2.1.

## Conventions

- **Superpowers-driven planning.** Non-trivial changes start with a dated `docs/superpowers/specs/<date>-<topic>-design.md` followed by `docs/superpowers/plans/<date>-<topic>.md`. Implementation follows the plan; ad-hoc improvisation is the exception, not the rule.
- **TDD enforced.** Tests precede implementation, including for transport, OAuth, and tool-output normalisation slices.
- **MCP tool annotations** (`read_only_hint` / `destructive_hint` / `idempotent_hint` / `open_world_hint`) are mandatory on new tools — see the existing 34 for examples.
- **Output shapes** prefer typed `Json<T>` with derived `JsonSchema` over loose `CallToolResult` text. Slice G is migrating the residual.
- **Releases** are tagged `v0.x.0`. The crate published to crates.io is `zotero-mcp`.
- **Mirror tax.** The Plan-8 transport stack (`bearer.rs`, `oauth.rs`, `oauth/token_store.rs`, `http_transport.rs`, `setup.rs`) is duplicated into the sister repo `things-mcp` (`mcp-things` — path varies by machine; on the VM it will be `/vault/code/mcp-things` once migrated) by deliberate choice — no shared library extraction (pinned principle). Any fix to one of those modules MUST be cherry-picked to the sister repo in the same session, or the repos silently drift.

## Layout

| Path | Purpose |
|---|---|
| `crates/zotero-mcp/src/tools/` | MCP tool surface — `search`, `attachments`, `writes`, `enrichment`, `citations` |
| `crates/zotero-mcp/src/core/` | Reader pool, writer client, enrichment sources, pdf, web, citations, types |
| `crates/zotero-mcp/src/server.rs` | `#[tool_router]` registrations, server handler, `tool_handler!` glue |
| `crates/zotero-mcp/src/oauth*.rs` | OAuth 2.1 (auth-code + PKCE) for the streamable-HTTP transport |
| `docs/superpowers/specs/` | Per-change design briefs (dated) |
| `docs/superpowers/plans/` | Per-change execution plans (dated) |
| `docs/CLAUDE_CODE_SETUP.md` | How to wire the server into Claude Code (stdio) |
| `docs/CLAUDE_COWORK_SETUP.md` | How to wire the server into Cowork (streamable-HTTP via Tailscale Funnel) |

## Project brain (the vault)

Project context — architecture decisions, release state, the offload-spec backlog, open
questions — lives in the second-brain vault: `/vault/brain/Projects/Zotero MCP.md` (hub).
Load it with `/obsidian-projects zotero`; save state back at session end with
`/obsidian-log` or `/obsidian-save`. (Migrated 2026-07-23 from the old Cowork workspace,
now archived. Note: the VM reads a read-only Resilio mirror of the Mac's ~/Zotero data;
writes go via api.zotero.org — file attachments cannot use the local-sync path here.)

## Sister repos

- **`book-ingestion`** (`/Users/rjl/Code/tool-book-ingestion`) — Python CLI for book-shaped sources (PDF + EPUB metadata extraction shipped as M2.0 on 2026-05-13). Federated with this repo at the *skill* layer, not the code layer. Cowork project at `claude-cowork/project/book-ingestion/`.
