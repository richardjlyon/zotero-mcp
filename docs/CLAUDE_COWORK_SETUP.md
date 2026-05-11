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
A bearer token in front of the HTTP server gates access.

## Prerequisites

- macOS with Tailscale installed and signed in.
- **Funnel enabled** on your tailnet — one-time admin action at
  `https://login.tailscale.com/admin/settings/features` (toggle "Funnel").
- `zotero-mcp` built and installed: `cargo install --path crates/zotero-mcp`.
- Zotero desktop running (Preferences → Advanced → "Allow other
  applications to communicate with Zotero" enabled).

## One-time setup

1. **Generate a bearer token** and store it in `~/.config/zotero-mcp/token`:

   ```bash
   mkdir -p ~/.config/zotero-mcp ~/Library/Logs/zotero-mcp
   python3 -c "import secrets,sys; sys.stdout.write(secrets.token_hex(32))" \
     > ~/.config/zotero-mcp/token
   chmod 600 ~/.config/zotero-mcp/token
   ```

2. **Install the launchd plist** for the HTTP server (the file is checked
   into this repo at `docs/launchd/com.zotero-mcp.http.plist`; copy or
   recreate it under `~/Library/LaunchAgents/`). It sets
   `ZOTERO_MCP_HTTP=127.0.0.1:8765` and sources the bearer token from
   the file above.

   ```bash
   launchctl load ~/Library/LaunchAgents/com.zotero-mcp.http.plist
   ```

3. **Enable Tailscale Funnel** for port 8765:

   ```bash
   tailscale funnel --bg 8765
   ```

   This prints a public URL like `https://laptop.<tailnet>.ts.net`.
   Funnel state persists across reboots; `tailscaled` re-establishes
   it automatically.

4. **Verify** end to end:

   ```bash
   URL="https://laptop.<tailnet>.ts.net"
   TOKEN=$(cat ~/.config/zotero-mcp/token)
   curl -sN -H "Authorization: Bearer $TOKEN" -m 5 "$URL/sse" | head -3
   ```

   You should see an SSE comment line followed by:

   ```
   event: endpoint
   data: /message?sessionId=<hex>
   ```

## Add the connector in Claude.ai

In Claude.ai → Settings → Connectors → "Add custom connector":

- **Remote MCP server URL**: `https://laptop.<tailnet>.ts.net/sse`
- **Authentication**: Bearer token. Paste the contents of
  `~/.config/zotero-mcp/token`.

Restart Claude (Desktop and any open Cowork sessions). The Zotero tools
should now appear in Cowork's tool list.

## Security notes

- The bearer token is the only access control. Treat it like a password.
- Funnel exposes the URL publicly; anyone with the URL **and** the token
  can read your library.
- The HTTP server binds to `127.0.0.1` only, so direct LAN access is not
  possible — clients must go through Tailscale Funnel.

## Troubleshooting

- **401 Unauthorized**: token mismatch. Re-paste from
  `~/.config/zotero-mcp/token`.
- **Connection hangs with no events**: check
  `~/Library/Logs/zotero-mcp/http.err.log`. The "SSE buffer flush"
  padding handles most HTTP/2 proxy layers, but if you see no events
  *at all* the upstream Zotero SQLite open may have failed at startup.
- **Funnel says "Funnel is not enabled on your tailnet"**: visit
  `https://login.tailscale.com/admin/settings/features` and toggle it on.
