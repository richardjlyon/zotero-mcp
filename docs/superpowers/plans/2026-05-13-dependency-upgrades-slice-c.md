# Dependency Upgrades — Slice C Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring `rmcp 0.1.5 → 1.7.0` and `schemars 0.8 → 1.x` to their latest releases in one atomic commit. Minimal port: convert the `#[tool(...)]` macro syntax in `server.rs`, port the one schemars-API consumer (`tests/schema_shape.rs`), and verify everything else compiles.

**Architecture:** The two crates are coupled (rmcp 0.1 requires schemars 0.8 in its public macro output; rmcp 1.x requires schemars 1.0), so they must move together. One atomic commit; no per-crate splits. Compile-error-driven: iterate `cargo build` and apply documented fix patterns until clean. The implementer has the full migration map in this plan and the spec's escalation triggers.

**Tech Stack:** Rust, Cargo. rmcp (MCP Rust SDK from `modelcontextprotocol/rust-sdk`), schemars (JSON Schema generator from `gresau/schemars`).

**Spec:** `docs/superpowers/specs/2026-05-13-dependency-upgrades-slice-c-design.md` (commit `4283852`).

---

## File Structure

| File | Purpose | Action |
|---|---|---|
| `Cargo.toml` (workspace, line 21) | Holds the `rmcp` version + features | Modify |
| `crates/zotero-mcp/Cargo.toml` | Holds the `schemars` version | Modify |
| `Cargo.lock` | Pinned dependency graph | Modify (joint update) |
| `crates/zotero-mcp/src/server.rs` | 69 `#[tool(...)]` attribute sites + manual `ServerHandler` impl | Modify (macro-shape port) |
| `crates/zotero-mcp/src/main.rs` | Uses `rmcp::ServiceExt` for runtime wiring | Verify; modify only if import path shifted |
| `crates/zotero-mcp/src/http_transport.rs` | Uses `rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage}` + tuple transport | Verify; modify only if type paths shifted |
| `crates/zotero-mcp/src/tools/{search,attachments,citations,enrichment,writes}.rs` | 27 `#[derive(JsonSchema)]` sites; 5 files use `rmcp::Error::{internal_error, invalid_params}` | Verify; modify only if a derive or error constructor changed shape |
| `crates/zotero-mcp/tests/schema_shape.rs` | 3 tests using `schemars::schema_for!` + struct-field navigation | Modify (port to `Schema = serde_json::Value` API) |

---

## Pre-flight: confirm clean state

**Files:**
- Read-only: working tree

- [ ] **Step 1: Confirm clean tree on `main`**

Run: `cd /Users/rjl/Code/github/zotero-connector && git status`

Expected: `nothing to commit, working tree clean` and branch `main`.

If dirty, stop and resolve before starting the slice.

- [ ] **Step 2: Capture baseline test results**

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo test -p zotero-mcp 2>&1 | grep "^test result:" | sort | uniq -c`

Expected: every line `ok`, no `FAILED`. Lib tests should still be `105 passed; 0 failed` (the Slice B baseline). Write this number down — Slice C's lib-test count may shift up or down by the 3 tests in `tests/schema_shape.rs` if their assertions need to be rewritten under schemars 1.x. The new number gets recorded in the commit body.

- [ ] **Step 3: Record the current pre-flight SHA**

Run: `cd /Users/rjl/Code/github/zotero-connector && git rev-parse HEAD`

Expected: `4283852` (the Slice C spec commit) — or, if this slice is being re-run after a checkpoint, whatever the current HEAD is. Write down the SHA. This is the rollback point if the joint bump triggers escalation.

---

## Task 1: Joint atomic bump `rmcp 0.1 → 1.7` + `schemars 0.8 → 1.x`

**Files (all changes land in one commit at Step 14):**
- Modify: `Cargo.toml` (workspace, line 21)
- Modify: `crates/zotero-mcp/Cargo.toml` (the `schemars = "0.8"` line)
- Modify: `Cargo.lock`
- Modify: `crates/zotero-mcp/src/server.rs` (macro shape)
- Modify: `crates/zotero-mcp/tests/schema_shape.rs` (Schema API)
- Possibly modify: `crates/zotero-mcp/src/main.rs`, `crates/zotero-mcp/src/http_transport.rs`, `crates/zotero-mcp/src/tools/*.rs` — verify-only unless an import path or error constructor shifted; the spec lists those as in-bounds one-line fixes.

**Surface inventory** (from the Explore agent's report; spec records the same):

- **rmcp** — 69 `#[tool(...)]` attribute sites in `server.rs`: 1 struct-level `#[tool(tool_box)]` (line 29), 1 struct-level on the `ServerHandler` impl (line 280), 40 method-level `#[tool(description = "...")]` (lines 35–276), 27 parameter-level `#[tool(aggr)]` (lines 48, 58, 66, 74, 84, 92, 100, 108, 116, 124, 132, 140, 162, 167, 172, 177, 182, 187, 196, 203, 211, 219, 225, 232, 239, 246, 251, 256, 263, 268, 275). `ServiceExt::serve`/`.waiting` used in `main.rs` and `http_transport.rs`. `rmcp::Error::{internal_error, invalid_params}` used in all 5 `tools/*.rs` modules.
- **schemars** — 27 bare `#[derive(JsonSchema)]` derives across 5 files (`tools/{search,attachments,citations,enrichment,writes}.rs`), no field-level `#[schemars(...)]` attrs, no custom `impl JsonSchema` blocks. `schema_for!` used in 3 tests in `tests/schema_shape.rs`.

**Key API changes confirmed at plan-time** (sources: rmcp 1.5 docs via Context7, schemars migration guide via Context7):

| Change | Impact on this codebase |
|---|---|
| `#[tool(tool_box)]` (struct-level) → `#[tool_router]` | Required — 1 site at `server.rs:29` |
| `#[tool(aggr)] args: T` (parameter attribute) → `Parameters(args): Parameters<T>` (Rust pattern destructure on the parameter, plus a `use rmcp::handler::server::tool::Parameters` import) | Required — 27 sites in `server.rs` |
| `#[tool(description = "...")]` (method-level) | Stable; unchanged |
| Manual `impl ServerHandler for ZoteroServer { ... }` block with custom `get_info()`, `list_resources`, `read_resource` | Stable. Optional cosmetic improvement: `#[tool_handler]` macro can auto-generate `get_info()`, but the manual block is also fine. If the macro and manual override methods conflict, keep manual — non-blocking. |
| `rmcp::Error::{internal_error, invalid_params}` constructors | Stable; per rmcp 1.5 docs the `Error` type and these constructors remain |
| `rmcp::ServiceExt::serve` / `.waiting` | Stable; unchanged in 1.x |
| `rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage}` | Stable; unchanged path |
| rmcp `schemars` feature now opt-in (was implicit via `server` defaults) | Required Cargo.toml feature edit — add `"schemars"` to the rmcp features list |
| `rmcp-macros` re-export of `#[tool]` is now under crate feature `macros` | Required — add `"macros"` to the rmcp features list (was implicit) |
| schemars `Schema` is now a `serde_json::Value` wrapper (was a typed struct) | Required test edit — `tests/schema_shape.rs` must use `schema.get(...)` / `.as_str()` Value navigation, not `.schema.instance_type` / struct field access |
| schemars `schema_for!` macro | Stable name; return type changed from `RootSchema` to `Schema` |
| Bare `#[derive(JsonSchema)]` on simple structs | Stable; should compile unchanged |

### Step 1: Bump the workspace `Cargo.toml` `rmcp` line

In `/Users/rjl/Code/github/zotero-connector/Cargo.toml`, find line 21:

```toml
rmcp = { version = "0.1", features = ["server", "transport-io"] }
```

Change to:

```toml
rmcp = { version = "1", features = ["server", "macros", "schemars", "transport-io"] }
```

Three changes in one line:
- `version = "0.1"` → `version = "1"`
- Add `"macros"` to features (was implicit in 0.1; explicit in 1.x).
- Add `"schemars"` to features (rmcp 1.x split it out of the `server` defaults).

### Step 2: Bump the `crates/zotero-mcp/Cargo.toml` `schemars` line

In `/Users/rjl/Code/github/zotero-connector/crates/zotero-mcp/Cargo.toml`, find the `schemars` line (it's in the `[dependencies]` block as `schemars = "0.8"` — `grep -n '^schemars' crates/zotero-mcp/Cargo.toml` finds it).

Change to: `schemars = "1"`

(Major-only pin, matching workspace convention for post-1.0 crates.)

### Step 3: Joint lockfile update

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo update -p rmcp -p schemars 2>&1 | tail -20`

Expected:
- `Updating rmcp v0.1.5 -> v1.7.x`
- `Updating rmcp-macros v0.1.5 -> v1.7.x` (transitive)
- `Updating schemars v0.8.22 -> v1.x.x`
- `Updating schemars_derive v0.8.22 -> v1.x.x` (transitive)
- Possible: `paste` removed → `pastey` added (rmcp's macros switched).

### Step 4: First build attempt (expected to fail)

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo build -p zotero-mcp 2>&1 | tail -40`

Expected: FAIL with macro/attribute errors. The first error will almost certainly be in `server.rs` and will name either `tool_box` or `aggr` as an unknown attribute. That's expected — Step 5 fixes it. Do not panic or escalate yet.

If, surprisingly, the build succeeds: skip to Step 10.

### Step 5: Convert struct-level `#[tool(tool_box)]` → `#[tool_router]`

In `/Users/rjl/Code/github/zotero-connector/crates/zotero-mcp/src/server.rs`, find line 29:

```rust
#[tool(tool_box)]
impl ZoteroServer {
```

Change to:

```rust
#[tool_router]
impl ZoteroServer {
```

Also check line 280 (`#[tool(tool_box)]` on the `impl ServerHandler` block — there are two `tool_box` sites per the surface inventory). For the second site, the right rmcp 1.x macro is `#[tool_handler]`:

```rust
#[tool_handler]
impl ServerHandler for ZoteroServer {
```

You may also need to add the new macro names to the existing import:

```rust
use rmcp::{Error as McpError, ServerHandler, tool, tool_router, tool_handler, RoleServer, ...};
```

If `tool_router` or `tool_handler` aren't found at `rmcp::`, they're at `rmcp::handler::server::router::tool::tool_router` (or similar) — `cargo doc -p rmcp --open` shows the exact re-export path. The error message will name the path.

### Step 6: Convert parameter-level `#[tool(aggr)]` → `Parameters<T>` destructure (27 sites)

Add an import at the top of `server.rs`:

```rust
use rmcp::handler::server::tool::Parameters;
```

If the path is different in 1.x (the compile error will name the actual path), use what the error says.

For each of the 27 sites where the current shape is:

```rust
pub async fn search_items(
    &self,
    #[tool(aggr)] args: SearchArgs,
) -> Result<CallToolResult, McpError> {
    search::search_items(&self.state, args).await
}
```

Convert to:

```rust
pub async fn search_items(
    &self,
    Parameters(args): Parameters<SearchArgs>,
) -> Result<CallToolResult, McpError> {
    search::search_items(&self.state, args).await
}
```

The 27 sites are at lines 48, 58, 66, 74, 84, 92, 100, 108, 116, 124, 132, 140, 162, 167, 172, 177, 182, 187, 196, 203, 211, 219, 225, 232, 239, 246, 251, 256, 263, 268, 275 — confirm them by grepping: `grep -n '#\[tool(aggr)\]' crates/zotero-mcp/src/server.rs`. Each conversion is the same shape: replace the attribute + identifier with the destructuring pattern; the rest of the parameter stays the same.

The `args: T` variable name is referenced in the method body — keep it as `args` after the destructure so the bodies don't need updating.

### Step 7: Re-run build, iterate

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo build -p zotero-mcp 2>&1 | tail -40`

Expected outcomes (in likely order):

| Error | Fix |
|---|---|
| `the trait \`schemars::JsonSchema\` is not implemented for ...` on an `Args` struct | The struct may now need a `Default` derive — the rmcp 1.5 example shows `#[derive(Deserialize, schemars::JsonSchema, Default)]`. Add `Default` to the derive list of any affected struct. |
| `cannot find type \`Parameters\` in scope` | Step 6 import path was wrong — read the compile error's `help: consider importing one of these items` suggestion and use what it says. |
| `tool_router` or `tool_handler` not found | Step 5 import was incomplete. Read the error's suggested path. |
| `the method get_info exists for ZoteroServer but its signature is different` | `#[tool_handler]` is auto-generating `get_info` and conflicting with the manual one. Remove `#[tool_handler]` and keep the manual block (non-blocking per spec). |
| `cannot find function/macro schema_for! ...` in tests | schemars `schema_for!` moved — Step 9 covers this. |
| `RootSchema is not in scope` in tests | schemars 1.x removed `RootSchema`; Step 9 ports the test. |
| `the trait Send + Sync + 'static is not implemented for ...` on transport | rmcp 1.x tightened transport bounds. Likely escalation if the fix is non-trivial — see escalation block below. |

For each error: apply the documented fix, re-run `cargo build -p zotero-mcp`. Loop until either (a) the only remaining errors are in `tests/schema_shape.rs` (proceed to Step 9), or (b) escalation triggers fire.

### Step 8: Verify the verify-only files unchanged

If Step 7 completes with the build clean for `server.rs` and tool modules, run:

```bash
cd /Users/rjl/Code/github/zotero-connector && \
git diff --stat crates/zotero-mcp/src/main.rs crates/zotero-mcp/src/http_transport.rs crates/zotero-mcp/src/tools/ crates/zotero-mcp/src/oauth.rs
```

Expected: empty, or at most a one-line `use` import update per file. Anything larger that wasn't covered by the Step 7 in-bounds list triggers escalation (see below).

### Step 9: Port `tests/schema_shape.rs` to schemars 1.x

Open `/Users/rjl/Code/github/zotero-connector/crates/zotero-mcp/tests/schema_shape.rs`. Read all 3 test bodies and translate per this pattern.

**Before (schemars 0.8):**
```rust
let schema = schema_for!(CreateItemArgs);
assert_eq!(schema.schema.instance_type, Some(InstanceType::Object.into()));
// or: schema.schema.object.as_ref()...
```

**After (schemars 1.0):**
```rust
let schema = schema_for!(CreateItemArgs);
// Schema is now a serde_json::Value wrapper.
assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
// For nested navigation:
let properties = schema.get("properties").and_then(|v| v.as_object()).expect("properties");
let item_type = properties.get("item_type").expect("item_type field present");
```

Remove the `use schemars::schema::{InstanceType, RootSchema, ...}` line (those types no longer exist in 1.x). Keep `use schemars::schema_for;` — that macro is still re-exported at the crate root.

If `schema_for` is no longer at the crate root in 1.x, the compile error will name the new path — use what it says.

The 3 test bodies should end up shorter (Value navigation is more concise than typed-struct chains). If a particular assertion can't be expressed cleanly under the new API, it's acceptable to relax it from strict equality to a `.is_some()` / structural shape check — this is documented in the spec's Risk 4.

### Step 10: Full build clean

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo build -p zotero-mcp 2>&1 | tail -10`

Expected: `Finished dev [unoptimized + debuginfo] target(s) in X.XXs`. No errors, no warnings about deprecated symbols beyond the standard cargo build output.

If errors remain that aren't covered by Steps 5–9's fix patterns, this is escalation territory — see the escalation block below.

### Step 11: Run the full test suite

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo test -p zotero-mcp 2>&1 | tail -30`

Expected: all binaries pass. Lib tests: a number that is `105` or close to it (the 3 schema_shape tests may have been rewritten with different individual test names or split count). Write the new number down; it goes in the commit body.

If any test fails for reasons other than schema_shape rewrites — escalation.

### Step 12: Review the lockfile diff

Run: `cd /Users/rjl/Code/github/zotero-connector && git diff Cargo.lock | grep -E "^[-+]name|^[-+]version" | head -40`

Expected: `rmcp` 0.1.5 → 1.7.x, `rmcp-macros` matching, `schemars` 0.8.22 → 1.x.x, `schemars_derive` matching. Possible: `paste` → `pastey`, `tokio-stream` patch bump, `serde_spanned`/`toml_datetime`/`winnow` no change (those came in with Slice B's toml bump). Anything outside the rmcp/schemars/macro ecosystem is a surprise — flag it in the commit body.

### Step 13: Confirm only intended files are staged

Run:

```bash
cd /Users/rjl/Code/github/zotero-connector && \
git add Cargo.toml crates/zotero-mcp/Cargo.toml Cargo.lock \
  crates/zotero-mcp/src/server.rs \
  crates/zotero-mcp/src/main.rs \
  crates/zotero-mcp/src/http_transport.rs \
  crates/zotero-mcp/src/tools/search.rs \
  crates/zotero-mcp/src/tools/attachments.rs \
  crates/zotero-mcp/src/tools/citations.rs \
  crates/zotero-mcp/src/tools/enrichment.rs \
  crates/zotero-mcp/src/tools/writes.rs \
  crates/zotero-mcp/tests/schema_shape.rs 2>/dev/null; \
git status --short
```

Files that weren't actually modified will be silently skipped by `git add` — `git status` confirms exactly what's staged. Expected at minimum: the two `Cargo.toml`, `Cargo.lock`, `server.rs`, `tests/schema_shape.rs`. The verify-only files (`main.rs`, `http_transport.rs`, `tools/*.rs`) may or may not appear, depending on whether import paths shifted.

If `oauth.rs` shows up — that's an escalation trigger per the spec (out-of-bounds; oauth.rs is reserved for Slice D).

### Step 14: Commit

Build the commit body. Replace the `[bracketed]` placeholders with reality from Steps 5–12. Do not leave brackets in the actual commit message.

```bash
cd /Users/rjl/Code/github/zotero-connector && \
git commit -m "$(cat <<'EOF'
chore(deps): bump rmcp 0.1 → 1.7 (jointly with schemars 0.8 → 1)

Joint atomic bump — the two crates are coupled (rmcp 0.1 requires
schemars ^0.8 and its #[tool] macros consume schemars 0.8::JsonSchema
on the Args structs; rmcp 1.7 requires schemars ^1.0). Splitting was
not viable.

server.rs macro port: [N] sites converted. Struct-level #[tool(tool_box)]
→ #[tool_router] / #[tool_handler]. 27 parameter-level #[tool(aggr)]
args: T → Parameters(args): Parameters<T> destructure. Manual
ServerHandler impl [retained / replaced by #[tool_handler]]; custom
list_resources and read_resource [retained / refit].

schemars: 27 bare #[derive(JsonSchema)] sites compiled unchanged. The
3 tests in tests/schema_shape.rs ported to the Schema = serde_json::Value
wrapper API (schema.get(...).as_str() navigation instead of typed
struct field access).

Verify-only files [unchanged | one-line import path update at <file:line>]:
main.rs, http_transport.rs, tools/{search,attachments,citations,
enrichment,writes}.rs.

Test result: [N] passed; 0 failed (was 105 at Slice B baseline; [delta
explanation if N != 105]).

Lockfile churn: rmcp 0.1.5 → 1.7.x, rmcp-macros matching; schemars
0.8.22 → 1.x.x, schemars_derive matching. [paste → pastey if observed.
Any surprise transitives.]

oauth.rs and http_transport.rs deliberately untouched — those are
reserved for Slice D (auth feature) and Slice E (transport-streamable-
http-server feature) per the spec.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

After the commit, run `git show --stat HEAD` to confirm only the expected files are in it.

### Escalation block (use only if Steps 5–11 cannot reach a clean build/test pass via the documented fix patterns)

Per the spec's Decision 4, the slice cannot be partially landed. If any of these surface and resist mechanical fixing:

1. `ServerHandler` trait signature change requiring redesign of `list_resources`/`read_resource` (more than swapping a method signature).
2. `Tool` macro output incompatible with `Result<CallToolResult, McpError>` async return shape.
3. schemars 1.x rejects `Vec<serde_json::Map<String, Value>>` in `ProposeArgs` or `EnrichArgs` (requires a custom `JsonSchema` impl or shape change).
4. rmcp 1.x's transport bounds require touching `http_transport.rs` beyond a one-line import update.
5. Anything that requires non-trivial edits in `oauth.rs` or `http_transport.rs` (reserved for Slice D/E).

Revert:

```bash
cd /Users/rjl/Code/github/zotero-connector && \
git checkout -- Cargo.toml crates/zotero-mcp/Cargo.toml Cargo.lock \
  crates/zotero-mcp/src/server.rs \
  crates/zotero-mcp/src/main.rs \
  crates/zotero-mcp/src/http_transport.rs \
  crates/zotero-mcp/src/tools/ \
  crates/zotero-mcp/tests/schema_shape.rs
```

Then amend `docs/superpowers/specs/2026-05-13-dependency-upgrades-slice-c-design.md` — add a new section at the bottom:

```markdown
---

## Deferred (rmcp+schemars joint bump)

[Date]: This slice was attempted and reverted. Blocker:

[One paragraph describing the specific blocker. Name the file, the API,
and the reason mechanical fixing wasn't possible. Reference the
escalation-block item number from the plan.]
```

Commit that amendment as: `docs(spec): defer rmcp+schemars joint bump — <one-line reason>`.

Then report status DONE_WITH_CONCERNS describing the deferral. Do not attempt the bump again in this session.

---

## Final: install + restart + version check

Run only after Task 1 lands.

- [ ] **Step 1: Rebuild and install**

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo install --path crates/zotero-mcp 2>&1 | tail -3`

Expected: `Replacing /Users/rjl/.cargo/bin/zotero-mcp` with the new version.

- [ ] **Step 2: Restart the launchd service**

Run: `launchctl kickstart -k gui/$(id -u)/com.zotero-mcp.http && sleep 2 && launchctl list | grep com.zotero-mcp.http`

Expected: a numeric PID in the first column. The status column may show `-15` — that's the prior-instance SIGTERM from `kickstart -k`, not a fault. Run the command a second time after a 2-second sleep if the value looks ambiguous.

- [ ] **Step 3: Confirm the new SHA is live via the ping probe**

Tell the user: "Slice C landed. Call `ping` from your MCP client; it should return `pong (v0.2.0, <new-sha>)` where `<new-sha>` matches `git rev-parse --short HEAD`."

Don't try to invoke the MCP client yourself — that's the user's job.

---

## Verification checklist

After Task 1 completes:

- [ ] Exactly one new commit lands on `main` with the documented message format.
- [ ] The commit passes `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp`.
- [ ] The new lib-test count is explicitly noted in the commit body (whether it shifted from 105 or stayed).
- [ ] `Cargo.lock` is included in the commit.
- [ ] If the slice escalated, the spec's "Deferred (rmcp+schemars joint bump)" section is amended, the slice's edits are fully reverted, and main is clean at the pre-flight SHA.
- [ ] The final binary is installed at `/Users/rjl/.cargo/bin/zotero-mcp` and the launchd service is running with that binary.
- [ ] The git log shows: `chore(deps): bump rmcp 0.1 → 1.7 (jointly with schemars 0.8 → 1)` immediately after the spec commit.
