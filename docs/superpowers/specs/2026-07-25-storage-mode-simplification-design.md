# Spec: Storage-mode simplification — `attach_file` stops second-guessing Zotero

**Status:** Approved design (specified 2026-05-15 in the vault; ported to this repo 2026-07-25), ready for plan-writing.
**Author:** Richard Lyon (with Claude Opus 5).
**Date:** 2026-07-25.
**Source of truth:** `~/Resilio/second-brain/Projects/Zotero MCP — Storage Mode Simplification.md`.
**Goal:** Remove the `imported_file` vs `linked_file` decision from the agent-facing abstraction and from config. `attach_file(parent_key, file_path)` attaches the way Zotero's own UI would; `mode` survives as an advanced escape hatch defaulting to `null`. `attachment_mode` and `linked_attachment_base_dir` are deprecated (warn + ignore), not removed.

---

## Problem

From an agent's point of view there is no `imported_file` vs `linked_file` distinction — there is just *attach this file to this item*. Zotero already has user-level file-sync preferences (Zotero cloud, WebDAV, no sync) that decide where bytes end up. The connector duplicated that decision at its own layer, and the duplication was load-bearing on three separate mistakes (see the vault spec for the evidence chain):

1. The 2026-05-14 HIGH-PRIORITY bug was misdiagnosed as "config not honoured" when the real cause was two unrelated bugs in the Web-API protocol and the row-body shape.
2. `attachment_mode = "linked_file"` + `linked_attachment_base_dir = "/Users/rjl/Resilio/Zotero"` were written on a false premise — the actual portable-library mechanism was a pre-existing WebDAV-to-Unraid sync.
3. A recovery agent's whole brief was framed around "restoring 579 broken `linked_file` attachments" — framing that was unnecessary.

Today `attach_file_t` (`tools/attachments.rs:268`) reads `cfg.attachment_mode` as the default and threads `cfg.linked_attachment_base_dir` into the call, and the tool description in `server.rs:358` tells the model to reason about both. The config participates in a decision that is not the connector's to make.

---

## Decisions

1. **`mode` defaults to `null`, meaning "the way Zotero's UI would".** Concretely that is the `imported_file` path fixed in v0.3.1: bytes at `<data_dir>/storage/<key>/<filename>`, `syncState` left for the desktop client's sync engine. `mode` is **kept** in the schema (removing it is a breaking change) but re-documented as an advanced escape hatch. `mode: "linked_file"` still works.

2. **Config no longer participates.** `attach_file_t` never reads `cfg.attachment_mode` or `cfg.linked_attachment_base_dir`. With no config base dir, an explicit `mode: "linked_file"` call stores the file's absolute path — behaviour already covered by `linked_file_without_base_dir_uses_absolute_path`.

3. **Deprecate, don't remove.** Both fields stay in `ZoteroConfig` so existing `config.toml` files parse cleanly:
   - `attachment_mode: Option<String>` (was `String` with a `default_attachment_mode()` fn — the fn goes).
   - `linked_attachment_base_dir: Option<String>` (unchanged type).

   Neither is read by any call path. Removal is scheduled for v0.5.x.

4. **Deprecation warnings are a pure function, logged by `load()`.** `Config::deprecation_warnings(&self) -> Vec<String>` returns one human-readable string per deprecated field that is present; `Config::load()` emits each via `tracing::warn!`. Rationale: asserting on `tracing` output in a unit test is awkward and flaky, whereas asserting on the returned `Vec` is exact. The stderr side is one line of glue over a tested function.

5. **`AttachmentOutsideBaseDir`'s message drops its config reference.** The variant is now only reachable through the core API (`AttachFileOptions.linked_attachment_base_dir = Some(..)`), which the tool layer never sets. The message stops naming `linked_attachment_base_dir` (a config key that no longer does anything) and stops advising `mode = "imported_file"` (the default now). New hint: omit `mode` and let Zotero store the file its own way.

6. **The `AttachmentMode` enum, `from_config`, and the whole `linked_file` write path stay.** The simplification is at the default and the agent-facing prose, not the capability. `from_config` keeps its warn-and-fall-back-to-`ImportedFile` behaviour for unknown strings, which is now also the parse path for a bad per-call `mode` argument.

7. **`attach_link` (URL attachments) unchanged. No migration script.** Per the vault spec's out-of-scope list.

8. **Version:** the CHANGELOG entry goes under `[Unreleased]`, which is already carrying a due minor bump. Cutting the release is Richard's call, not part of this change.

---

## Touchpoints

| File | Change |
|---|---|
| `crates/zotero-mcp/src/core/config.rs` | `attachment_mode` → `Option<String>`; drop `default_attachment_mode()`; both fields documented DEPRECATED; add `deprecation_warnings()`; log from `load()`; update the two existing tests |
| `crates/zotero-mcp/src/core/error.rs` | Reword `AttachmentOutsideBaseDir`; update its test assertion |
| `crates/zotero-mcp/src/core/writer/attachments.rs` | Doc-comment only: `AttachFileOptions.linked_attachment_base_dir` and `attach_file` no longer describe a config-driven default |
| `crates/zotero-mcp/src/tools/attachments.rs` | `attach_file_t` defaults to `ImportedFile` with no config read; `AttachFileArgs.mode` doc rewritten |
| `crates/zotero-mcp/src/server.rs` | `attach_file` tool description rewritten — no `cfg.zotero.*` references |
| `crates/zotero-mcp/tests/writer_attach_file.rs` | New guard test: default options route to the imported-file path |
| `crates/zotero-mcp/tests/writer_live_zotero.rs` | Comment/wording only — the live tests construct `AttachFileOptions` directly and stay valid |
| `README.md` | Rewrite the storage-mode config block, the `attach_file` tool-table row, and the `AttachmentOutsideBaseDir` troubleshooting entry |
| `CHANGELOG.md` | `[Unreleased] → Changed` + `Deprecated` entries |

**Not touched:** `bearer.rs`, `oauth.rs`, `oauth/token_store.rs`, `http_transport.rs`, `setup.rs` (the things-mcp mirror set). Nothing in this change needs them.

---

## Tests (written before implementation)

1. `config.rs`: `attachment_mode_absent_by_default` — `Config::default()` has both fields `None` and `deprecation_warnings()` is empty.
2. `config.rs`: `deprecated_attachment_fields_parse_and_warn` — a `config.toml` containing `attachment_mode = "linked_file"` and `linked_attachment_base_dir = "..."` parses without error and `deprecation_warnings()` returns two strings naming the two keys.
3. `writer_attach_file.rs`: `default_options_route_to_imported_file` — `AttachFileOptions` with `mode: ImportedFile` and `linked_attachment_base_dir: None` posts a `linkMode: "imported_file"` row and writes bytes to `<storage>/<key>/<filename>` (mock-server assertion, mirroring the existing `imported_file_creates_row_without_md5_...` test but asserting specifically on the *no-base-dir, no-config* shape).
4. `error.rs`: existing `attachment_outside_base_dir_message_includes_hint` updated — asserts both paths appear and the hint no longer names `linked_attachment_base_dir`.

The canonical imported-file write path is already covered by `imported_file_creates_row_without_md5_and_writes_bytes_to_storage`; this change does not alter it.

---

## Risks

1. **Someone's `config.toml` genuinely relies on `attachment_mode = "linked_file"`.** Richard's does not (verified 2026-05-15 — the linked-file config was written on a false premise). The deprecation warning plus the surviving per-call `mode` argument is the documented migration path.
2. **`AttachmentOutsideBaseDir` becomes dead from the MCP surface.** Deliberate: the core function keeps the guard for direct API callers and its tests. Not removed, because removing an error variant is a bigger break than leaving one unreachable-from-one-caller.
3. **Type change on `attachment_mode` (`String` → `Option<String>`) breaks any code reading it.** Only `attach_file_t` read it, and that read is being deleted. Compiler catches anything missed.

---

## Acceptance

- `attach_file(parent_key, file_path)` with no `mode` routes to the imported-file path.
- A config with `attachment_mode = "linked_file"` parses cleanly, warns, and changes nothing about where bytes go.
- No `cfg.zotero.attachment_mode` or `cfg.zotero.linked_attachment_base_dir` read anywhere in the call path (`grep` clean outside config.rs and its tests).
- README storage-mode section and CHANGELOG rewritten.
- `cargo test -p zotero-mcp` green; `cargo clippy` no new warnings.
- No commit, no reinstall, no launchd restart — Richard's call.
