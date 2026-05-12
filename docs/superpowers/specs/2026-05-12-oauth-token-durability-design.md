# Spec: OAuth token durability for zotero-mcp HTTP transport

**Status:** Approved design, ready for plan-writing.
**Author:** Richard Lyon (with Claude Opus 4.7).
**Date:** 2026-05-12.
**Goal:** Use the Cowork / Claude.ai connector for a full working day (or longer) without being forced through the browser-redirect auth flow, while keeping the OAuth gate cryptographically sound.

---

## Problem

The current OAuth implementation in `crates/zotero-mcp/src/oauth.rs` has three properties that combine to force re-authentication far too often:

1. **Access-token TTL is hardcoded at 3600s.** `const TOKEN_TTL_SECS: u64 = 3600;` (line 43).
2. **Token store is in-memory only.** `Inner.tokens: RwLock<HashMap<String, u64>>` (line 166). Any process restart wipes every live token. macOS launchd restarts (log out/in, sleep, system updates, `zotero-mcp setup` re-bootstrap) all wipe tokens.
3. **No refresh-token grant.** `token_handler` only accepts `authorization_code` and `client_credentials`. When an access token expires, the only way to get a new one is to re-run the full PKCE browser flow.

In real Cowork use, this means daily — sometimes hourly — re-authentication.

## Research findings that constrain the design

Two findings from researching the MCP spec and Anthropic's bug tracker shape this design.

### Finding 1: The MCP spec endorses refresh tokens; short access tokens are recommended

- The [2025-06-18 MCP authorization spec](https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization) sequence diagram explicitly returns `Access token (+ refresh token)` from the token endpoint.
- The [draft spec](https://modelcontextprotocol.io/specification/draft/basic/authorization) adds a dedicated *Refresh Tokens* section: clients SHOULD include `refresh_token` in their `grant_types` metadata; clients MUST keep refresh tokens confidential.
- Security considerations: "Authorization servers SHOULD issue short-lived access tokens to reduce the impact of leaked tokens. For public clients, authorization servers MUST rotate refresh tokens as described in OAuth 2.1 Section 4.3.1."

The architecturally correct fix is therefore **short-lived access tokens + rotating refresh tokens**, persisted across restarts.

### Finding 2: Anthropic's `mcp-proxy.anthropic.com` has an open bug — it ignores refresh tokens

- [anthropics/claude-ai-mcp #228](https://github.com/anthropics/claude-ai-mcp/issues/228) (open, last updated 2026-04-30): "OAuth token refresh never attempted for custom connectors via mcp-proxy.anthropic.com". Server logs show zero `/oauth/token` requests after the access token expires; the proxy just keeps sending the expired token.
- [anthropics/claude-code #46328](https://github.com/anthropics/claude-code/issues/46328) (open, last updated 2026-04-24): same bug, confirmed on Claude Code v2.1.100. Direct HTTP path was fixed in v2.1.59 — proxy path remains broken.

The proxy bug is the load-bearing real-world constraint. Spec-correct refresh tokens alone will *not* solve the user's hourly re-auth problem on Cowork until Anthropic ships their fix. We need a workaround on top of the spec-correct foundation.

The workaround chosen (see *Decisions* below) is a long access-token TTL (default 7 days, configurable). The threat model already establishes that OAuth is the access gate — a leaked access token is the only thing standing between an attacker and the Zotero library if they also know the Funnel URL. Refresh-token rotation provides leak-detection regardless of access-token TTL.

## Decisions

| # | Decision | Chosen | Rationale |
|---|----------|--------|-----------|
| 1 | Cowork workaround strategy | Long access TTL + persistence + spec-correct refresh tokens | Hybrid: long TTL covers the Anthropic proxy bug now; refresh tokens are ready when they fix it; persistence solves the launchd-restart problem unconditionally. |
| 2 | Token storage backend | JSON file in config dir, mode 0600 | Simplest. Matches existing `oauth.toml` pattern. ~50 LoC. Cross-platform. SQLite is overkill for ~100 records; Keychain creates an inconsistent secret-handling story alongside the existing 0600 client_secret file. |
| 3 | Default access-token TTL | 7 days (configurable via `oauth.toml`) | Covers a full working week on Cowork. Defensible for a single-user single-tenant deployment behind Tailscale Funnel where the host already has filesystem access to everything. |
| 4 | Default refresh-token TTL | 90 days (configurable via `oauth.toml`) | Industry-standard for user-facing refresh tokens. Long enough to survive vacations; short enough that an old laptop's token loses utility. Idle-only — token rotation on every use means active users effectively never see expiry. |
| 5 | Token rotation policy | Refresh tokens rotate on every use; access tokens do not rotate | OAuth 2.1 §4.3.1 MUST for public clients. Rotation is the leak-detection mechanism: if a stolen refresh token is replayed, we revoke the entire chain. Access tokens don't need rotation because their replay window is bounded by their TTL. |
| 6 | Tokens at rest | Stored as `sha256(token)`, never as plaintext | Defense in depth. If `tokens.json` ever ends up in a backup, snapshot, or accidentally world-readable, the raw bearer values aren't there. Validation uses constant-time comparison of SHA256 digests. |
| 7 | Module split | New `crates/zotero-mcp/src/oauth/token_store.rs` | Current `oauth.rs` is 977 lines. Token persistence has independent invariants (file I/O, hashing, chains, replay detection) and deserves its own boundary. |

## Architecture

### File layout

```
crates/zotero-mcp/src/
├── oauth.rs                    # Discovery + /authorize + /oauth/token (modified)
└── oauth/
    └── token_store.rs          # NEW — file-backed token persistence
```

### `tokens.json` schema (on disk, mode 0600)

```json
{
  "version": 1,
  "client_id_hash": "<hex sha256(client_id)>",
  "access": [
    {
      "token_hash": "<hex sha256(raw_token)>",
      "expires_at": 1746000000,
      "chain_id": "<uuid>"
    }
  ],
  "refresh": [
    {
      "jti": "<uuid>",
      "token_hash": "<hex sha256(raw_token)>",
      "expires_at": 1758000000,
      "chain_id": "<uuid>",
      "consumed_at": null
    }
  ],
  "revoked_chains": ["<uuid>", "..."]
}
```

Path: `~/Library/Application Support/dev.zotero-mcp.zotero-mcp/tokens.json` on macOS;
`~/.config/zotero-mcp/tokens.json` on Linux. (Same `directories::ProjectDirs` convention as `oauth.toml`.)

### `OAuthConfig` extension

```rust
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub issuer: String,
    #[serde(default)] pub access_token_ttl_secs: Option<u64>,    // default 604800 (7d)
    #[serde(default)] pub refresh_token_ttl_secs: Option<u64>,   // default 7776000 (90d)
}
```

`#[serde(default)]` means existing `oauth.toml` files load unchanged. This is a non-breaking change.

### `TokenStore` public surface

```rust
pub struct TokenStore {
    snapshot: tokio::sync::RwLock<Snapshot>,
    path: PathBuf,
    access_ttl: Duration,
    refresh_ttl: Duration,
    client_id_hash: [u8; 32],
}

impl TokenStore {
    /// Load from disk. Drops expired entries. Wipes if `client_id_hash` mismatches.
    /// Treats missing or corrupt files as empty store (logs warning on corrupt).
    pub fn load(path: PathBuf, client_id: &str, access_ttl: Duration, refresh_ttl: Duration)
        -> anyhow::Result<Self>;

    /// Mint a fresh (access, refresh) pair. Pass Some(chain_id) to continue an existing
    /// rotation chain (i.e. during refresh_token grant). Pass None for a new chain
    /// (i.e. during authorization_code grant). Persists to disk before returning.
    pub async fn mint_pair(&self, chain_id: Option<ChainId>)
        -> (String, String, Duration, Duration);

    /// Validate an access token. Hot-path lookup against in-memory map. No disk I/O.
    pub async fn validate_access(&self, raw: &str) -> bool;

    /// Mark a refresh token as consumed and return its chain_id. If the token has
    /// already been consumed, returns Replayed{chain_id} as a leak signal — caller
    /// is expected to revoke the chain.
    pub async fn consume_refresh(&self, raw: &str) -> Result<ChainId, RefreshError>;

    /// Revoke all tokens (access and refresh) sharing the given chain_id.
    /// Persists to disk before returning.
    pub async fn revoke_chain(&self, chain_id: ChainId);
}

pub enum RefreshError {
    Unknown,
    Expired,
    Replayed { chain_id: ChainId },
}
```

That is the complete public surface. Callers cannot manipulate hashes, chains, or the snapshot directly — the store maintains its own invariants.

## Data flow

### Path 1: Initial authorization (`grant_type=authorization_code`)

Existing `handle_authorization_code` in `oauth.rs` is modified to:

1. Validate PKCE as today.
2. Call `token_store.mint_pair(chain_id=None)` to get `(access, refresh, access_ttl, refresh_ttl)`.
3. Return JSON:
   ```json
   {
     "access_token": "<opaque>",
     "token_type": "Bearer",
     "expires_in": 604800,
     "refresh_token": "<opaque>",
     "refresh_expires_in": 7776000,
     "scope": "mcp"
   }
   ```

### Path 2: Silent refresh (`grant_type=refresh_token`) — NEW

New arm in `token_handler`:

1. `token_store.consume_refresh(presented_refresh_token)`:
   - Unknown token → return `invalid_grant`.
   - Expired token → return `invalid_grant`.
   - Already-consumed token → **replay detected**: call `revoke_chain(chain_id)`, log WARN, return `invalid_grant`.
   - Otherwise: mark consumed, return `Ok(chain_id)`.
2. `token_store.mint_pair(chain_id=Some(chain_id))` to continue the same chain.
3. Return same JSON shape as Path 1.

### Path 3: Bearer-token validation on `/sse`, `/message`

Existing `require_bearer_token` middleware in `http_transport.rs`:

1. Extract `Authorization: Bearer <token>`.
2. Replace current `oauth_state.validate_token(token)` call with `token_store.validate_access(token)`.
3. Pass-through on hit. On miss: existing 401 + `WWW-Authenticate` response.

### Path 4: Cold start (the bug-fix that drove this design)

`OAuthState::new(config)` constructor change: instead of building empty in-memory maps, calls `TokenStore::load(config_path, client_id, ttls)`:

1. Read `tokens.json`. If missing → start with empty maps (info log).
2. If unreadable/corrupt → rename aside to `tokens.json.broken-{unix_ts}`, start empty (warn log).
3. If `client_id_hash` mismatches the current config's `client_id` → wipe both maps (warn log). This handles the case where the user re-runs `zotero-mcp setup` and gets a new client_secret pair.
4. Drop entries where `expires_at < now()`.

Authorization codes (`Inner.codes`) remain in-memory only — they have a 5-minute TTL and a server restart during that window is acceptable.

## Failure modes and recovery

### F1. `tokens.json` missing on startup
**Behavior:** Treat as empty store. Log info. First successful auth seeds it.
**Why this matters:** This is the upgrade path and the post-`zotero-mcp setup` path.

### F2. `tokens.json` corrupt or unreadable
**Behavior:** Move bad file aside to `tokens.json.broken-{unix_ts}` for inspection. Start with empty store. Log warning with the parse error.
**Why this matters:** Don't crash the server for a recoverable disk issue.

### F3. Atomic write fails during `persist()` (disk full, permission glitch)
**Behavior:** Log error. Keep tokens in memory only — do NOT fail the in-flight token-mint.
**Why this matters:** The user already authenticated; rejecting the response would be worse than dropping persistence for one cycle. Next mint retries the write.

### F4. Refresh-token replay detected
**Behavior:** Revoke entire chain (all access + refresh tokens). Log WARN with `chain_id`. Return `invalid_grant`.
**Consequence:** Forces a full browser auth flow on next request. This is correct: we cannot distinguish attacker-replay from legitimate-replay, so we assume worst case.

### F5. Concurrent refresh requests racing on the same refresh token
**Behavior:** First request consumes the token successfully. Second triggers replay revocation as if it were an attack.
**Why this is acceptable:** Single-tenant deployment. Anthropic's clients refresh sequentially. The cost of a false-positive (one extra browser re-auth) is much lower than the cost of skipping replay detection.

## Bounded growth

Three sources of self-pruning keep `tokens.json` small:

- **Access tokens** dropped on `load()` and on every `mint_pair()` if `expires_at < now()`. With 7-day TTL and one mint per refresh per day, ≤7 records steady-state.
- **Consumed refresh tokens** dropped under same rule. With daily rotation for 90-day TTL, ≤90 records steady-state.
- **Revoked chains** dropped once their latest refresh token has expired.

Steady-state file size: well under 100KB. No GC daemon needed.

## Security considerations

### S1. Tokens at rest
Stored as `sha256(token)`, not plaintext. File mode 0600. Same-disk-access threat model as existing `oauth.toml` (which also holds the long-lived `client_secret`).

### S2. Refresh-token rotation as leak detection
RFC 6749 §10.4 / OAuth 2.1 §4.3.1 mandate this for public clients. Rotation means each refresh token is one-time-use. If an attacker steals a refresh token, the moment either party (attacker or legitimate client) presents it after the other has consumed it, the chain is revoked. The longer the refresh-token TTL, the more important this mechanism becomes.

### S3. Constant-time token comparison
All token validations use a constant-time equality check on SHA256 digests. Prevents timing oracles from leaking partial token information.

### S4. Logging hygiene
Never log raw token values. Log: `chain_id` (UUID, not derived from secret), `expires_at` (unix timestamp), grant type, outcome. Enforces that production logs are safe to share.

### S5. Atomic file replacement
`persist()` writes to `tokens.json.tmp.<random>`, fsyncs, then renames over `tokens.json`. The rename is atomic on POSIX filesystems — readers either see the old file or the new file, never a torn write. After rename, the temp file's mode 0600 is preserved.

## Testing strategy

### Unit tests in `token_store.rs`

```
mint_pair_returns_two_distinct_tokens
mint_pair_persists_to_disk_atomically
mint_pair_creates_file_with_mode_0600
load_drops_expired_access_tokens
load_drops_expired_refresh_tokens
load_wipes_store_on_client_id_hash_mismatch
load_treats_missing_file_as_empty
load_renames_corrupt_file_aside_and_starts_fresh
validate_access_returns_false_for_expired_token
validate_access_returns_false_for_unknown_token
validate_access_returns_false_for_revoked_chain
consume_refresh_marks_token_consumed_and_returns_chain_id
consume_refresh_replay_returns_replayed_error_with_chain_id
consume_refresh_unknown_returns_unknown_error
consume_refresh_expired_returns_expired_error
revoke_chain_invalidates_all_access_tokens_in_chain
revoke_chain_invalidates_all_refresh_tokens_in_chain
persist_failure_keeps_in_memory_state
tokens_at_rest_are_hashed_not_plaintext
```

### Unit tests added to `oauth.rs`

```
auth_code_response_includes_refresh_token
auth_code_response_includes_refresh_expires_in
refresh_token_grant_returns_new_access_and_refresh
refresh_token_grant_invalidates_old_refresh_token
refresh_token_grant_with_unknown_token_returns_invalid_grant
refresh_token_grant_with_expired_token_returns_invalid_grant
refresh_token_grant_replay_revokes_chain
discovery_advertises_refresh_token_grant_type
```

### Integration test in `http_transport.rs`

```
tokens_survive_oauth_state_recreation
```

The single test that proves the bug is fixed:

1. Build `OAuthState` A with a tempdir backing.
2. Run full auth_code flow → get `(access, refresh)`.
3. Drop A. Build `OAuthState` B from the same tempdir.
4. Assert: `validate_access(access) == true`. **This is the regression test.**
5. POST `/oauth/token` with `grant_type=refresh_token`. Get new pair.
6. Assert: old refresh now rejected; new access valid.

### Manual end-to-end verification

Cannot be automated (needs Anthropic's UI in the loop):

1. `cargo install --path crates/zotero-mcp`.
2. `launchctl bootout … && launchctl bootstrap …` to force a clean restart.
3. Verify `tokens.json` exists at config_dir, mode 0600.
4. `cat tokens.json` — confirm hashes only, no raw secrets.
5. Use Cowork normally for an hour. Confirm: no re-auth.
6. `launchctl bootout/bootstrap` mid-conversation. Confirm: no re-auth on next tool call.
7. (To validate the spec-correct refresh path that Cowork will use post-Anthropic-fix:) connect Claude Code CLI to the connector, let access token expire by running with a temporarily short TTL (e.g. 90s), verify Claude Code sends `grant_type=refresh_token` and gets a new token without browser interaction.

### What we are explicitly NOT testing

- The Anthropic mcp-proxy bug itself — out of our control. Will resolve when they ship their fix.
- Concurrent-refresh false-positive replay detection — single-tenant deployment, accepted in F5.
- Cross-platform Keychain integration — out of scope, see Decision #2.

## Out of scope (deferred)

- **Dynamic Client Registration (RFC 7591)** — current pre-shared client_id/secret continues to work for the single-tenant case.
- **JWT-formatted tokens** — opaque tokens are simpler; JWTs would only matter if we needed stateless multi-instance validation.
- **Multi-tenancy** — single-user, single-tenant by design.
- **Audit logging beyond `tower-http::trace`** — existing infrastructure adequate.
- **Token revocation endpoint (RFC 7009)** — useful but not required by spec, and the user can revoke by deleting `tokens.json` and restarting.

## Migration / rollout

This is a single-user single-tenant deployment, so rollout is "install new binary, restart launchd."

Sequence on the user's machine after the new binary is installed:

1. New binary starts. Sees no `tokens.json` (or sees old one and ignores it — same outcome).
2. The Cowork connector's stored access token (issued by old binary, in-memory only) is already invalid because the old process is gone. Cowork sends it; gets 401.
3. Per the Anthropic proxy bug, Cowork does not refresh. User clicks "reconnect" once → browser auth flow → new auth_code grant → new tokens minted, persisted to disk with 7-day access TTL and 90-day refresh TTL.
4. From this point forward, no re-auth needed for 7 days at a time on Cowork; never again until the refresh token's 90-day idle clock expires (which resets on every successful refresh from clients that *do* refresh, e.g. Claude Code direct or future-fixed Cowork).

Backward compatibility:
- Existing `oauth.toml` loads unchanged (new TTL fields are `#[serde(default)]`).
- `client_credentials` grant retained for headless scripting and tests — unchanged.
- `/.well-known/oauth-authorization-server` discovery doc adds `"refresh_token"` to `grant_types_supported`. This is additive; existing clients won't break.
- `setup.rs` flow unchanged — still generates `oauth.toml` if missing; does not need to know about `tokens.json`.

## Crate version

This change MUST land before the next crates.io publish (per the user's stated constraint that the version bump should wait until after the OAuth fix). The crate version bump itself is a follow-up step, not part of this spec.

## References

- [MCP authorization spec — 2025-06-18](https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization)
- [MCP authorization spec — draft (Refresh Tokens section)](https://modelcontextprotocol.io/specification/draft/basic/authorization)
- [OAuth 2.1 IETF draft v13](https://datatracker.ietf.org/doc/html/draft-ietf-oauth-v2-1-13)
- [RFC 6749 §6 — Refreshing an Access Token](https://datatracker.ietf.org/doc/html/rfc6749#section-6)
- [RFC 6749 §10.4 — Refresh tokens (security)](https://datatracker.ietf.org/doc/html/rfc6749#section-10.4)
- [anthropics/claude-ai-mcp #228 — proxy refresh bug](https://github.com/anthropics/claude-ai-mcp/issues/228)
- [anthropics/claude-code #46328 — same bug, Claude Code report](https://github.com/anthropics/claude-code/issues/46328)
- Existing implementation: `crates/zotero-mcp/src/oauth.rs`, `crates/zotero-mcp/src/http_transport.rs`
- Original auth design: `docs/superpowers/plans/2026-05-11-oauth-authentication.md`
