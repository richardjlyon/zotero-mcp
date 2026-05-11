# zotero-connector

A local MCP server that gives Claude fast, safe access to your Zotero library.

## Status

v0.1 — see `docs/superpowers/specs/2026-05-11-zotero-connector-design.md`.

## Requirements

- Zotero desktop (running), with **Preferences → Advanced → Allow other applications to communicate with Zotero** enabled.
- The BetterBibTeX plugin installed (soft dependency — without it, `citation_key` fields are `null` but everything else still works).
- Rust toolchain (stable).

## Build

```bash
cargo build --release -p zotero-mcp
```

Binary at `target/release/zotero-mcp`.

## Configure

Optional TOML file at `~/.config/zotero-mcp/config.toml`. See `crates/zotero-core/src/config.rs` for fields and defaults.

## Use with Claude Code

See `docs/CLAUDE_CODE_SETUP.md`.
