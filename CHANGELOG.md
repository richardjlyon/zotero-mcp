# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.2]

### Fixed

- **MCP clients silently dropped all tools.** Three struct fields typed as
  `serde_json::Value` (`Item.fields`, `FieldChange.current`, `FieldChange.proposed`)
  derived a boolean JSON Schema (`true`) under schemars. Claude Code's tool-schema
  validator rejects a boolean where a property schema is expected and, on that
  rejection, discards the *entire* `tools/list` response — so the server showed as
  "Connected" with zero usable tools. These fields now emit object-form schemas via
  `#[schemars(schema_with = ...)]` (`{}` for free-form values, `{"type": "object"}`
  for `Item.fields`), so the full tool surface registers again.

### Added

- OAuth: defensive alias `/.well-known/openid-configuration` → OAuth authorization
  server metadata, for clients that probe the OIDC discovery path.

## [0.3.1]

### Fixed

- `attach_file`: `imported_file` attachments now write bytes to local Zotero
  storage and omit `md5`/`mtime` from the row body, repairing attachment creation
  for WebDAV users.
