# Spec: Dependency upgrades — Slice B (HTTP client + config parser)

**Status:** Approved design, ready for plan-writing.
**Author:** Richard Lyon (with Claude Opus 4.7).
**Date:** 2026-05-13.
**Goal:** Bring `reqwest` and `toml` to their latest majors. Both were deferred from Slice A as "wider call-site surface than the mechanical six" — still likely mechanical, but with enough exposure (32 call sites across 17 files combined) that a separate slice keeps the diagnostic burden small if either escalates.

---

## Problem

After Slice A landed (`rand`, `sha2`, `md-5`, `directories`, `which`, `pdf-extract`), four pinned deps remain on the audit board:

| Crate | Pinned | Latest |
|---|---|---|
| `reqwest` | 0.12.28 | 0.13.3 |
| `toml` | 0.8.23 | 1.1.2 |
| `schemars` | 0.8 | 1.x |
| `rmcp` | 0.1.5 | 1.6 |

`schemars` (Slice C) touches every `Args` struct's derive output; `rmcp` (Slice D) is a major rewrite that warrants a research spike before any code. Both stay deferred.

This spec covers `reqwest` and `toml` — the two remaining mechanical-ish bumps. Surface area verified against HEAD (`53acea6`):

- **`reqwest`**: 21 uses across 13 files — `state.rs`, `core/bbt.rs`, `core/error.rs`, `core/web.rs`, the four writer modules (`notes`, `client`, `items`, `attachments`, `tags`), the four enrichment providers (`openlibrary`, `crossref`, `semantic_scholar`, `arxiv`). Pinned at workspace level as `reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "gzip", "brotli"] }`. A single major bump (0.12 → 0.13).
- **`toml`**: 11 uses across 4 files — `oauth.rs`, `setup.rs`, `core/config.rs`, `core/error.rs`. Pinned at workspace level as `toml = "0.8"`. Crosses the 0.x → 1.x major boundary (0.8 → 1.1.2), but mostly via `from_str` / `to_string` / `Value` usage that has held stable in spirit across that jump.

The MSRV floors for both crates (reqwest 1.64, toml 1.66) are far below the local toolchain (`rustc 1.93`, `stable` channel via `rust-toolchain.toml`). No MSRV pressure.

---

## Decisions

1. **Each crate gets its own commit.** Two small commits, one per crate. Per-crate bisectability; identical pattern to Slice A.

2. **Order: reqwest first, toml second.**
   1. `reqwest 0.12 → 0.13` — single major, broad surface. Get the HTTP plumbing green first so toml lands on stable ground.
   2. `toml 0.8 → 1` — crosses the 0.x → 1.x boundary, narrower surface. Second pole; lower escalation cost if it needs to defer.

   Commit-message format: `chore(deps): bump <crate> <old> → <new>` with body documenting call-site verification and any source changes.

3. **Test gate per commit:** `cargo build -p zotero-mcp` AND `cargo test -p zotero-mcp` must both pass before the commit lands. Hard requirement.

4. **Escalation rule:** if either crate needs more than mechanical call-site fixes — anything beyond a renamed function, a moved trait import, a feature-flag rename, or a one-file `From`-impl tweak — back out that crate's edits (`git checkout -- Cargo.toml Cargo.lock <touched sources>`), add it to this spec's "Deferred to later slice" section with a one-paragraph reason, and continue with the remaining crate. Out-of-bounds: async-converting existing sync calls, new error variants that need handling at call sites, ripple changes across more than ~5 files in non-identical shapes.

5. **No server reinstall between commits.** Reinstall + launchd restart only at slice end.

6. **Version specifier style** matches the workspace's existing conventions:
   - `reqwest = "0.13"` (pre-1.0 → `major.minor`).
   - `toml = "1"` (post-1.0 → `major` only; matches the workspace pattern used for `which`, `directories`, `pdf-extract`, `clap`, etc.).
   - Existing reqwest features (`rustls-tls`, `json`, `gzip`, `brotli`) and `default-features = false` are preserved unless a feature was renamed upstream — in which case the renamed name replaces the old, and the change is noted in the commit body.

7. **Workspace-vs-crate placement:** both deps are declared at workspace level (`/Users/rjl/Code/github/zotero-connector/Cargo.toml`, lines 20 and 27). Edits go in the workspace `Cargo.toml`, not in `crates/zotero-mcp/Cargo.toml`.

---

## Per-crate plan

### 1. `reqwest 0.12 → 0.13`

**Touches:** workspace `Cargo.toml` (line 20). Possibly source — see "Likely API shifts" below.

**Call-site surface:** 21 uses across 13 files. Most uses are the standard `reqwest::Client` builder pattern plus `Response::json()` / `Response::error_for_status()` plumbing. The HTTP transport (`state.rs`), every enrichment provider, and every writer module all use the same shape.

**Likely API shifts to watch for** (confirmed during plan-time changelog research):
- **Feature-flag renames** — `rustls-tls` is the historically most-renamed flag; confirm it still exists or substitute. The other features in use (`json`, `gzip`, `brotli`) are stable.
- **`ClientBuilder` method changes** — e.g. `danger_accept_invalid_certs` deprecation, `connect_timeout` semantic changes.
- **`Response::error_for_status()`** signature stability (the codebase chains this in several places).
- **`Error` type variants** — confirmed at call-site survey: zero exhaustive matches on `reqwest::Error` in the codebase. All conversions are via `?` into a custom error type, so new upstream variants are absorbed transparently.
- **`multipart::Form` and `Body`** builder pattern changes — relevant for `writer/attachments.rs` (Zotero file upload).

**Verification:**
- Full `cargo test -p zotero-mcp` (lib + integration suite) covers writer modules, enrichment providers, OAuth flows.
- The HTTP transport and OAuth integration tests are the most direct exercise of the reqwest call chain.

---

### 2. `toml 0.8 → 1`

**Touches:** workspace `Cargo.toml` (line 27). Possibly source — see "Likely API shifts" below.

**Call-site surface:** 11 uses across 4 files. The codebase uses toml primarily as a config parser:
- `core/config.rs` — read/write the main project config (`config.toml`).
- `oauth.rs` and `setup.rs` — read/write `oauth.toml`.
- `core/error.rs:113` — `From<toml::de::Error>` impl on the codebase's error enum. The serialize side (`toml::ser::Error`) is not converted; serialize errors are produced inside `core/config.rs` and bubble up through their own paths.

**Likely API shifts to watch for** (confirmed during plan-time changelog research):
- **`toml::from_str` / `toml::to_string` / `toml::to_string_pretty`** — the most-used entry points; historically stable. Signatures should hold.
- **`toml::Value` enum** — the 1.x line may have consolidated variants (`Datetime` handling moved into the `toml_datetime` crate). The codebase uses `toml::Value` opaquely (no pattern-match seen at call-site survey), but plan-time research must confirm.
- **`toml::de::Error`** — may have been renamed or merged into a single top-level `toml::Error` in the 1.x line. If so, the single `From<toml::de::Error>` impl in `core/error.rs:113` needs updating to match — one line, one file, mechanical.
- **`Serializer` / `Deserializer` builder API** — only matters if the codebase uses custom serializer config (it does not, based on call-site survey).

**Verification:**
- The OAuth tests round-trip `oauth.toml` through `from_str` / `to_string`.
- The config tests in `core/config.rs` cover the main config path.
- Setup-flow tests cover `setup.rs`.

---

## Risks

1. **Reqwest feature-flag drift.** Most likely friction point. If `rustls-tls` renamed, that's a one-line `Cargo.toml` fix — still mechanical. If multiple features renamed in non-obvious ways or a feature combination became invalid, escalate.

2. **Toml error-type rename.** If `toml::de::Error` was renamed or absorbed into a top-level `toml::Error` in the 1.x line, the single `From<toml::de::Error>` impl in `core/error.rs:113` needs updating. One line, one file — still mechanical, but mark it as the most likely source-edit site.

3. **Transitive lockfile churn.** Reqwest 0.13 will pull a different `hyper` / `http` / `tokio-rustls` / `rustls` cohort. Per-commit lockfile diff review catches surprises; expect ~50–200 line lockfile diff for reqwest.

4. **MSRV creep.** Both crates' stated MSRVs (reqwest 1.64, toml 1.66) are far below local toolchain. No pressure.

5. **One crate's bump pulls in a new transitive that breaks the other crate.** Unlikely. The per-commit test gate catches it.

6. **Reqwest 0.13 + rustls cohort interaction with `pdf-extract`'s new crypto cluster.** Slice A introduced `aes`, `cbc`, `ecb`, `cipher`, `inout`, `block-padding` via `lopdf 0.38`. Reqwest 0.13's rustls dependency may pull a different version of `aes`-family crates; if so, the lockfile will resolve a single version per crate by SemVer rules, which usually works but could surface a version-resolution stalemate. Lockfile review catches it.

---

## Out of scope (deferred)

Carried forward from Slice A's deferred list — unchanged by this slice:

- **`rusqlite 0.38 → 0.39`**: blocked by `deadpool-sqlite 0.13` pinning `rusqlite = "0.38"`. Belongs in its own slice once deadpool updates or is replaced.
- **`schemars 0.8 → 1.x`**: Slice C. Touches every `Args` struct's derive output; needs a research spike on the new attribute syntax.
- **`rmcp 0.1.5 → 1.6`**: Slice D. Major API rewrite; rmcp moved to `modelcontextprotocol/rust-sdk` and now offers native `auth` and `transport-streamable-http-server` features that could replace or simplify the codebase's hand-rolled `oauth.rs` and `http_transport.rs`. Needs a research spike before any code.

---

## Verification checklist (end of slice)

- [ ] Two commits land in order, one per crate, with the documented commit-message format.
- [ ] Each commit individually passes `cargo build -p zotero-mcp` and `cargo test -p zotero-mcp` (lib tests still at 105 passing).
- [ ] `Cargo.lock` is in each commit, not deferred to a separate "lockfile" commit.
- [ ] Any deferred crates are added to this spec's "Deferred to later slice" section (amend) with the reason.
- [ ] After the final commit: `cargo install --path crates/zotero-mcp`, restart launchd service (`launchctl kickstart -k gui/$(id -u)/com.zotero-mcp.http`), confirm `ping` returns the new SHA.
- [ ] The MSRV implication of each new crate is checked and accepted (both well below local toolchain).

---

## Decisions deferred to implementation

- The exact list of call-site edits is discovered during the bump (compile-error-driven), not pre-listed in the spec. The plan documents the discovery process.
- Whether to skip MSRV-incompatible crates entirely or bump the project's expectations. Not expected to surface in Slice B.
- Whether to amend or land a follow-up fix. Default: follow-up commit (matches project convention from Slice A).
- Reqwest 0.13's specific feature-flag renames and error-variant additions will be enumerated by plan-time research (changelog read), not invented in this spec.
- Toml 1.x's specific API consolidations (Value variant merges, Error type unification) likewise.
