# Dependency Upgrades — Slice B Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring two pinned dependencies to their latest release with one commit per crate: `reqwest 0.12 → 0.13`, then `toml 0.8 → 1`.

**Architecture:** Pure dependency bumps with one anticipated metadata edit. Reqwest 0.13 renamed the `rustls-tls` feature to `rustls` and moved `query` off-by-default — both are required Cargo.toml edits (the codebase uses `.query()` once at `writer/client.rs:84`). Toml's 0.x → 1.x jump is expected to be metadata-only based on plan-time changelog research. Order: reqwest first (broader surface, more likely friction), toml second (narrow surface, mostly insulated).

**Tech Stack:** Rust, Cargo. The crate ecosystem's standard upgrade flow.

**Spec:** `docs/superpowers/specs/2026-05-13-dependency-upgrades-slice-b-design.md` (commit `a30e82e`)

---

## File Structure

| File | Purpose | Action |
|---|---|---|
| `Cargo.toml` (workspace) | Holds `reqwest` and `toml` versions | Modify (Tasks 1, 2) |
| `Cargo.lock` | Pinned dependency graph | Modify (every task) |
| `crates/zotero-mcp/src/core/writer/client.rs` | Has the one `.query()` call | Read-only; flag if Step 4 of Task 1 reveals an API rename |
| `crates/zotero-mcp/src/core/error.rs` | Has `From<toml::de::Error>` impl | (Possibly) modify if `toml::de::Error` was renamed — defensive |

Per plan-time research (reqwest CHANGELOG, toml CHANGELOG), only the workspace `Cargo.toml` and `Cargo.lock` should change. Source files are listed defensively; if a build error names one, Step 4 of the corresponding task covers the fix.

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

Expected: every line shows `ok`, no `FAILED`. Lib tests should still be `105 passed; 0 failed` (the Slice A baseline). Note this so regressions are visible.

- [ ] **Step 3: Record the current pre-flight SHA**

Run: `cd /Users/rjl/Code/github/zotero-connector && git rev-parse HEAD`

Write down the SHA. This is the rollback point if a single bump goes wrong. Expected at slice start: `a30e82e` (the Slice B spec commit) — or, if Slice B is being re-run after a checkpoint, whatever the current HEAD is.

---

## Task 1: `reqwest 0.12 → 0.13`

**Files:**
- Modify: `Cargo.toml` (workspace, NOT crate-level — `reqwest` is in workspace deps)
- Modify: `Cargo.lock`
- Possibly modify: source files only if Step 4 reveals an API rename. Per plan-time research, no source changes are expected.

**Call-site inventory** (21 uses across 13 files):
- `crates/zotero-mcp/src/state.rs:93` — `reqwest::Client::new()`
- `crates/zotero-mcp/src/core/bbt.rs:9,14` — `reqwest::Client`, `reqwest::Client::builder()`
- `crates/zotero-mcp/src/core/web.rs:55,135,166` — `Client::builder().user_agent(...).build()`, `reqwest::Method::POST`
- `crates/zotero-mcp/src/core/error.rs:107` — `#[from] reqwest::Error` (no pattern match)
- `crates/zotero-mcp/src/core/writer/client.rs:2,84` — `use reqwest::{Client, Method, RequestBuilder};` and `.query(&[...])`
- `crates/zotero-mcp/src/core/writer/{tags,notes,attachments,items}.rs` — `use reqwest::Method;` plus `reqwest::Response` (items.rs:34)
- `crates/zotero-mcp/src/core/enrichment/{arxiv,crossref,openlibrary,semantic_scholar}.rs` — `reqwest::Client::builder().user_agent(...).build().unwrap()` (4 files, same shape)

**Key API changes confirmed at plan-time** (source: reqwest CHANGELOG via WebFetch):

| Change | Impact on this codebase |
|---|---|
| `rustls-tls` feature renamed to `rustls` | **Required Cargo.toml edit** — we set `features = ["rustls-tls", ...]` |
| `query` feature now disabled by default | **Required Cargo.toml edit** — we call `.query(&[...])` at `writer/client.rs:84` |
| `form` feature now disabled by default | No impact — codebase has no `.form()` calls |
| rustls crypto provider defaults to aws-lc instead of ring | Lockfile-level only; expect `aws-lc-*` crates to replace `ring` |
| `native-tls` now includes ALPN | No impact — we don't use `native-tls` |
| `trust-dns` feature removed | No impact — not used |
| TLS-related method renames (e.g. `use_rustls_tls` → `tls_backend_rustls`) | Soft deprecation — old names still work without warnings |
| `reqwest::Error` variants | No impact — zero exhaustive matches confirmed at call-site survey |

- [ ] **Step 1: Bump the version and update features in workspace `Cargo.toml`**

In `/Users/rjl/Code/github/zotero-connector/Cargo.toml`, find line 20:

```toml
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "gzip", "brotli"] }
```

Change to:

```toml
reqwest = { version = "0.13", default-features = false, features = ["rustls", "json", "gzip", "brotli", "query"] }
```

Two changes in one line:
- `version = "0.12"` → `version = "0.13"`
- features: `"rustls-tls"` → `"rustls"`; add `"query"` (preserving `json`, `gzip`, `brotli` in order).

- [ ] **Step 2: Update the lockfile**

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo update -p reqwest`

Expected: `Updating reqwest v0.12.28 -> v0.13.x` plus a cloud of transitive updates — most notably the `aws-lc-*` family appearing (new default crypto provider) and possibly `ring` exiting if not held by other deps.

- [ ] **Step 3: Build**

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo build -p zotero-mcp 2>&1 | tail -30`

Expected: PASS based on plan-time research. If errors, continue to Step 4.

- [ ] **Step 4: Fix compile errors (only if Step 3 failed)**

| Likely error | Fix |
|---|---|
| `the package 'reqwest' does not have feature 'query'` | The `query` feature spelling differs in 0.13. Run `cargo info reqwest@0.13.3` and check the feature list; substitute the correct name. If `query()` is not feature-gated in 0.13 after all, remove `"query"` from the features list and rely on default availability. |
| `the package 'reqwest' does not have feature 'rustls'` | Confirm against `cargo info reqwest@0.13.3`. The changelog said `rustls-tls` → `rustls`; if the actual name is `rustls-tls-manual-roots` or similar, substitute. |
| `cannot find function 'query' on RequestBuilder` | Feature flag missing — verify Step 1's feature additions landed. |
| `Response::error_for_status` signature changed | Re-read the error; if it now returns `Result<Response, Error>` (unchanged), the existing chain compiles. If signature shifted, update at the named file:line. |
| `multipart::Form::part` or `Body` builder changed | `writer/attachments.rs` is the only file that uses multipart (Zotero file upload). Inspect the new signature and apply at the named file:line. |
| `cannot find type 'Method' in module 'reqwest'` | Unlikely (Method is a stable re-export from `http` crate); if surfaced, update the `use` clause to `use reqwest::Method;` or `use http::Method;`. |
| Anything else | If the fix needs more than 5 minutes or touches semantics, **stop and escalate** (revert per the spec's escalation rule). |

Re-run Step 3 after each fix attempt. Loop until clean.

- [ ] **Step 5: Run the full test suite**

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo test -p zotero-mcp 2>&1 | tail -20`

Expected: PASS. Lib tests still `105 passed; 0 failed`.

- [ ] **Step 6: Review the lockfile diff**

Run: `cd /Users/rjl/Code/github/zotero-connector && git diff Cargo.lock | grep -E "^[-+]name|^[-+]version" | head -60`

Expected updates and disappearances:
- `reqwest` 0.12.28 → 0.13.x
- New `aws-lc-rs`, `aws-lc-sys`, possibly `bindgen` and `paste` transitive cluster (rustls's new default crypto)
- `ring` removed (if no other crate holds it)
- Possible `hyper`, `http`, `rustls`, `tokio-rustls`, `rustls-platform-verifier` cohort updates
- Possible `hickory-resolver` appearance / `trust-dns-resolver` disappearance

Flag anything outside the HTTP-client / crypto / DNS domain as a surprise in the commit message.

- [ ] **Step 7: Commit**

Run:
```bash
cd /Users/rjl/Code/github/zotero-connector && \
git add Cargo.toml Cargo.lock \
  crates/zotero-mcp/src/state.rs \
  crates/zotero-mcp/src/core/bbt.rs \
  crates/zotero-mcp/src/core/web.rs \
  crates/zotero-mcp/src/core/error.rs \
  crates/zotero-mcp/src/core/writer/client.rs \
  crates/zotero-mcp/src/core/writer/tags.rs \
  crates/zotero-mcp/src/core/writer/notes.rs \
  crates/zotero-mcp/src/core/writer/attachments.rs \
  crates/zotero-mcp/src/core/writer/items.rs \
  crates/zotero-mcp/src/core/enrichment/arxiv.rs \
  crates/zotero-mcp/src/core/enrichment/crossref.rs \
  crates/zotero-mcp/src/core/enrichment/openlibrary.rs \
  crates/zotero-mcp/src/core/enrichment/semantic_scholar.rs 2>/dev/null; \
git status --short
```

Files that weren't modified will be silently skipped by `git add`; `git status` confirms exactly what's staged. Expected: only `Cargo.toml` and `Cargo.lock` based on plan-time research.

Then:
```bash
git commit -m "$(cat <<'EOF'
chore(deps): bump reqwest 0.12 → 0.13

Workspace Cargo.toml: feature 'rustls-tls' renamed to 'rustls' (0.13
rename); 'query' added to features list because it is no longer
default-enabled and writer/client.rs:84 uses .query(&[...]).

Call sites verified: 21 uses across 13 files (Client, Client::builder,
Method, Response, RequestBuilder; #[from] reqwest::Error). No
exhaustive matches on reqwest::Error. API stable in this codebase's
usage; no source changes [required | applied — list any here].

Lockfile churn: aws-lc-* cluster added (new default crypto provider);
ring [removed | retained]; hyper/http/rustls/tokio-rustls cohort
[bumped — list]. [Add any surprise transitives here.]

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Edit the bracketed blocks to match reality: write `required` if Step 4 was skipped, or list each `file:line` that was edited. Fill in the actual transitive churn from Step 6's lockfile review.

---

## Task 2: `toml 0.8 → 1`

**Files:**
- Modify: `Cargo.toml` (workspace, NOT crate-level — `toml` is in workspace deps)
- Modify: `Cargo.lock`
- Possibly modify: `crates/zotero-mcp/src/core/error.rs` (only if `toml::de::Error` was renamed in 1.x — defensive; the changelog did not flag this)

**Call-site inventory** (11 uses across 4 files):
- `crates/zotero-mcp/src/oauth.rs:117` — `toml::from_str(std::str::from_utf8(&bytes)?)`
- `crates/zotero-mcp/src/oauth.rs:161` — `toml::to_string_pretty(config)?`
- `crates/zotero-mcp/src/oauth.rs:746,762,781` — `toml::from_str(...)` (test code)
- `crates/zotero-mcp/src/setup.rs:329` — `toml::from_str(std::str::from_utf8(&bytes)?)?`
- `crates/zotero-mcp/src/core/config.rs:178` — `toml::from_str(&text)?`
- `crates/zotero-mcp/src/core/config.rs:261,288,309` — `toml::from_str(toml).unwrap()` (test code)
- `crates/zotero-mcp/src/core/error.rs:113` — `Toml(#[from] toml::de::Error)`

**Key API changes confirmed at plan-time** (source: toml CHANGELOG via WebFetch):

| Change | Impact on this codebase |
|---|---|
| `toml::from_str` signature | Unchanged — primary entry point, stable across 0.9/1.0/1.1 |
| `toml::to_string_pretty` signature | Unchanged — stable |
| `toml::Value` enum variant changes (e.g. Datetime moved to `toml_datetime`) | No impact — codebase does not use `toml::Value` (verified — earlier `Value::` greps were all `serde_json::Value`) |
| `Time::second` / `Time::nanosecond` wrapped in `Option` (1.0) | No impact — codebase doesn't use `Time` |
| `Deserializer::new` deprecated | No impact — codebase uses top-level `toml::from_str`, not the Deserializer directly |
| `Serializer::new` / `Serializer::pretty` accept `&mut Buffer` | No impact — codebase uses top-level `toml::to_string_pretty`, not Serializer directly |
| Order preservation default-off | No impact — codebase doesn't depend on order; config files are unordered fact-tables |
| Serde and std moved to default features | No impact — codebase pins `toml = "0.8"` without `default-features = false`, so defaults apply post-bump too |
| MSRV bumped to 1.85 in 1.1 | No impact — local rustc 1.93 |

The `toml::de::Error` type was NOT flagged as renamed in the changelog. Defensively, if Step 3 fails with a type-not-found error on `toml::de::Error`, Step 4 covers the rename fix.

- [ ] **Step 1: Bump the version in workspace `Cargo.toml`**

In `/Users/rjl/Code/github/zotero-connector/Cargo.toml`, find line 27:

```toml
toml = "0.8"
```

Change to:

```toml
toml = "1"
```

(Major-only pin, matching the workspace's existing convention for post-1.0 crates like `which`, `directories`, `pdf-extract`.)

- [ ] **Step 2: Update the lockfile**

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo update -p toml`

Expected: `Updating toml v0.8.23 -> v1.1.x` plus possible bumps to `toml_edit`, `toml_datetime` siblings.

- [ ] **Step 3: Build**

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo build -p zotero-mcp 2>&1 | tail -30`

Expected: PASS. If errors, continue to Step 4.

- [ ] **Step 4: Fix compile errors (only if Step 3 failed)**

| Likely error | Fix |
|---|---|
| `cannot find type 'Error' in module 'toml::de'` | The error type was renamed in 1.x. Update `crates/zotero-mcp/src/core/error.rs:113` from `Toml(#[from] toml::de::Error)` to whichever name 1.x uses (most likely `toml::Error` or `toml::de::Error` still — verify with `cargo doc -p toml --open` or read `~/.cargo/registry/src/index.crates.io-.../toml-1.x.x/src/lib.rs`). One file, one line — still mechanical. |
| `toml::from_str` requires explicit type annotation | Unlikely — the codebase always annotates the LHS (`let cfg: OAuthConfig = toml::from_str(...)`). If it surfaces, add the type annotation at the named file:line. |
| `toml::to_string_pretty` signature changed | Unlikely. If surfaced, check the new signature at the named file:line (`oauth.rs:161`). |
| Anything else | If the fix needs more than 5 minutes or touches semantics, **stop and escalate** (revert per the spec's escalation rule). |

Re-run Step 3 after each fix attempt. Loop until clean.

- [ ] **Step 5: Run the targeted config + oauth tests, then the full suite**

```bash
cd /Users/rjl/Code/github/zotero-connector && \
cargo test -p zotero-mcp config 2>&1 | tail -10 && \
cargo test -p zotero-mcp oauth 2>&1 | tail -10 && \
cargo test -p zotero-mcp 2>&1 | tail -10
```

Expected: all three commands PASS. The targeted runs exercise the config-parse and oauth-toml round-trip paths, which are the most direct toml callers.

- [ ] **Step 6: Review the lockfile diff**

Run: `cd /Users/rjl/Code/github/zotero-connector && git diff Cargo.lock | grep -E "^[-+]name|^[-+]version" | head -20`

Expected: just `toml` 0.8.x → 1.x and possibly `toml_edit` / `toml_datetime` patch updates. Anything else is a surprise — note it in the commit body.

- [ ] **Step 7: Commit**

```bash
cd /Users/rjl/Code/github/zotero-connector && \
git add Cargo.toml Cargo.lock \
  crates/zotero-mcp/src/core/error.rs \
  crates/zotero-mcp/src/oauth.rs \
  crates/zotero-mcp/src/setup.rs \
  crates/zotero-mcp/src/core/config.rs 2>/dev/null; \
git status --short
```

Then:
```bash
git commit -m "$(cat <<'EOF'
chore(deps): bump toml 0.8 → 1

Call sites verified: 11 uses across 4 files — toml::from_str (8x in
oauth.rs/setup.rs/core/config.rs, both prod and tests),
toml::to_string_pretty (1x at oauth.rs:161), #[from] toml::de::Error
(1x at core/error.rs:113). API stable in this codebase's usage; no
source changes [required | applied — list any here].

Crossed the 0.x → 1.x major boundary plus the 1.0 → 1.1 minor.
Lockfile churn: toml 0.8.23 → 1.1.x [list any toml_edit / toml_datetime
sibling updates].

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Edit the bracketed blocks to match reality.

---

## Final: install + restart + version check

After both tasks land (or whichever survived without escalating):

- [ ] **Step 1: Rebuild and install**

Run: `cd /Users/rjl/Code/github/zotero-connector && cargo install --path crates/zotero-mcp 2>&1 | tail -3`

Expected: `Replacing /Users/rjl/.cargo/bin/zotero-mcp` with the new version.

- [ ] **Step 2: Restart the launchd service**

Run: `launchctl kickstart -k gui/$(id -u)/com.zotero-mcp.http && sleep 2 && launchctl list | grep com.zotero-mcp.http`

Expected: a numeric PID in the first column (service running).

- [ ] **Step 3: Confirm the new SHA is live via the ping probe**

Tell the user: "Both bumps landed. Call `ping` from your MCP client; it should return `pong (v0.2.0, <new-sha>)` where `<new-sha>` matches `git rev-parse --short HEAD`."

Don't try to invoke the MCP client yourself — that's the user's job.

---

## Verification checklist

After all tasks complete:

- [ ] Two commits (or fewer, if either crate escalated) land in the documented order on `main`.
- [ ] Each commit individually passes `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp`. Lib tests still `105 passed; 0 failed`.
- [ ] `Cargo.lock` is included in every dep-bump commit.
- [ ] If any crate escalated, its row appears in the spec's "Deferred to later slice" section with the reason (per spec's escalation rule).
- [ ] The final binary is installed at `/Users/rjl/.cargo/bin/zotero-mcp` and the launchd service is running with that binary.
- [ ] The git log shows: `chore(deps): bump reqwest 0.12 → 0.13` followed by `chore(deps): bump toml 0.8 → 1` (or just the surviving one).
