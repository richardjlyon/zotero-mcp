# Dependency Upgrades — Slice A Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring six pinned dependencies to their latest release with one commit per crate: `rand 0.9→0.10`, `sha2 0.10→0.11`, `md-5 0.10→0.11`, `directories 5→6`, `which 7→8`, `pdf-extract 0.7→0.10`.

**Architecture:** Pure dependency bumps. Each task edits `Cargo.toml`, runs `cargo update -p <crate>`, fixes any call-site breakage, runs the test suite, and commits. No new modules, no design changes. Order is smallest-expected-delta first so the highest-risk crate (`pdf-extract`) lands on a clean diff.

**Tech Stack:** Rust, Cargo. The crate ecosystem's standard upgrade flow.

**Spec:** `docs/superpowers/specs/2026-05-13-dependency-upgrades-slice-a-design.md` (commit `6f75756`)

---

## File Structure

| File | Purpose | Action |
|---|---|---|
| `Cargo.toml` (workspace) | Holds `directories` version | Modify (Task 4) |
| `crates/zotero-mcp/Cargo.toml` | Holds the other five crate versions | Modify (Tasks 1, 2, 3, 5, 6) |
| `Cargo.lock` | Pinned dependency graph | Modify (every task) |

No source files are touched unless a bump produces a compile error, in which case the affected file is named in the task's call-site inventory below.

---

## Pre-flight: confirm clean state

**Files:**
- Read-only: working tree

- [ ] **Step 1: Confirm clean tree on `main`**

Run: `cd /Users/rjl/Code/mcp-zotero && git status`

Expected: `nothing to commit, working tree clean` and branch `main`.

If dirty, stop and resolve before starting the slice.

- [ ] **Step 2: Capture baseline test results**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo test -p zotero-mcp 2>&1 | grep "^test result:" | sort | uniq -c`

Expected: every line shows `ok`, no `FAILED`. Note the total count (e.g. 105 lib tests + various integration tests) so we can spot regressions.

- [ ] **Step 3: Record the current pre-flight SHA**

Run: `cd /Users/rjl/Code/mcp-zotero && git rev-parse HEAD`

Write down the SHA (this is the rollback point if a single bump goes wrong).

---

## Task 1: `rand 0.9 → 0.10`

**Files:**
- Modify: `crates/zotero-mcp/Cargo.toml`
- Modify: `Cargo.lock`
- Possibly modify: `crates/zotero-mcp/src/oauth.rs`, `crates/zotero-mcp/src/oauth/token_store.rs`, `crates/zotero-mcp/src/http_transport.rs` (only if `rand::random::<T>()` API changed)

**Call-site inventory** (8 uses, all `rand::random::<T>()`):
- `crates/zotero-mcp/src/oauth.rs:133, 176, 613, 797`
- `crates/zotero-mcp/src/oauth/token_store.rs:335, 342`
- `crates/zotero-mcp/src/http_transport.rs:76, 241`

The free function `rand::random::<T>()` is preserved across the 0.9→0.10 bump. Most likely outcome: zero source-code changes.

- [ ] **Step 1: Bump the version in `crates/zotero-mcp/Cargo.toml`**

Find the line:
```toml
rand = "0.9"
```
Change to:
```toml
rand = "0.10"
```

- [ ] **Step 2: Update the lockfile**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo update -p rand`

Expected: `Updating rand v0.9.x -> v0.10.y` (and possibly a small cloud of transitive updates to `rand_*` sibling crates).

- [ ] **Step 3: Build**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo build -p zotero-mcp 2>&1 | tail -30`

Expected outcomes:
- **PASS:** continue to Step 5.
- **Compile errors:** continue to Step 4 to apply fixes.

- [ ] **Step 4: Fix compile errors (only if Step 3 failed)**

The likely error shapes and fixes:

| Error | Likely fix |
|---|---|
| `cannot find function 'thread_rng' in crate 'rand'` | `rand::thread_rng()` → `rand::rng()`. (Not used in this codebase per current grep, but flagged in case a transitive update reorganises imports.) |
| `gen() is deprecated and removed` | `rng.gen::<T>()` → `rng.random::<T>()`. (Also not used in this codebase, but documented.) |
| Anything else | Re-read the error, locate the file:line, apply the named API change. If the change requires more than 5 minutes or touches semantics, **stop and escalate** (revert per the spec's escalation rule). |

Re-run Step 3 after each fix attempt. Loop until clean.

- [ ] **Step 5: Run the full test suite**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo test -p zotero-mcp 2>&1 | tail -20`

Expected: PASS — same test counts as baseline (Pre-flight Step 2).

- [ ] **Step 6: Review the lockfile diff**

Run: `cd /Users/rjl/Code/mcp-zotero && git diff Cargo.lock | grep -E "^[-+]name|^[-+]version" | head -20`

Confirm the only updated crates are `rand` and its transitive siblings (`rand_chacha`, `rand_core`, etc.). Flag any surprise updates in the commit message.

- [ ] **Step 7: Commit**

Run:
```bash
cd /Users/rjl/Code/mcp-zotero && \
git add Cargo.toml Cargo.lock crates/zotero-mcp/Cargo.toml \
  crates/zotero-mcp/src/oauth.rs \
  crates/zotero-mcp/src/oauth/token_store.rs \
  crates/zotero-mcp/src/http_transport.rs 2>/dev/null; \
git status --short
```

If any source file from the list above is unmodified, `git add` silently skips it; that's fine. The `git status` confirms exactly what's staged.

Then:
```bash
git commit -m "$(cat <<'EOF'
chore(deps): bump rand 0.9 → 0.10

Call sites verified: 8 uses of rand::random::<T>() in oauth.rs,
oauth/token_store.rs, http_transport.rs. API stable; no source changes
[required | applied — list any here].

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Edit the bracketed block to match reality: `required` if Step 4 was skipped, or list each `file:line` that was edited.

---

## Task 2: `sha2 0.10 → 0.11`

**Files:**
- Modify: `crates/zotero-mcp/Cargo.toml`
- Modify: `Cargo.lock`
- Possibly modify: `crates/zotero-mcp/src/oauth.rs`, `crates/zotero-mcp/src/oauth/token_store.rs` (only if `Digest` trait or `Sha256` API changed)

**Call-site inventory** (4 uses):
- `crates/zotero-mcp/src/oauth.rs:43` — `use sha2::{Digest, Sha256};`
- `crates/zotero-mcp/src/oauth.rs:567` — `Sha256::digest(verifier.as_bytes())`
- `crates/zotero-mcp/src/oauth/token_store.rs:17` — same `use` clause
- `crates/zotero-mcp/src/oauth/token_store.rs:94` — `Sha256::digest(input.as_bytes())`

The RustCrypto `Digest` trait is stable across major bumps. Most likely outcome: zero source-code changes.

- [ ] **Step 1: Bump the version in `crates/zotero-mcp/Cargo.toml`**

Find:
```toml
sha2 = "0.10"
```
Change to:
```toml
sha2 = "0.11"
```

- [ ] **Step 2: Update the lockfile**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo update -p sha2`

Expected: `Updating sha2 v0.10.x -> v0.11.y` (and possibly `digest`, `block-buffer`, transitive RustCrypto crates).

- [ ] **Step 3: Build**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo build -p zotero-mcp 2>&1 | tail -30`

If clean, go to Step 5. If errors, Step 4.

- [ ] **Step 4: Fix compile errors (only if Step 3 failed)**

| Error | Likely fix |
|---|---|
| `Digest is not in scope` | Trait moved; update `use sha2::Digest` to whichever module re-exports it (e.g. `sha2::digest::Digest`). |
| `Sha256::digest` signature changed | Replace with `let mut h = Sha256::new(); h.update(bytes); h.finalize()`. |

If the fix needs more than 5 minutes or touches semantics, escalate (revert).

- [ ] **Step 5: Run the full test suite**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo test -p zotero-mcp 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 6: Review the lockfile diff**

Run: `cd /Users/rjl/Code/mcp-zotero && git diff Cargo.lock | grep -E "^[-+]name|^[-+]version" | head -15`

Confirm only `sha2` and RustCrypto-family transitives updated.

- [ ] **Step 7: Commit**

```bash
cd /Users/rjl/Code/mcp-zotero && \
git add Cargo.toml Cargo.lock crates/zotero-mcp/Cargo.toml \
  crates/zotero-mcp/src/oauth.rs \
  crates/zotero-mcp/src/oauth/token_store.rs 2>/dev/null; \
git commit -m "$(cat <<'EOF'
chore(deps): bump sha2 0.10 → 0.11

Call sites verified: 4 uses of Sha256::digest in oauth.rs and
oauth/token_store.rs (PKCE S256). API stable; no source changes
[required | applied — list].

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `md-5 0.10 → 0.11`

**Files:**
- Modify: `crates/zotero-mcp/Cargo.toml`
- Modify: `Cargo.lock`
- Possibly modify: `crates/zotero-mcp/src/core/writer/attachments.rs`

**Call-site inventory** (2 uses, both inside the same fn):
- `crates/zotero-mcp/src/core/writer/attachments.rs:15` — `use md5::{Digest, Md5};`
- `crates/zotero-mcp/src/core/writer/attachments.rs:16` — `let mut h = Md5::new();`

Identical migration pattern to sha2 — same `Digest` trait. Most likely outcome: zero source-code changes.

- [ ] **Step 1: Bump the version in `crates/zotero-mcp/Cargo.toml`**

Find:
```toml
md-5 = "0.10"
```
Change to:
```toml
md-5 = "0.11"
```

- [ ] **Step 2: Update the lockfile**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo update -p md-5`

- [ ] **Step 3: Build**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo build -p zotero-mcp 2>&1 | tail -30`

If clean, go to Step 5. If errors, Step 4.

- [ ] **Step 4: Fix compile errors (only if Step 3 failed)**

| Error | Likely fix |
|---|---|
| `Digest is not in scope` | Same as sha2: trait may have moved; update the `use` line. |
| `Md5::new`, `update`, `finalize` signature changed | Apply the new pattern. |

5-minute rule applies. Escalate if non-trivial.

- [ ] **Step 5: Run the writer-attach test specifically, then the full suite**

```bash
cd /Users/rjl/Code/mcp-zotero && \
cargo test -p zotero-mcp --test writer_attach_file 2>&1 | tail -10 && \
cargo test -p zotero-mcp 2>&1 | tail -10
```

Expected: both PASS. The targeted test runs the upload path that uses the MD5 digest.

- [ ] **Step 6: Review the lockfile diff**

Run: `cd /Users/rjl/Code/mcp-zotero && git diff Cargo.lock | grep -E "^[-+]name|^[-+]version" | head -15`

- [ ] **Step 7: Commit**

```bash
cd /Users/rjl/Code/mcp-zotero && \
git add Cargo.toml Cargo.lock crates/zotero-mcp/Cargo.toml \
  crates/zotero-mcp/src/core/writer/attachments.rs 2>/dev/null; \
git commit -m "$(cat <<'EOF'
chore(deps): bump md-5 0.10 → 0.11

Call sites verified: 1 use of Md5 in core/writer/attachments.rs
(Zotero upload Content-MD5). API stable; no source changes
[required | applied — list].

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `directories 5 → 6`

**Files:**
- Modify: `Cargo.toml` (workspace, NOT the crate-level Cargo.toml — `directories` is in workspace deps)
- Modify: `Cargo.lock`
- Possibly modify: `crates/zotero-mcp/src/oauth.rs`, `crates/zotero-mcp/src/setup.rs`

**Call-site inventory** (3 uses of `ProjectDirs` / `UserDirs`):
- `crates/zotero-mcp/src/oauth.rs:95` — `ProjectDirs::from("dev", "zotero-mcp", "zotero-mcp")`
- `crates/zotero-mcp/src/oauth.rs:221` — same
- `crates/zotero-mcp/src/setup.rs:221` — `UserDirs::new()`

> **Important:** this is the one bump in the slice with a potential **on-disk data** impact. If `directories 6.0` returns a different `config_dir()` for the same project tuple, an existing user's config file (`oauth.toml`) becomes invisible to the new build.

- [ ] **Step 1: Record the current `config_dir()` path under directories 5**

The existing binary already writes `oauth.toml` and `tokens.json` under the `directories 5.x` path. List that directory now so we have a "before" snapshot to compare against post-bump.

```bash
ls -la ~/Library/Application\ Support/dev.zotero-mcp.zotero-mcp/ 2>/dev/null || \
ls -la ~/.config/zotero-mcp/ 2>/dev/null
```

Record the full path printed (e.g. `/Users/rjl/Library/Application Support/dev.zotero-mcp.zotero-mcp/`). This is the "before" path.

If neither path exists, the user hasn't run OAuth yet and there's nothing to break. In that case the path-comparison check in Step 6 can be skipped — but still run the probe binary there so the spec's "Deferred" decision has data behind it.

- [ ] **Step 2: Bump the version in workspace `Cargo.toml`**

In `/Users/rjl/Code/mcp-zotero/Cargo.toml`, find:
```toml
directories = "5"
```
Change to:
```toml
directories = "6"
```

- [ ] **Step 3: Update the lockfile**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo update -p directories`

Expected: `Updating directories v5.0.x -> v6.0.y` (and possibly `directories-next`, `dirs-sys` transitive updates).

- [ ] **Step 4: Build**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo build -p zotero-mcp 2>&1 | tail -30`

If errors, Step 5. If clean, Step 6.

- [ ] **Step 5: Fix compile errors (only if Step 4 failed)**

| Error | Likely fix |
|---|---|
| `ProjectDirs::from` signature changed | Inspect the new signature; usually still takes 3 strings. If a 4th argument or different ordering: update the 2 call sites in `oauth.rs:95` and `oauth.rs:221` and the 1 in `setup.rs:221`. |
| `UserDirs::new` removed | Look for the replacement (e.g. `BaseDirs::new` or `UserDirs::new` returning a `Result`). |

Escalate if non-trivial.

- [ ] **Step 6: Verify the resolved config path didn't change**

Write a temporary integration test (or use the running binary):

Create `/tmp/probe_dirs/Cargo.toml`:
```toml
[package]
name = "probe-dirs"
version = "0.0.1"
edition = "2021"

[dependencies]
directories = "6"
```

Create `/tmp/probe_dirs/src/main.rs`:
```rust
fn main() {
    let p = directories::ProjectDirs::from("dev", "zotero-mcp", "zotero-mcp").unwrap();
    println!("config_dir = {:?}", p.config_dir());
    println!("data_dir = {:?}", p.data_dir());
    println!("cache_dir = {:?}", p.cache_dir());
}
```

Then:
```bash
cd /tmp/probe_dirs && cargo run --quiet 2>&1 | head -5
```

Compare the output to what you recorded in Step 1.

- **If the paths are unchanged:** continue to Step 7.
- **If the paths differ:** STOP. Revert this task's changes (`git checkout -- Cargo.toml Cargo.lock`), add `directories 5 → 6` to the spec's "Deferred to later slice" section (with the resolved path delta documented), and skip to Task 5. The reason: a silent path move would orphan the user's existing `oauth.toml`. A migration shim is needed first.

Clean up `/tmp/probe_dirs`:
```bash
rm -rf /tmp/probe_dirs
```

- [ ] **Step 7: Run the full test suite**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo test -p zotero-mcp 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 8: Review the lockfile diff**

Run: `cd /Users/rjl/Code/mcp-zotero && git diff Cargo.lock | grep -E "^[-+]name|^[-+]version" | head -20`

- [ ] **Step 9: Commit**

```bash
cd /Users/rjl/Code/mcp-zotero && \
git add Cargo.toml Cargo.lock \
  crates/zotero-mcp/src/oauth.rs \
  crates/zotero-mcp/src/setup.rs 2>/dev/null; \
git commit -m "$(cat <<'EOF'
chore(deps): bump directories 5 → 6

Call sites verified: 3 uses of ProjectDirs/UserDirs in oauth.rs and
setup.rs. Probed resolved config_dir under 5 and 6 — paths match,
so no on-disk migration needed [or: list the paths if non-obvious].

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `which 7 → 8`

**Files:**
- Modify: `crates/zotero-mcp/Cargo.toml`
- Modify: `Cargo.lock`
- Possibly modify: `crates/zotero-mcp/src/core/pdf.rs`

**Call-site inventory** (1 use):
- `crates/zotero-mcp/src/core/pdf.rs:157` — `which::which("pdftotext").ok()`

`which::which(name)` is stable across major bumps. The major version usually bumps for MSRV reasons.

- [ ] **Step 1: Check the new MSRV**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo info which@8.0.2 2>&1 | grep -i "rust-version"`

If the output shows an MSRV ≥ 1.85, that's the new floor. Confirm the project's toolchain meets it:

```bash
rustc --version
```

If `rustc` is below the new MSRV, escalate (revert without committing) — the toolchain bump is a separate decision.

- [ ] **Step 2: Bump the version in `crates/zotero-mcp/Cargo.toml`**

Find:
```toml
which = "7"
```
Change to:
```toml
which = "8"
```

- [ ] **Step 3: Update the lockfile**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo update -p which`

- [ ] **Step 4: Build**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo build -p zotero-mcp 2>&1 | tail -30`

If errors, Step 5. If clean, Step 6.

- [ ] **Step 5: Fix compile errors (only if Step 4 failed)**

| Error | Likely fix |
|---|---|
| `which::which(name)` signature changed | Adjust the call. If it now returns `Result<PathBuf, Error>` (it already does in 7.x), the existing `.ok()` chain still works. |
| MSRV-related compile failure | Same as Step 1 — escalate. |

- [ ] **Step 6: Run the full test suite**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo test -p zotero-mcp 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 7: Review the lockfile diff**

Run: `cd /Users/rjl/Code/mcp-zotero && git diff Cargo.lock | grep -E "^[-+]name|^[-+]version" | head -10`

- [ ] **Step 8: Commit**

```bash
cd /Users/rjl/Code/mcp-zotero && \
git add Cargo.toml Cargo.lock crates/zotero-mcp/Cargo.toml \
  crates/zotero-mcp/src/core/pdf.rs 2>/dev/null; \
git commit -m "$(cat <<'EOF'
chore(deps): bump which 7 → 8

Call sites verified: 1 use of which::which in core/pdf.rs
(pdftotext resolution). API stable; no source changes
[required | applied — list].

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: `pdf-extract 0.7 → 0.10`

**Files:**
- Modify: `crates/zotero-mcp/Cargo.toml` (or workspace `Cargo.toml` if pdf-extract is declared there)
- Modify: `Cargo.lock`
- Possibly modify: `crates/zotero-mcp/src/core/pdf.rs`

**Call-site inventory** (1 use):
- `crates/zotero-mcp/src/core/pdf.rs:69` — `pdf_extract::extract_text(&path).map_err(|e| e.to_string())`

Three minor versions of churn. Highest-risk crate in the slice.

> **Note:** `pdf-extract` is in workspace `Cargo.toml` (line 18). Edit there, not in the crate `Cargo.toml`.

- [ ] **Step 1: Bump the version in workspace `Cargo.toml`**

In `/Users/rjl/Code/mcp-zotero/Cargo.toml`, find:
```toml
pdf-extract = "0.7"
```
Change to:
```toml
pdf-extract = "0.10"
```

- [ ] **Step 2: Update the lockfile**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo update -p pdf-extract`

Expected: `Updating pdf-extract v0.7.x -> v0.10.y` plus transitive updates (notably `lopdf`).

- [ ] **Step 3: Build**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo build -p zotero-mcp 2>&1 | tail -30`

If errors, Step 4. If clean, Step 5.

- [ ] **Step 4: Fix compile errors (only if Step 3 failed)**

| Error | Likely fix |
|---|---|
| `extract_text` signature changed | Inspect the new signature with `cargo doc -p pdf-extract --open` (or read `~/.cargo/registry/src/index.crates.io-.../pdf-extract-0.10.0/src/lib.rs`). Update the call. |
| Error type renamed (e.g. `OutputError` → `PdfError`) | Update the `.map_err` chain. |
| Function renamed entirely | Look for `extract_text_from_path` or similar. |

**If the fix needs more than 5 minutes or touches semantics, ESCALATE per the escalation rule:**
1. Revert the workspace `Cargo.toml` and `Cargo.lock` changes (`git checkout -- Cargo.toml Cargo.lock`).
2. Add `pdf-extract 0.7 → 0.10` to the spec's "Deferred to later slice" section with a one-paragraph reason ("API reshape: `extract_text` now async / returns a different type / etc.").
3. Skip Task 6 entirely; the slice ends after Task 5.

- [ ] **Step 5: Run the PDF tests specifically, then the full suite**

```bash
cd /Users/rjl/Code/mcp-zotero && \
cargo test -p zotero-mcp --test pdf_text 2>&1 | tail -10 && \
cargo test -p zotero-mcp 2>&1 | tail -10
```

Expected: both PASS.

- [ ] **Step 6: Manual smoke test against a fixture PDF**

The fixture at `crates/zotero-mcp/tests/fixtures/hello.pdf` is a real PDF that the existing `pdf_text` tests use. Run a one-line probe to confirm pdf-extract 0.10 still returns non-empty text:

```bash
cd /Users/rjl/Code/mcp-zotero && \
cargo test -p zotero-mcp --test pdf_text -- --nocapture 2>&1 | grep -i "extracted\|hello\|text" | head -5
```

Expected: the tests log non-empty extraction. Exact byte-for-byte output may differ from pre-bump — that's acceptable per the spec's risk-accepted note on pdf-extract drift.

- [ ] **Step 7: Review the lockfile diff**

Run: `cd /Users/rjl/Code/mcp-zotero && git diff Cargo.lock | grep -E "^[-+]name|^[-+]version" | head -25`

Expect `pdf-extract` plus its transitive churn (`lopdf`, `adobe-cmap-parser`, etc.). Flag anything unexpected in the commit message.

- [ ] **Step 8: Commit**

```bash
cd /Users/rjl/Code/mcp-zotero && \
git add Cargo.toml Cargo.lock crates/zotero-mcp/src/core/pdf.rs 2>/dev/null; \
git commit -m "$(cat <<'EOF'
chore(deps): bump pdf-extract 0.7 → 0.10

Call sites verified: 1 use of pdf_extract::extract_text in core/pdf.rs.
Spans 3 minor versions; transitive lopdf bumped. API stable in this
codebase's usage; no source changes [required | applied — list any
here]. Smoke-tested against tests/fixtures/hello.pdf; extraction still
produces non-empty text.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Final: install + restart + version check

After all six tasks (or however many didn't escalate):

- [ ] **Step 1: Rebuild and install**

Run: `cd /Users/rjl/Code/mcp-zotero && cargo install --path crates/zotero-mcp 2>&1 | tail -3`

Expected: `Replacing /Users/rjl/.cargo/bin/zotero-mcp` with the new version.

- [ ] **Step 2: Restart the launchd service**

Run: `launchctl kickstart -k gui/$(id -u)/com.zotero-mcp.http && sleep 2 && launchctl list | grep com.zotero-mcp.http`

Expected: a numeric PID in the first column (service running).

- [ ] **Step 3: Confirm the new SHA is live via the ping probe**

Tell the user: "All six bumps landed. Call `ping` from your MCP client; it should return `pong (v0.2.0, <new-sha>)`. The `<new-sha>` should match `git rev-parse --short HEAD`."

Don't try to invoke the MCP client yourself — that's the user's job.

---

## Verification checklist

After all tasks complete:

- [ ] Six commits (or fewer, if any crate escalated) land in the documented order on `main`.
- [ ] Each commit individually passes `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp`.
- [ ] `Cargo.lock` is included in every dep-bump commit.
- [ ] If any crate escalated, its row appears in the spec's "Deferred to later slice" section with the reason.
- [ ] The final binary is installed at `/Users/rjl/.cargo/bin/zotero-mcp` and the launchd service is running with that binary.
- [ ] The git log shows a clean chronological story: `chore(deps): bump <crate> <old> → <new>` for each landed crate.
