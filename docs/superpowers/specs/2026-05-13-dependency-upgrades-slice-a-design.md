# Spec: Dependency upgrades — Slice A (mechanical bumps)

**Status:** Approved design, ready for plan-writing.
**Author:** Richard Lyon (with Claude Opus 4.7).
**Date:** 2026-05-13.
**Goal:** Bring six pinned dependencies to their latest release where the bump is mechanical (no API-shift requiring redesign). Sets a clean dependency baseline for subsequent slices (B: medium minors, C: schemars 1.x, D: rmcp 1.x).

---

## Problem

A `cargo info` audit (2026-05-13) shows multiple dependencies on older majors:

| Crate | Pinned | Latest |
|---|---|---|
| `directories` | 5 | 6 |
| `rand` | 0.9 | 0.10 |
| `sha2` | 0.10 | 0.11 |
| `md-5` | 0.10 | 0.11 |
| `which` | 7 | 8 |
| `pdf-extract` | 0.7 | 0.10 |
| `rusqlite` | 0.38 | 0.39 |
| `reqwest` | 0.12 | 0.13 |
| `toml` | 0.8 | 1.1 |
| `schemars` | 0.8 | 1.2 |
| `rmcp` | 0.1.5 | 1.6 |

The six in this slice (`directories`, `rand`, `sha2`, `md-5`, `which`, `pdf-extract`) are mechanical: their call sites in this codebase are narrow, the migration cost is bounded, and a regression would surface as a compile error or a test failure rather than a behaviour change.

Three crates are explicitly deferred:

- **`rusqlite 0.38 → 0.39`**: `deadpool-sqlite 0.13` (latest) pins `rusqlite = "0.38"`. Bumping rusqlite alone would either duplicate it in the dependency graph or force replacing the pool. That decision belongs in its own slice.
- **`reqwest 0.12 → 0.13`, `toml 0.8 → 1.1`**: Slice B. Wider surface than Slice A's crates (every HTTP client + the config parser); worth a separate pass.
- **`schemars 0.8 → 1.x`, `rmcp 0.1 → 1.6`**: Slices C and D. Each is a major API rewrite touching the tool macro / Args struct surface. Each requires a research spike before any code lands.

This spec covers Slice A only.

---

## Decisions

1. **Each crate gets its own commit.** Six small commits, one per crate. Each commit contains `Cargo.toml` / workspace `Cargo.toml` edits, any call-site fixes, and the resulting `Cargo.lock` churn. This gives per-crate bisectability if a regression surfaces later. Commit message format: `chore(deps): bump <crate> <old> → <new>` with one bullet per call site touched.

2. **Order: smallest expected delta first.**
   1. `rand 0.9 → 0.10`
   2. `sha2 0.10 → 0.11`
   3. `md-5 0.10 → 0.11`
   4. `directories 5 → 6`
   5. `which 7 → 8`
   6. `pdf-extract 0.7 → 0.10`

   Earlier failures don't compound the diagnostic burden on later crates. `pdf-extract` is last because three minor versions of churn make it the biggest API-risk.

3. **Test gate per commit:** `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp` must both pass before the commit lands. Test failures block the commit; they trigger the escalation rule.

4. **Escalation:** If a crate needs more than mechanical call-site fixes (e.g. an API got removed; an internal type changed in a way that ripples through 10+ call sites; the bump requires touching unrelated modules), back out that crate's edits, add it to the spec's "Deferred to later slice" list with the reason, and continue with the remaining crates. Do not balloon the slice.

5. **No server reinstall between commits.** Only at the very end. Tests are the gate during the slice.

6. **Version specifier style:** match the file's existing format (`"0.10"` style — major.minor for pre-1.0 crates, major-only for 1.0+ crates). No `=` pins, no `^` qualifiers added.

7. **Workspace-vs-crate placement:** edits go where the dep is currently declared. `directories` is in workspace deps; the rest live in `crates/zotero-mcp/Cargo.toml`.

---

## Per-crate plan

### 1. `rand 0.9 → 0.10`

**Touches:** `crates/zotero-mcp/src/oauth.rs` (PKCE code-verifier generation), `crates/zotero-mcp/src/oauth/token_store.rs` (refresh-token random bytes).

**Likely API shifts to watch for:**
- `rand::thread_rng()` → may have been renamed or moved to a different module.
- `rng.gen::<[u8; N]>()` → `rng.random()` (deprecation completed in 0.9.x).
- The `RngCore` and `Rng` trait re-export paths may have shifted.

**Verification:** existing oauth and token_store tests cover both call sites. PKCE-generation tests in `oauth.rs` and `token_store.rs` exercise the random byte paths.

---

### 2. `sha2 0.10 → 0.11`

**Touches:** `crates/zotero-mcp/src/oauth.rs` (PKCE S256 challenge).

**Likely API shifts:** `Sha256::new()` / `Digest::update()` / `Digest::finalize()` are stable across this jump. The `Digest` trait re-export path may have changed.

**Verification:** S256 challenge generation is exercised by the OAuth integration tests.

---

### 3. `md-5 0.10 → 0.11`

**Touches:** `crates/zotero-mcp/src/core/writer/attachments.rs` (Zotero's 3-step upload protocol needs `Content-MD5`).

**Likely API shifts:** identical pattern to sha2 — same `Digest` trait. Migration cost should be zero or one import line.

**Verification:** `writer_attach_file.rs` integration test exercises the upload path.

---

### 4. `directories 5 → 6`

**Touches:** `crates/zotero-mcp/src/core/config.rs`, `crates/zotero-mcp/src/state.rs` (platform-path resolution via `ProjectDirs`).

**Migration concern (load-bearing):** `directories 6.0` may have changed the macOS path convention. If `ProjectDirs::from("dev", "zotero-mcp", "zotero-mcp").unwrap().config_dir()` returns a different path than under 5.0, an existing user's config file becomes invisible to the new build.

**Verification:**
- Compare the path returned by `ProjectDirs::from(...)` under 5.0 (before edit) and 6.0 (after edit). Existing config locations:
  - macOS expected today: `~/Library/Application Support/dev.zotero-mcp.zotero-mcp/config.toml`
  - Linux expected today: `~/.config/zotero-mcp/config.toml`
- If 6.0 returns the same paths: commit as-is.
- If 6.0 returns different paths: revert this bump, add to "Deferred" with the resolved path delta documented so the next attempt can include a one-time migration shim (copy old → new on first launch).

---

### 5. `which 7 → 8`

**Touches:** `crates/zotero-mcp/src/core/pdf.rs` (`resolve_pdftotext` calls `which::which("pdftotext")`).

**Likely API shifts:** `which::which(name)` signature is stable across major bumps. Major bumps in this crate tend to be MSRV bumps. Confirm MSRV: `which 8.0` may require Rust 1.88+; check against `rust-toolchain.toml` (currently `channel = "stable"`, so always recent).

**Verification:** PDF text extraction tests indirectly exercise `which` via the fallback resolution path.

---

### 6. `pdf-extract 0.7 → 0.10`

**Touches:** `crates/zotero-mcp/src/core/pdf.rs` (`PdfExtractEngine::extract`).

**Likely API shifts:** unknown without inspection. Three minor versions of churn. The current code calls `pdf_extract::extract_text(path)` returning `Result<String, _>`. The return type, error type, or function name may have moved.

**Highest escalation risk in the slice.** If the API has shifted in a way that requires nontrivial reshape (e.g. async function, different error type with new variants), defer per the escalation rule.

**Verification:**
- `cargo test -p zotero-mcp` must pass.
- Manual smoke: extract text from `crates/zotero-mcp/tests/fixtures/hello.pdf` and confirm non-empty output. Do not compare byte-for-byte — pdf-extract output drifts even between patch versions; semantic equivalence is acceptable. Note any drift in the commit message.

---

## Risks

1. **`directories 6` moves the on-disk config path.** Documented above. Mitigation: deferral if paths differ; otherwise commit-and-go.

2. **`pdf-extract` output drift.** Even patch versions occasionally produce slightly different extracted text. The codebase already has a `pdftotext` fallback for resilience, and downstream tests check for *some* text rather than *exact* text. Risk accepted; flag drift in the commit message if seen.

3. **MSRV creep.** Any of the bumps could raise the required Rust version above what the project currently expects. The project uses `channel = "stable"` (always recent), so an end-user with a fresh rustup is fine — but a user on an older toolchain might suddenly fail to build. Check each crate's stated MSRV before committing.

4. **`Cargo.lock` transitive churn.** Each `cargo update -p <crate>` may pull a small cloud of transitive updates. Review `git diff Cargo.lock` per commit and note any surprising bumps. This isn't a risk per se but it's worth not blind-staging.

5. **One crate's bump pulls in a new transitive that breaks another crate.** Unlikely with this set; possible. The per-commit test gate catches it.

---

## Out of scope (deferred)

These are documented here so the next slice doesn't re-derive the decision:

- **`rusqlite 0.38 → 0.39`**: blocked by `deadpool-sqlite 0.13` pinning `rusqlite = "0.38"`. Resolution path: (a) wait for deadpool to update, (b) replace deadpool-sqlite with another async-sqlite pool, or (c) fork. Belongs in its own slice.
- **`reqwest 0.12 → 0.13`, `toml 0.8 → 1.1`**: Slice B. Wider call-site surface than Slice A's crates.
- **`schemars 0.8 → 1.x`**: Slice C. Touches every `Args` struct's derive output.
- **`rmcp 0.1.5 → 1.6`**: Slice D. Major API rewrite; rmcp moved from `4t145/rmcp` to `modelcontextprotocol/rust-sdk` and now offers native `auth` and `transport-streamable-http-server` features that could replace or simplify the codebase's hand-rolled `oauth.rs` and `http_transport.rs`. Needs a research spike before any code.

---

## Verification checklist (end of slice)

- [ ] Six commits land in order, one per crate, with the documented commit-message format.
- [ ] Each commit individually passes `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp`.
- [ ] `Cargo.lock` is in each commit, not deferred to a separate "lockfile" commit.
- [ ] Any deferred crates are added to a "Deferred" section in this spec (amend) with the reason.
- [ ] After the final commit: `cargo install --path crates/zotero-mcp`, restart launchd service, confirm `ping` returns the new SHA.
- [ ] The MSRV implication of each new crate is checked and either accepted or noted.

---

## Decisions deferred to implementation

- For each crate, the exact list of call-site edits is discovered during the bump, not pre-listed. The plan documents the discovery process (compile-error-driven) rather than enumerating diffs the spec can't predict.
- Whether to skip MSRV-incompatible crates entirely or bump the project's expectations. Decide per-crate when the issue surfaces.
- Whether to amend a previous commit when fixing a downstream-discovered issue, or land a follow-up fix. Default: follow-up commit (matches the project's no-amend pattern from prior work).
