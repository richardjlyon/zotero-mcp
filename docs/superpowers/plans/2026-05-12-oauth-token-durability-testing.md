# OAuth Token Durability — Manual Verification Plan

**Purpose:** Validate the implementation at commit `a7376d3` (final polish) by walking through a complete real-world verification. The automated tests already pass (79 lib + 29 integration). This document covers the things that can only be verified by hand — launchd interaction, the persistence file on disk, Cowork in a browser, and observed behavior over time.

**Estimated time:** ~3 hours of active work, spread across ≥1 working day (Phase 3 requires real-world Cowork use for ≥1 working day).

**Scope:** This plan verifies ONLY the OAuth token-durability feature. It does NOT verify the broader system (Zotero search, item creation, attachments, etc.) — those have their own integration tests and were not touched by this change.

**Document layout:** Phase-numbered with checkboxes. Each phase is independently runnable; you can pause between phases. Each step has:
- **Action:** exactly what to do
- **Expected:** what should happen if working correctly
- **Fail:** what failure looks like and how to interpret it

---

## Pre-flight checklist (15 min)

Run all of this once at the start. If any step fails, stop and investigate before continuing.

### P.1 — Verify you're on the right commit

The final OAuth implementation commit is `a7376d3`. HEAD may be newer if doc commits have been added on top.

- [ ] **Action:**
  ```bash
  cd /Users/rjl/Code/github/zotero-connector
  git log --oneline a7376d3..HEAD
  ```
- [ ] **Expected:** either no output (you're exactly on `a7376d3`), or a list containing only `docs(…)` commits like the testing-plan commit (`docs(test): add comprehensive manual verification plan`).
- [ ] **Fail:** if you see any `feat(…)`, `fix(…)`, `refactor(…)` commits in that list, source code has changed since the verification baseline was set — note them; verification may not cover them.

### P.2 — Verify the test suite passes locally

- [ ] **Action:**
  ```bash
  cargo test --package zotero-mcp 2>&1 | grep "test result"
  ```
- [ ] **Expected:** every line starts with `test result: ok.` and ends with `0 failed; …`. There will be ~30 lines (one per test binary — lib tests + each integration file + doc tests). Don't worry about counts per binary — what matters is that NO line says `FAILED` or any non-zero `failed` count.
- [ ] **Quick fail-fast variant:** `cargo test --package zotero-mcp 2>&1 | grep -E "FAILED|^error" && echo "FAIL" || echo "PASS"` — should print `PASS`.
- [ ] **Fail:** if any line says `failed: N` with N > 0, or if you see `FAILED` anywhere, STOP. Do NOT proceed — fix the failing test first.

### P.3 — Optional safe patch bumps

This is the option-3 patch upgrade for `generic-array` and `matchit`. Skip if you want pure isolation between the OAuth verification and any dep changes.

- [ ] **Action (optional):**
  ```bash
  cargo update -p generic-array -p matchit
  cargo test --package zotero-mcp 2>&1 | grep -E "FAILED|^error" && echo "FAIL" || echo "PASS"
  ```
- [ ] **Expected:** `Cargo.lock` updated; second command prints `PASS`.
- [ ] **Fail:** revert with `git checkout Cargo.lock` and proceed without the patch bumps.
- [ ] **If applied:** `git diff Cargo.lock` should show only `generic-array` and `matchit` lines changing. Commit if you want them included in this round.

### P.4 — Note the baseline you're testing against

Write down (on paper / in a notes app):
- [ ] Current commit SHA: `___________`
- [ ] Date/time of test start: `___________`
- [ ] macOS version: `sw_vers -productVersion` → `___________`
- [ ] Current Cowork URL: `___________` (the connector URL in Claude.ai settings)
- [ ] Hostname (from `tailscale status --json | jq -r .Self.DNSName`): `___________`

---

## Phase 1 — Install & smoke test (15 min)

Goal: confirm the new binary installs, launchd loads it, and the basic HTTP surface is up.

### 1.1 — Install the new binary

- [ ] **Action:**
  ```bash
  cargo install --path crates/zotero-mcp --force
  ```
- [ ] **Expected:** completes with `Installed package zotero-mcp …` and no errors. Takes ~1-2 minutes in release mode.
- [ ] **Fail:** if compile errors appear, your local working copy has uncommitted changes — `git status` to check.

### 1.2 — Verify binary version

- [ ] **Action:**
  ```bash
  which zotero-mcp
  zotero-mcp --help 2>&1 | head -5
  ```
- [ ] **Expected:** path is `~/.cargo/bin/zotero-mcp`; help text mentions "Local-first Zotero bridge".
- [ ] **Fail:** if `which` returns nothing, `~/.cargo/bin` isn't in PATH. Add it before continuing.

### 1.3 — Reload the launchd job

- [ ] **Action:**
  ```bash
  launchctl bootout gui/$UID/com.zotero-mcp.http 2>/dev/null || true
  sleep 1
  launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
  sleep 3
  ```
- [ ] **Expected:** no error output. The `bootout || true` is intentional — the job may not be loaded yet.
- [ ] **Fail:** if `bootstrap` errors with "Bootstrap failed: 5: Input/output error", the plist is malformed — check `~/Library/LaunchAgents/com.zotero-mcp.http.plist` for syntax issues.

### 1.4 — Confirm the server is listening

- [ ] **Action:**
  ```bash
  curl -sI http://127.0.0.1:8765/.well-known/oauth-protected-resource | head -3
  ```
- [ ] **Expected:** `HTTP/1.1 200 OK` with `content-type: application/json`.
- [ ] **Fail:** if connection refused, the server isn't running:
  ```bash
  tail -20 ~/Library/Logs/zotero-mcp/http.err.log
  ```
  Look for a Rust panic or startup error. Common cause: `tokens.json` corruption from a previous run — see Appendix A (Rollback).

### 1.5 — Confirm discovery doc advertises `refresh_token`

- [ ] **Action:**
  ```bash
  curl -s http://127.0.0.1:8765/.well-known/oauth-authorization-server | python3 -m json.tool | grep -A 5 grant_types
  ```
- [ ] **Expected:**
  ```json
  "grant_types_supported": [
      "authorization_code",
      "refresh_token",
      "client_credentials"
  ],
  ```
- [ ] **Fail:** if `refresh_token` is missing, you're running the old binary. Re-run step 1.1.

### 1.6 — Confirm Tailscale Funnel is publishing

- [ ] **Action:**
  ```bash
  tailscale funnel status
  ```
- [ ] **Expected:** lists port 8765 being served. The URL should match the one you noted in P.4.
- [ ] **Fail:** if Funnel is off, run `tailscale funnel --bg 8765` and re-check.

### 1.7 — Test the public URL via curl

- [ ] **Action:** (substitute your Funnel hostname)
  ```bash
  curl -sI https://YOUR-HOST.ts.net/.well-known/oauth-protected-resource | head -3
  ```
- [ ] **Expected:** `HTTP/1.1 200 OK`, `content-type: application/json`.
- [ ] **Fail:** if TLS errors or 502, Funnel isn't bridging to the local server — restart Funnel.

**End of Phase 1.** If all checks passed, the new binary is installed and reachable.

---

## Phase 2 — Token persistence (30 min)

Goal: confirm `tokens.json` is created with correct permissions, that tokens survive restarts, and that the file contains hashes (not raw tokens).

**Important:** this phase deliberately wipes any existing `tokens.json` so you start from a clean state. After this phase you'll need to re-auth once via Cowork's UI.

### 2.1 — Start from a clean token store

- [ ] **Action:**
  ```bash
  TOKENS="$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/tokens.json"
  rm -f "$TOKENS"
  launchctl bootout gui/$UID/com.zotero-mcp.http
  sleep 1
  launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
  sleep 3
  ```
- [ ] **Expected:** no errors. Confirm `tokens.json` doesn't yet exist:
  ```bash
  ls -l "$TOKENS"  # should say "No such file or directory"
  ```
- [ ] **Fail:** if `tokens.json` still exists after `rm`, you're hitting a permission issue. `sudo` is not the right answer — investigate which process holds the file.

### 2.2 — Confirm log mentions empty store on startup

- [ ] **Action:**
  ```bash
  grep -E "tokens.json|TokenStore" ~/Library/Logs/zotero-mcp/http.out.log | tail -5
  ```
- [ ] **Expected:** at least one INFO line containing `no tokens.json found; starting fresh` from the boot we just did.
- [ ] **Fail:** if you see ERROR lines or WARN about corruption, something else is wrong with the file or path. Stop and investigate.

### 2.3 — Trigger the OAuth flow (browser interaction)

- [ ] **Action:**
  1. Open Cowork in your browser.
  2. In the connector list, find the Zotero connector. It should currently show as disconnected or red (because the old in-memory token was lost on the restart).
  3. Click "Reconnect" / "Authorize" — this triggers the OAuth flow.
  4. Browser will redirect to your Funnel URL, then back to Claude with `?code=…&state=…`.
  5. Cowork's backend POSTs `/oauth/token` to mint tokens.
- [ ] **Expected:** after the flow completes (a few seconds), the connector shows green/connected.
- [ ] **Fail modes:**
  - Browser shows "invalid_redirect_uri": check `ALLOWED_REDIRECT_URI_PREFIXES` in `oauth.rs` matches what Cowork uses.
  - Connector stays red: tail `http.err.log` for the actual error.
  - Browser redirect loops: usually a `state` mismatch — clear cookies and retry.

### 2.4 — Verify `tokens.json` was created

- [ ] **Action:**
  ```bash
  ls -la "$TOKENS"
  ```
- [ ] **Expected:** file exists, mode `-rw-------` (0600), owner is you, size is non-zero (~500-2000 bytes for one pair).
- [ ] **Fail:** if perms are NOT 0600, this is a security bug — STOP and investigate `atomic_write_0600` in `token_store.rs`. If perms are wider than 0600 the security spec is violated.

### 2.5 — Verify tokens at rest are hashed (NOT plaintext)

- [ ] **Action:**
  ```bash
  cat "$TOKENS" | python3 -m json.tool
  ```
- [ ] **Expected:**
  ```json
  {
      "version": 1,
      "client_id_hash": "<64-char hex>",
      "access": [
          {
              "token_hash": "<64-char hex>",
              "expires_at": <unix ts>,
              "chain_id": "<32-char hex>"
          }
      ],
      "refresh": [
          {
              "token_hash": "<64-char hex>",
              "expires_at": <unix ts>,
              "chain_id": "<32-char hex>",
              "consumed_at": null
          }
      ],
      "revoked_chains": []
  }
  ```
- [ ] **Fail:** if ANY field with name `_token` (without `_hash`) appears in the file, that's a security violation — STOP. The implementation is leaking raw token values to disk.
- [ ] **Sanity check:** all `token_hash` and `client_id_hash` values should be exactly 64 hex characters (SHA-256 hex). `chain_id` should be 32 hex characters.

### 2.6 — Use a Zotero tool from Cowork (smoke test)

- [ ] **Action:** in the active Cowork session, ask Claude to do something simple, e.g., "search my Zotero library for items containing 'machine learning'".
- [ ] **Expected:** Cowork makes the tool call, gets results, displays them.
- [ ] **Fail:** if the call 401s, check that the access token validated:
  ```bash
  grep "bearer auth failed" ~/Library/Logs/zotero-mcp/http.err.log | tail -3
  ```
  If you see entries, the token in Cowork's request doesn't match what's in `tokens.json`. Possible cause: the access token in Cowork is stale from a previous session that wasn't cleared on reconnect.

### 2.7 — Bounce launchd mid-conversation (the regression test)

This is the load-bearing test: BEFORE this fix, restarting the daemon would have invalidated the token and forced a fresh browser auth. AFTER this fix, the token survives.

- [ ] **Action:**
  ```bash
  launchctl bootout gui/$UID/com.zotero-mcp.http
  sleep 2
  launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
  sleep 3
  ```
  Then immediately switch to your active Cowork conversation (don't refresh the browser) and ask Claude to do another Zotero tool call — e.g., "now show me the abstract of the first one".
- [ ] **Expected:** the tool call succeeds without any reconnect prompt or browser redirect. THIS IS THE FIX.
- [ ] **Fail:** if the call 401s and Cowork asks to reconnect, the persistence didn't work. Check:
  ```bash
  ls -l "$TOKENS"                          # file should still exist
  grep "validate_access" ~/Library/Logs/zotero-mcp/http.out.log | tail -5
  ```

### 2.8 — Bounce mid-conversation a second time, with a 30-second wait

This catches subtle issues with SSE session reconnect.

- [ ] **Action:** repeat 2.7 but wait 30 seconds before bouncing, and another 30 seconds before the next tool call.
- [ ] **Expected:** still works without re-auth.

**End of Phase 2.** The core regression is verified. You can safely use Cowork normally from this point.

---

## Phase 3 — Real-world Cowork use (≥1 working day, passive)

Goal: confirm tokens last for a full working day without re-auth. This phase runs in the background — you just use Cowork normally and watch for any re-auth prompts.

### 3.1 — Baseline: note the time and the current token's expiry

- [ ] **Action:**
  ```bash
  date
  cat "$TOKENS" | python3 -c "import json,sys,datetime; d=json.load(sys.stdin); [print(datetime.datetime.fromtimestamp(a['expires_at']).isoformat(), 'access') for a in d['access']]; [print(datetime.datetime.fromtimestamp(r['expires_at']).isoformat(), 'refresh') for r in d['refresh']]"
  ```
- [ ] **Expected:** access expiry is ~7 days from now; refresh expiry is ~90 days.
- [ ] **Record:** time now: `___________`, access expires at: `___________`.

### 3.2 — Use Cowork actively for 1-2 hours

Use the connector for normal work. Make Zotero queries periodically. There's no specific test — just observe.

- [ ] **Expected:** no re-auth prompts. No connector errors. Tool calls work.
- [ ] **Fail:** if Cowork shows "Reconnect" at any point, note the time and check the logs:
  ```bash
  grep -E "WARN|ERROR" ~/Library/Logs/zotero-mcp/http.{err,out}.log | tail -20
  ```

### 3.3 — Let the system sleep / lid close for 30+ minutes

- [ ] **Action:** close the laptop lid, walk away for ≥30 minutes. When you return, open the lid and resume your Cowork session.
- [ ] **Expected:** Cowork tool calls still work without re-auth. The OS sleep should not invalidate the token store (the file is on disk; the server resumes from where launchd suspended it or whenever the next request arrives).
- [ ] **Fail:** if you're prompted to reconnect after sleep/wake, check:
  ```bash
  grep -E "tokens.json|TokenStore" ~/Library/Logs/zotero-mcp/http.out.log | tail -10
  ```

### 3.4 — Across an overnight break

- [ ] **Action:** stop working, leave the system idle (or shut it). Return the next day.
- [ ] **Expected:** open Cowork, the connector is still connected, tool calls work.
- [ ] **Fail:** if you must reconnect, note whether it was due to a system reboot (which is expected to preserve tokens.json) or some other reason.

### 3.5 — Verify file growth is bounded

- [ ] **Action:** after a day of use, check file size and contents:
  ```bash
  wc -c "$TOKENS"
  cat "$TOKENS" | python3 -c "import json,sys; d=json.load(sys.stdin); print('access:', len(d['access']), 'refresh:', len(d['refresh']), 'revoked:', len(d['revoked_chains']))"
  ```
- [ ] **Expected:** file size < 5 KB; access count ≤ 2 (one is normal, two during rare race conditions); refresh count = same as access count (because Cowork doesn't refresh due to the Anthropic bug, so refresh records accumulate only if Cowork did a fresh auth_code flow); revoked count = 0 (no replays).
- [ ] **Fail:** if access count is high (>5) or file size is > 50 KB, something is generating more tokens than expected. Investigate the log for repeated auth_code grants:
  ```bash
  grep "OAuth token pair minted" ~/Library/Logs/zotero-mcp/http.out.log | wc -l
  ```

**End of Phase 3.** The user-visible success criterion ("a working day without re-auth on Cowork") is met if Phase 3 completes without any reconnect prompt.

---

## Phase 4 — Refresh-path verification on Claude Code direct (30 min)

Goal: prove that the spec-correct refresh flow works for clients that DO refresh — namely Claude Code direct HTTP (not via the broken Anthropic proxy). This validates that when Anthropic eventually fixes their proxy, Cowork will get silent refresh too.

This phase only matters if you want to verify the refresh code itself. Skip if you're satisfied with Phase 3's Cowork-survives-restarts result.

### 4.1 — Lower the access TTL to make the refresh observable

- [ ] **Action:** edit `~/Library/Application Support/dev.zotero-mcp.zotero-mcp/oauth.toml` and add (or modify):
  ```toml
  access_token_ttl_secs = 90
  ```
  Then bounce launchd:
  ```bash
  launchctl bootout gui/$UID/com.zotero-mcp.http
  sleep 1
  launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
  sleep 3
  ```
- [ ] **Expected:** server picks up the new TTL. Confirm:
  ```bash
  curl -s http://127.0.0.1:8765/.well-known/oauth-authorization-server | python3 -m json.tool
  ```
  (The discovery doc doesn't expose TTL but the server should still respond.)
- [ ] **Fail:** if launchd doesn't restart cleanly, check `http.err.log`.

### 4.2 — Connect Claude Code CLI directly to the connector

Configure Claude Code to use the Zotero MCP server via direct HTTP (NOT via the claude.ai proxy). The exact config depends on your Claude Code setup — typically you'd add to `~/.claude/mcp_servers.json` or equivalent:

- [ ] **Action:** add the connector to Claude Code's MCP config pointing at `https://YOUR-HOST.ts.net/sse` with the same client_id/client_secret as Cowork. Start Claude Code.
- [ ] **Expected:** Claude Code completes the OAuth flow (may use the same tokens.json or do a fresh auth — both fine).

### 4.3 — Do one tool call, then WAIT 100 SECONDS, then do another

- [ ] **Action:** in Claude Code, ask for any Zotero tool result. Then literally wait 100 seconds (over the 90s TTL). Then do another tool call.
- [ ] **Expected:** the second tool call succeeds. Behind the scenes, Claude Code's HTTP client should have either:
  - Got a 401 from the expired access token, then exchanged the refresh token for a new pair, then retried — OR
  - Proactively refreshed before the expiry.
- [ ] **Verify:** check the log for the refresh event:
  ```bash
  grep "OAuth token pair minted (refreshed)" ~/Library/Logs/zotero-mcp/http.out.log
  ```
  You should see a line with `grant=refresh_token` and a `chain_id`.
- [ ] **Fail:** if no `(refreshed)` line appears AND the second tool call also worked, it might mean Claude Code's HTTP client is proactively refreshing before expiry (also valid). If the second tool call fails with 401 AND no refresh line appears, then either (a) Claude Code isn't refreshing, or (b) the refresh request is failing — check for `OAuth grant rejected` in the log.

### 4.4 — Verify chain_id continuity

If refresh worked, the new access token should share a `chain_id` with the original:

- [ ] **Action:**
  ```bash
  cat "$TOKENS" | python3 -c "import json,sys; d=json.load(sys.stdin); chains=set(); [chains.add(a['chain_id']) for a in d['access']]; [chains.add(r['chain_id']) for r in d['refresh']]; print('unique chain_ids:', len(chains))"
  ```
- [ ] **Expected:** 1 unique chain_id (all tokens belong to the same rotation chain from the original auth_code grant).
- [ ] **Fail:** if 2+ chain_ids appear, you've had multiple separate auth_code grants — the refresh path didn't fire and a fresh browser auth happened instead.

### 4.5 — Restore the normal access TTL

- [ ] **Action:** remove the `access_token_ttl_secs = 90` line from `oauth.toml` (or change to `604800`). Bounce launchd.
- [ ] **Expected:** server restarts with 7-day default.

**End of Phase 4.** Refresh path is verified for spec-compliant clients.

---

## Phase 5 — Security spot checks (15 min)

Goal: confirm no raw tokens leak anywhere they shouldn't, and that revocation works.

### 5.1 — Search logs for raw token values

- [ ] **Action:** pick a current access token's hash from `tokens.json`. You can't easily get the raw token (it's hashed), but the absence of any raw token-shape strings in logs is what matters. Check that logs only mention hashes/chain_ids and never the raw 32-char hex values that look like opaque tokens:
  ```bash
  # Hash-shape strings (acceptable in logs):
  grep -oE '[0-9a-f]{64}' ~/Library/Logs/zotero-mcp/http.out.log | head -5
  # Raw-token-shape strings (NOT acceptable - 32-char hex appearing as Authorization values or response bodies):
  grep -oE 'Bearer [0-9a-f]{32}' ~/Library/Logs/zotero-mcp/http.out.log | head -5
  ```
- [ ] **Expected:** no `Bearer <hex>` strings in the log. (The TraceLayer DOES log request headers but should redact Authorization. If it doesn't, that's a separate hygiene issue worth fixing — but not part of this verification.)
- [ ] **Note:** SHA-256 hashes (64-char hex) in chain_id log fields are fine — those are derived values.

### 5.2 — Verify the file is unreadable by other users

- [ ] **Action:**
  ```bash
  stat -f "%Sp %u %g" "$TOKENS"
  ```
- [ ] **Expected:** `-rw------- <your_uid> <your_gid>` — readable only by you.
- [ ] **Fail:** if mode is `-rw-r--r--` or wider, that's a security bug. Should not happen because `atomic_write_0600` sets mode explicitly.

### 5.3 — Manual revocation works

- [ ] **Action:**
  ```bash
  rm "$TOKENS"
  launchctl bootout gui/$UID/com.zotero-mcp.http
  sleep 1
  launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
  sleep 3
  ```
  Then try to use Cowork — it should now require re-auth.
- [ ] **Expected:** Cowork tool call 401s, prompts to reconnect, you do the OAuth flow again, new `tokens.json` appears.
- [ ] **Fail:** if Cowork keeps working without re-auth, the in-memory state survived somehow — investigate. This shouldn't happen because `bootout` kills the process.

**End of Phase 5.** Note: after this, you've done a fresh auth, so you can continue using Cowork normally.

---

## Phase 6 — Failure mode tests (45 min)

Goal: confirm the server handles disk-level oddities gracefully (per spec failure modes F1-F4).

### 6.1 — F1: Missing tokens.json on startup

Already tested implicitly in 2.1. Specifically:
- [ ] **Verified earlier:** server starts with missing file → empty store → first OAuth grant creates the file.
- [ ] **Log signature:** `no tokens.json found; starting fresh`.

### 6.2 — F2: Corrupt tokens.json

- [ ] **Action:**
  ```bash
  cp "$TOKENS" "/tmp/tokens-good.json"            # backup
  echo "this is not json" > "$TOKENS"
  launchctl bootout gui/$UID/com.zotero-mcp.http
  sleep 1
  launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
  sleep 3
  ```
- [ ] **Verify:**
  ```bash
  ls "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/"
  grep "tokens.json corrupt" ~/Library/Logs/zotero-mcp/http.out.log | tail -1
  ```
- [ ] **Expected:** original `tokens.json` was renamed to `tokens.json.broken-<unix_ts>`. A new `tokens.json` does NOT exist yet (will be created on next auth). Server is running.
- [ ] **Fail:** if the server panicked / failed to start, the F2 path is broken. Check `http.err.log`.
- [ ] **Restore:**
  ```bash
  rm "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/tokens.json.broken-"*
  cp "/tmp/tokens-good.json" "$TOKENS"
  chmod 0600 "$TOKENS"
  launchctl bootout gui/$UID/com.zotero-mcp.http
  sleep 1
  launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
  ```
  Verify Cowork works again without re-auth.

### 6.3 — F2b: Tokens.json IO error (NEW — the polish fix)

This validates the fix in commit `a7376d3`. Make the file unreadable temporarily.

- [ ] **Action:**
  ```bash
  chmod 0000 "$TOKENS"
  launchctl bootout gui/$UID/com.zotero-mcp.http
  sleep 1
  launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
  sleep 3
  ```
- [ ] **Verify:**
  ```bash
  grep "could not read tokens.json" ~/Library/Logs/zotero-mcp/http.out.log | tail -1
  curl -sI http://127.0.0.1:8765/.well-known/oauth-protected-resource | head -1
  ```
- [ ] **Expected:** log shows `could not read tokens.json (transient I/O error?); starting fresh`. Server is RUNNING (HTTP 200 from discovery). Pre-fix this would have crashed the server on startup.
- [ ] **Fail:** if the server is not responding, the IO-error fallthrough fix didn't work. Check `http.err.log` for the actual error.
- [ ] **Restore:**
  ```bash
  chmod 0600 "$TOKENS"
  launchctl bootout gui/$UID/com.zotero-mcp.http
  sleep 1
  launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
  ```

### 6.4 — F4 + client_id_hash mismatch: regenerate oauth.toml

This simulates what happens if you re-run `zotero-mcp setup` and get a fresh client_secret pair.

- [ ] **Action:**
  ```bash
  cp "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/oauth.toml" "/tmp/oauth-good.toml"
  cp "$TOKENS" "/tmp/tokens-good-2.json"
  rm "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/oauth.toml"
  launchctl bootout gui/$UID/com.zotero-mcp.http
  sleep 1
  launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
  sleep 3
  ```
- [ ] **Verify:**
  ```bash
  ls "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/oauth.toml"  # new one generated
  grep -E "tokens.json client_id_hash mismatch|generated OAuth credentials" ~/Library/Logs/zotero-mcp/http.out.log | tail -3
  cat "$TOKENS" | python3 -c "import json,sys; d=json.load(sys.stdin); print('access:', len(d['access']), 'refresh:', len(d['refresh']))"
  ```
- [ ] **Expected:** server generates a new `oauth.toml` with new client_id; loads `tokens.json`; sees `client_id_hash` mismatch; wipes the store (access and refresh both 0). Log confirms `client_id_hash mismatch; wiping`.
- [ ] **Fail:** if the old tokens are still valid (access count > 0), the wipe didn't fire. That's a security bug — the old tokens were issued under a different client_id and should not be honored.
- [ ] **Restore (important — Cowork's credentials are now stale):**
  ```bash
  cp "/tmp/oauth-good.toml" "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/oauth.toml"
  cp "/tmp/tokens-good-2.json" "$TOKENS"
  chmod 0600 "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/oauth.toml" "$TOKENS"
  launchctl bootout gui/$UID/com.zotero-mcp.http
  sleep 1
  launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
  ```
  Verify Cowork works again without re-auth.

**End of Phase 6.** Failure modes covered.

---

## Phase 7 — Stdio regression (15 min)

Goal: confirm the HTTP/OAuth changes didn't break the stdio mode that Claude Desktop uses.

The OAuth code only runs when `ZOTERO_MCP_HTTP` is set. Without it, the binary should run as a stdio MCP server unchanged.

### 7.1 — Test stdio mode manually

- [ ] **Action:**
  ```bash
  unset ZOTERO_MCP_HTTP
  unset ZOTERO_MCP_OAUTH_ISSUER
  echo '{"jsonrpc":"2.0","method":"initialize","id":1,"params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.0.1"}}}' | timeout 3 zotero-mcp 2>/dev/null | head -1
  ```
- [ ] **Expected:** a JSON response on stdout containing `"jsonrpc":"2.0"` and `"result"`. No errors.
- [ ] **Fail:** if no output, or panic on stderr, stdio mode is broken — investigate `main.rs` for accidental coupling between stdio path and OAuth init.

### 7.2 — Confirm Claude Desktop's connector still works

- [ ] **Action:** open Claude Desktop, navigate to a conversation that uses the Zotero MCP server (via stdio config). Ask for a Zotero tool result.
- [ ] **Expected:** works exactly as before this work.
- [ ] **Fail:** if Claude Desktop reports the MCP server crashed or won't start, check whether your `claude_desktop_config.json` points to the updated binary at `~/.cargo/bin/zotero-mcp` and that that binary is the new one.

**End of Phase 7.** Stdio is unaffected.

---

## Phase 8 — Sign-off checklist (5 min)

Run through this final list. All boxes must be checked for the verification to be complete.

### Core feature
- [ ] `tokens.json` is created at `~/Library/Application Support/dev.zotero-mcp.zotero-mcp/tokens.json`
- [ ] File mode is `0600`
- [ ] File contains hashes (`token_hash`), never raw bearer values
- [ ] Access tokens survive `launchctl bootout/bootstrap` cycle (Phase 2.7)
- [ ] Cowork used for ≥1 working day without re-auth prompt (Phase 3.4)
- [ ] System sleep/wake doesn't invalidate tokens (Phase 3.3)
- [ ] Refresh path observable on Claude Code direct (Phase 4.3) — OR skipped intentionally

### Failure handling
- [ ] Corrupt `tokens.json` is renamed aside; server keeps running (Phase 6.2)
- [ ] Unreadable `tokens.json` is logged and ignored; server keeps running (Phase 6.3)
- [ ] `client_id_hash` mismatch wipes the store (Phase 6.4)

### Regressions
- [ ] All 79 lib tests + 29 integration tests pass (P.2)
- [ ] Stdio mode still works (Phase 7.1)
- [ ] Claude Desktop connector still works (Phase 7.2)

### Security
- [ ] No raw tokens visible in `http.out.log` or `http.err.log` (Phase 5.1)
- [ ] File perms are 0600 (Phase 5.2)
- [ ] Manual revocation (delete file + restart) forces re-auth (Phase 5.3)

If all boxes are checked: **verification complete**. The OAuth durability feature is fit for the v0.x version bump and crates.io publish.

If any box is unchecked, note which and why. Some failures may be acceptable (e.g., Phase 4 skipped on purpose) — others are blockers.

---

## Appendix A — Rollback procedure

If at any point during this verification the system is in a broken state that you can't fix:

### A.1 — Revert the OAuth durability change entirely

- [ ] **Action:**
  ```bash
  cd /Users/rjl/Code/github/zotero-connector
  git log --oneline 9b6cbf7..HEAD
  # Note the SHAs of the 16 OAuth commits.
  ```
  Identify the parent of the first OAuth commit (`9b6cbf7` — the plan commit). The last commit before any OAuth code is the one before `d3b3457`. To revert:
  ```bash
  git revert --no-commit d3b3457..HEAD   # revert all OAuth commits as a series
  git commit -m "revert: roll back OAuth token durability work pending investigation"
  cargo install --path crates/zotero-mcp --force
  launchctl bootout gui/$UID/com.zotero-mcp.http
  sleep 1
  launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist
  ```
- [ ] **Expected:** server runs on the OLD code. Tokens are in-memory only. Re-auth required on every restart (the original behavior).
- [ ] **Cleanup:**
  ```bash
  rm -f "$TOKENS"  # the new persistence file is unused on the old code; safe to delete
  ```

### A.2 — Forward-fix instead of rolling back

If you find a specific bug during verification, capture it cleanly:
1. Take a `git diff > /tmp/state.diff` of any uncommitted changes.
2. Note the exact reproducer (commands + expected vs actual).
3. Open a new session and let me fix it.
4. Re-run only the affected Phase from this plan.

---

## Appendix B — Reference

### Paths
- **Binary:** `~/.cargo/bin/zotero-mcp`
- **OAuth client config:** `~/Library/Application Support/dev.zotero-mcp.zotero-mcp/oauth.toml`
- **Token persistence:** `~/Library/Application Support/dev.zotero-mcp.zotero-mcp/tokens.json`
- **Launchd plist:** `~/Library/LaunchAgents/com.zotero-mcp.http.plist`
- **Logs:** `~/Library/Logs/zotero-mcp/http.out.log`, `~/Library/Logs/zotero-mcp/http.err.log`
- **HTTP bind:** `127.0.0.1:8765`
- **Public URL:** `https://<your-tailscale-hostname>.ts.net/`

### Useful commands
```bash
# Show current OAuth credentials (for copy-paste into Cowork's connector config)
zotero-mcp show-credentials

# Full status snapshot
zotero-mcp status

# Watch logs in real time
tail -f ~/Library/Logs/zotero-mcp/http.out.log

# Quick bounce (the canonical "restart server" command)
launchctl bootout gui/$UID/com.zotero-mcp.http && \
  sleep 1 && \
  launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.zotero-mcp.http.plist

# Pretty-print the persistence file
cat "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/tokens.json" | python3 -m json.tool

# Decode access-token expiry
python3 -c "import json,datetime; d=json.load(open('$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/tokens.json')); [print(datetime.datetime.fromtimestamp(a['expires_at']), 'access') for a in d['access']]; [print(datetime.datetime.fromtimestamp(r['expires_at']), 'refresh') for r in d['refresh']]"
```

### Log signatures to recognize

| Signature | What it means |
|---|---|
| `no tokens.json found; starting fresh` | First start after install, or post-rm — empty store, OK |
| `tokens.json corrupt … renamed aside` | F2 fired — JSON was malformed, backup created |
| `could not read tokens.json (transient I/O error?)` | F2b fired — IO error, store empty, server still running |
| `tokens.json client_id_hash mismatch; wiping` | F4 fired — oauth.toml regenerated, old tokens discarded |
| `OAuth token pair minted` (grant=authorization_code) | New auth_code flow — browser auth happened |
| `OAuth token pair minted (refreshed)` (grant=refresh_token) | Refresh flow — should be common on Claude Code direct, rare on Cowork |
| `refresh-token replay detected; revoking chain` | Replay attack signal — chain revoked |
| `OAuth grant rejected` | Token grant denied (expired, replayed, unknown, malformed) |
| `bearer auth failed` | A request to `/sse` or `/message` had a missing/expired/unknown token — followed by 401 challenge |

### Spec and plan
- Design spec: `docs/superpowers/specs/2026-05-12-oauth-token-durability-design.md`
- Implementation plan: `docs/superpowers/plans/2026-05-12-oauth-token-durability.md`
- This testing plan: `docs/superpowers/plans/2026-05-12-oauth-token-durability-testing.md`

---

## Appendix C — Cleanup after verification

After all phases pass, you may want to:

- [ ] Remove the temporary backup files in `/tmp/` if you used them in Phase 6:
  ```bash
  rm -f /tmp/tokens-good.json /tmp/tokens-good-2.json /tmp/oauth-good.toml
  ```
- [ ] Confirm `oauth.toml` doesn't have a left-over `access_token_ttl_secs = 90` from Phase 4. Should be either absent (= 7-day default) or set to whatever value you want:
  ```bash
  grep -E "ttl_secs" "$HOME/Library/Application Support/dev.zotero-mcp.zotero-mcp/oauth.toml"
  ```
- [ ] Commit the safe patch bumps if you ran P.3 and want them.
- [ ] If everything is green, proceed to the deferred crate version bump + crates.io publish.

Then, in a fresh session, start the `rmcp 0.1.5 → 1.x` upgrade as its own brainstorm → spec → plan → execute cycle.
