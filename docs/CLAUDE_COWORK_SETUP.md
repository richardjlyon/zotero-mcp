# Wiring zotero-mcp into Claude Cowork

Cowork sessions run in an isolated Linux sandbox that cannot launch
stdio subprocesses on your Mac, so the Claude-Desktop-style stdio
config does not reach Cowork. Instead, `zotero-mcp` exposes an
HTTP/SSE endpoint that the sandbox can reach as a "Custom connector".

## Architecture

```
Cowork sandbox  →  https://<your-host>.<tailnet>.ts.net/sse  (HTTPS, Tailscale Funnel)
                →  http://127.0.0.1:8765                      (loopback, on your Mac)
                →  zotero-mcp                                 (HTTP/SSE transport)
```

The Mac runs `zotero-mcp` in HTTP mode under `launchd`. Tailscale Funnel
publishes that local port to the public internet with a stable URL.
**OAuth 2.1 (authorization_code + PKCE)** gates the resource endpoints
so anyone hitting the public URL without a valid bearer token gets a
401 and the Claude.ai connector flow is the only way in.

## Prerequisites

- macOS with Tailscale installed and signed in.
- **Funnel enabled** on your tailnet — one-time admin action at
  `https://login.tailscale.com/admin/settings/features` (toggle "Funnel").
- `zotero-mcp` built and installed: `cargo install --path crates/zotero-mcp`.
- Zotero desktop running (Preferences → Advanced → "Allow other
  applications to communicate with Zotero" enabled).

## One-time setup (recommended)

```bash
cargo install --path crates/zotero-mcp     # or: cargo install zotero-mcp
zotero-mcp setup
```

`zotero-mcp setup` auto-detects the Tailscale Funnel hostname from
`tailscale status --json`, writes
`~/Library/LaunchAgents/com.zotero-mcp.http.plist`, bootstraps the
launchd job, enables `tailscale funnel --bg 8765`, waits for the server
to materialize `oauth.toml`, and prints a paste-ready credentials block:

```
=== Paste these into Claude.ai → Settings → Connectors → Add custom ===

  Server URL          https://laptop.<tailnet>.ts.net/sse
  Advanced ▸ Client ID     zotero-mcp-<8-hex>
  Advanced ▸ Client Secret <32-hex>
```

Two other subcommands round out the CLI:

- **`zotero-mcp status`** — health check across launchd, the HTTP
  listener, Funnel, the Zotero local API, and `oauth.toml`.
- **`zotero-mcp show-credentials`** — re-print the paste-ready block
  without regenerating anything.

If `setup` reports "Tailscale not available", install it from
<https://tailscale.com/download/macos> and enable Funnel on your tailnet
at <https://login.tailscale.com/admin/settings/features>, then re-run.

## Manual setup (fallback)

If you prefer to wire everything by hand — or you're on an OS without
launchd — set the environment yourself:

- `ZOTERO_MCP_HTTP=127.0.0.1:8765`
- `ZOTERO_MCP_OAUTH_ISSUER=https://laptop.<tailnet>.ts.net` — **must
  match the public Tailscale Funnel hostname exactly**, with `https://`
  and no trailing slash. OAuth clients validate this in discovery, so a
  mismatch breaks the handshake.

A reference plist is checked into this repo at
`docs/launchd/com.zotero-mcp.http.plist`. Copy it to
`~/Library/LaunchAgents/`, then:

```bash
mkdir -p ~/Library/Logs/zotero-mcp
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.zotero-mcp.http.plist
tailscale funnel --bg 8765
```

On first start the server generates a pre-shared OAuth client
credential pair at
`~/Library/Application Support/dev.zotero-mcp.zotero-mcp/oauth.toml`
(mode `0600`). Inspect with `zotero-mcp show-credentials` or `cat`:

```toml
client_id = "zotero-mcp-<8-hex>"
client_secret = "<32-hex>"
issuer = "https://laptop.<tailnet>.ts.net"
```

Verify discovery + 401 challenge:

```bash
URL="https://laptop.<tailnet>.ts.net"
curl -sS "$URL/.well-known/oauth-authorization-server" | jq .
curl -sSi -m 5 "$URL/sse" | head -3
```

The discovery doc should advertise `authorization_endpoint`,
`token_endpoint`, and `code_challenge_methods_supported=["S256"]`.
`/sse` should return `401 Unauthorized` with a
`WWW-Authenticate: Bearer …, resource_metadata="…"` header.

## Add the connector in Claude.ai

In Claude.ai → Settings → Connectors → "Add custom connector":

- **Remote MCP server URL**: `https://laptop.<tailnet>.ts.net/sse`
- **Advanced → OAuth Client ID**: paste `client_id` from `oauth.toml`
- **Advanced → OAuth Client Secret**: paste `client_secret` from `oauth.toml`

Claude.ai handles the rest:

1. It hits `/sse`, sees `401` + `WWW-Authenticate: Bearer
   resource_metadata="…"`.
2. It fetches `/.well-known/oauth-protected-resource` and
   `/.well-known/oauth-authorization-server` to discover the endpoints.
3. It opens your browser to `/authorize?…&code_challenge=…&
   code_challenge_method=S256&…`.
4. We redirect to `https://claude.ai/api/mcp/auth_callback?code=…&state=…`.
5. Claude.ai posts to `/oauth/token` with the code + `code_verifier`.
   We verify `SHA256(verifier) == stored code_challenge` and mint a
   1-hour Bearer access token.
6. Claude.ai retries `/sse` with `Authorization: Bearer …` and the
   Zotero tools start working.

Restart Cowork sessions after first connect; the Zotero tools appear
in Cowork's tool list.

## Operational notes

### Token lifecycle

Access tokens live in memory only — they have a 1-hour TTL and a
server restart invalidates every outstanding one. Claude.ai detects
the 401 on the next request and silently re-runs the OAuth flow.
This is also the recommended way to **revoke all sessions**: restart
the launchd job.

```bash
launchctl kickstart -k gui/$(id -u)/com.zotero-mcp.http
```

### Rotating the OAuth client credentials

To re-issue `client_id`/`client_secret` (e.g. you suspect leak):

```bash
rm "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/oauth.toml"
launchctl kickstart -k gui/$(id -u)/com.zotero-mcp.http
zotero-mcp show-credentials
```

Then paste the new `client_id` / `client_secret` into the Claude.ai
connector dialog (Advanced section, same place as initial setup).

### Listening without OAuth (development only)

If `ZOTERO_MCP_OAUTH_ISSUER` is unset and no `oauth.toml` exists, the
server runs without an auth gate. **Do not expose that mode through
Funnel** — the public URL becomes an open Zotero proxy.

## Security notes

- The OAuth client_secret is the only thing protecting the public URL.
  Treat it like a password and rotate after any leak.
- Funnel exposes the URL publicly; anyone with the URL **and** the
  credentials can read your library.
- The HTTP server binds to `127.0.0.1` only, so direct LAN access is
  not possible — clients must go through Tailscale Funnel.
- `oauth.toml` is written with mode `0600`. Keep it that way.
- `/authorize` only redirects to URIs under `https://claude.ai/api/mcp/`
  or `https://claude.com/api/mcp/` — other redirect targets are rejected
  before any code is issued.

## Troubleshooting

- **401 Unauthorized on /sse with no `Authorization` header**: expected
  — that's the OAuth challenge. Claude.ai should follow up by hitting
  `/.well-known/oauth-protected-resource`. If it doesn't, check the
  connector's Server URL field includes `/sse`.
- **Claude.ai shows "Server unreachable" / cached failure**: edit the
  connector and Save (no changes needed) to invalidate cached state and
  force a fresh OAuth handshake.
- **Connection hangs with no events**: check
  `~/Library/Logs/zotero-mcp/http.err.log`. The "SSE buffer flush"
  padding handles most HTTP/2 proxy layers, but if you see no events
  *at all* the upstream Zotero SQLite open may have failed at startup.
- **Funnel says "Funnel is not enabled on your tailnet"**: visit
  `https://login.tailscale.com/admin/settings/features` and toggle it on.
- **`invalid_grant` in logs after `/oauth/token`**: PKCE mismatch or
  expired/already-used code. Usually self-corrects on the next attempt;
  if it persists, rotate the OAuth credentials (above).
