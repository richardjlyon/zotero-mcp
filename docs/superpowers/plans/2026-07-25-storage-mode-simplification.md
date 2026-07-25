# Storage-mode Simplification — Implementation Plan

**Goal:** `attach_file` defaults to "attach the way Zotero's UI would"; `attachment_mode` and `linked_attachment_base_dir` become deprecated no-ops (warn + ignore). Tests first.

**Spec:** `docs/superpowers/specs/2026-07-25-storage-mode-simplification-design.md`.

**Constraint:** the things-mcp mirror files (`bearer.rs`, `oauth.rs`, `oauth/token_store.rs`, `http_transport.rs`, `setup.rs`) are out of bounds. Nothing here touches them.

---

## Task 1: Tests (red)

- [ ] **Step 1 — config tests.** In `core/config.rs`'s `mod tests`, replace `attachment_mode_defaults_to_imported_file` with `attachment_mode_absent_by_default`:

```rust
#[test]
fn attachment_mode_absent_by_default() {
    let c = Config::default();
    assert!(c.zotero.attachment_mode.is_none());
    assert!(c.zotero.linked_attachment_base_dir.is_none());
    assert!(c.deprecation_warnings().is_empty());
}
```

  and replace `attachment_mode_parses_from_toml` with:

```rust
#[test]
fn deprecated_attachment_fields_parse_and_warn() {
    let toml = r#"
[zotero]
attachment_mode = "linked_file"
linked_attachment_base_dir = "/Users/rjl/Resilio/Zotero-Attachments"
"#;
    let c: Config = toml::from_str(toml).expect("deprecated keys must still parse");
    let warnings = c.deprecation_warnings();
    assert_eq!(warnings.len(), 2);
    assert!(warnings.iter().any(|w| w.contains("attachment_mode")));
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("linked_attachment_base_dir"))
    );
    // Present but inert: nothing in the call path consults them.
    assert!(warnings.iter().all(|w| w.contains("ignored")));
}
```

- [ ] **Step 2 — error message test.** Update `attachment_outside_base_dir_message_includes_hint` in `core/error.rs`: keep the two path assertions, replace `assert!(s.contains("imported_file"))` with an assertion that the hint tells the caller to omit `mode` and does *not* name the dead config key:

```rust
assert!(s.contains("omit"));
assert!(!s.contains("linked_attachment_base_dir"));
```

- [ ] **Step 3 — imported-file default guard.** *(Revised during implementation: the spec sketched a mock-server test in `tests/writer_attach_file.rs`, but that would have duplicated `imported_file_creates_row_without_md5_and_writes_bytes_to_storage` almost line for line — same options, same assertions. The behaviour that actually changes is mode **resolution** in the tool layer, so the guard goes there instead. The end-to-end imported-file write path stays covered by the existing test.)*

  Extract the resolution rule in `tools/attachments.rs` as a pure function and unit-test it:

```rust
/// Resolve the storage mode for one `attach_file` call. Config does not
/// participate: an omitted `mode` means "store it the way Zotero's own UI
/// would", which is the imported-file route.
fn resolve_mode(mode: Option<&str>) -> AttachmentMode { ... }
```

```rust
#[test]
fn omitted_mode_routes_to_imported_file() {
    assert_eq!(resolve_mode(None), AttachmentMode::ImportedFile);
}

#[test]
fn explicit_linked_file_still_honoured() {
    assert_eq!(resolve_mode(Some("linked_file")), AttachmentMode::LinkedFile);
}

#[test]
fn unknown_mode_falls_back_to_imported_file() {
    assert_eq!(resolve_mode(Some("nonsense")), AttachmentMode::ImportedFile);
}
```

  `tools/attachments.rs` has no test module today; this introduces one.

- [ ] **Step 4 — run and confirm red.** `cargo test -p zotero-mcp` must fail on the three new/changed tests (`deprecation_warnings` does not exist yet → compile error is an acceptable red).

---

## Task 2: Implementation (green)

- [ ] **Step 5 — `core/config.rs`.**
  - `attachment_mode: String` → `Option<String>`, `#[serde(default)]`, doc comment marked **DEPRECATED (v0.4.0)**: parsed, warned about, ignored; removal at v0.5.x; per-call `mode` on `attach_file` is the escape hatch.
  - `linked_attachment_base_dir`: same DEPRECATED doc treatment.
  - Delete `fn default_attachment_mode()`.
  - `impl Default for ZoteroConfig`: both fields `None`.
  - Add to `impl Config`:

```rust
/// Human-readable warnings for deprecated config keys that are present
/// but no longer do anything. Logged at WARN by [`Config::load`];
/// returned rather than logged directly so it is exactly testable.
pub fn deprecation_warnings(&self) -> Vec<String> { ... }
```

    One string per present field, each naming the key, saying it is **ignored**, and naming the replacement (nothing — Zotero's own file-sync preference decides; pass `mode` per call if you really need `linked_file`).
  - `Config::load()`: after a successful parse, `for w in cfg.deprecation_warnings() { tracing::warn!("{w}") }`. Must fire on the file path only — `Config::default()` has nothing to warn about, so a bare `for` over the result is safe either way.

- [ ] **Step 6 — `core/error.rs`.** Reword `AttachmentOutsideBaseDir`:

```
attachment file {file_path} is not inside the linked-attachment base
directory ({base_dir}). Move it in first, or omit `mode` to let Zotero
store the file the way its own UI would.
```

- [ ] **Step 7 — `tools/attachments.rs`.** In `attach_file_t`:

```rust
let mode = a
    .mode
    .as_deref()
    .map(AttachmentMode::from_config)
    .unwrap_or(AttachmentMode::ImportedFile);
let opts = AttachFileOptions {
    mode,
    linked_attachment_base_dir: None,
    ...
};
```

  No `cfg.attachment_mode`, no `cfg.linked_attachment_base_dir`. `cfg` is still needed for `max_attachment_bytes` and `path_map`. Rewrite the `AttachFileArgs.mode` doc comment: advanced escape hatch, omit it and the file is stored the way Zotero's UI would; `"linked_file"` stores only a path reference for BYO-storage setups.

- [ ] **Step 8 — `core/writer/attachments.rs`.** Doc comments only: `AttachFileOptions.linked_attachment_base_dir` notes that the MCP tool layer never sets this (direct API callers may); `attach_file`'s doc drops any implication of a config-driven default.

- [ ] **Step 9 — `server.rs` tool description.** Replace the `attach_file` description with prose that carries no config reference. Target text:

```
Attach a local file to a Zotero parent item. The file is stored the way
Zotero's own UI would store it, and Zotero's file-sync preference (cloud,
WebDAV, or none) decides where the bytes go from there — that is not this
server's decision. Input: { parent_key, file_path (absolute), mode?,
filename?, content_type? }. `mode` is an advanced escape hatch: omit it
unless you specifically need "linked_file" (Zotero stores only a path
reference, for BYO-storage setups). Returns { attachment_key }.
```

  Keep the existing `annotations(...)` block untouched.

- [ ] **Step 10 — `tests/writer_live_zotero.rs`.** Wording only: the `// Step 2: attach_file (imported_file)` comments and the module header should read as "the default route", not "the configured mode". Both live tests keep constructing `AttachFileOptions` directly and remain valid, including the `linked_file` roundtrip (the capability stays).

- [ ] **Step 11 — build + test.** `cargo build -p zotero-mcp`, then `cargo test -p zotero-mcp`. All green, including the previously red three.

- [ ] **Step 12 — grep gate.**

```bash
grep -rn "attachment_mode\|linked_attachment_base_dir" crates/zotero-mcp/src/
```

  Expected hits only in `core/config.rs` (the deprecated fields, their doc comments, `deprecation_warnings`, and its tests). Any hit in `tools/`, `server.rs`, or `core/writer/` is a miss.

---

## Task 3: Docs

- [ ] **Step 13 — README.** Three edits:
  1. Config block (~line 515): replace the `attachment_mode` / `linked_attachment_base_dir` stanza with a short DEPRECATED note — the keys are accepted and ignored, will be removed at v0.5.x, and attachments are stored the way Zotero's UI stores them (`<data_dir>/storage/<key>/<filename>`), synced per the user's own Zotero file-sync preference. Cross-link Zotero's file-sync docs (<https://www.zotero.org/support/sync#file_syncing>).
  2. Tool-table row for `attach_file` (~line 431): drop the two-modes framing; "attaches the way Zotero's UI would; `mode` is a rarely-needed escape hatch".
  3. Troubleshooting entry for `AttachmentOutsideBaseDir` (~line 594): it can no longer arise from config — only from an explicit `mode: "linked_file"` call against a base dir set by a direct API caller. Reword accordingly.

- [ ] **Step 14 — CHANGELOG.** Under `[Unreleased]`:
  - **Changed** — `attach_file` no longer decides storage mode. `mode` defaults to null, meaning "the way Zotero's own UI would store it"; Zotero's file-sync preference owns where bytes go. `linked_file` remains available per call.
  - **Deprecated** — `[zotero] attachment_mode` and `[zotero] linked_attachment_base_dir`. Present-but-ignored, a WARN is logged at startup, removal at v0.5.x.

---

## Hand-off

- [ ] Report the touched-file list and the test count to Richard.
- [ ] **No commit, no `cargo install`, no `launchctl kickstart`.** Richard's call.
