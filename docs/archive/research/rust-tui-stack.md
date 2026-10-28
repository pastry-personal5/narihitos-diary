# Rust TUI stack research

Status: Archived

Research date: 2026-10-28

This document surveys a Rust software stack for the macOS TUI and its file and
Notion integrations. Versions are a dated compatibility snapshot, not approved
dependencies or a `Cargo.toml` plan.

## Recommended TUI baseline for evaluation

| Concern | Candidate | Research snapshot | Why it fits |
|---|---|---|---|
| Rendering and widgets | `ratatui` | 0.30.2 | Maintained immediate-mode TUI library with layouts, widgets, and a test backend. |
| Terminal backend and events | `crossterm` | 0.29.x | Ratatui's default and most commonly used backend; works on macOS and provides keyboard, paste, resize, raw mode, and alternate-screen support. |
| Multiline diary editing | `ratatui-textarea` | 0.9.3 | Ratatui-maintained fork with multiline editing, selection, undo/redo, wrapping, and compatible modular Ratatui dependencies. |

Ratatui 0.30.2 uses Crossterm 0.29 by default. Ratatui warns that multiple
semver-incompatible Crossterm versions keep separate event queues and raw-mode
state, which can cause lost events or broken terminal restoration. Keep the
Ratatui backend, direct Crossterm dependency, and editor widget on one
compatible version family; verify with `cargo tree -p crossterm` once a manifest
exists.

Use `ratatui-textarea`, not the older `tui-textarea` 0.7 line: the latter is
tied to Ratatui 0.29 and Crossterm 0.28 and would introduce incompatible TUI
types into a Ratatui 0.30 application.

## Application shape supported by the ecosystem

Ratatui redraws the complete interface from application state each frame. Its
documentation presents a model/update/view or action/message organization,
which fits a diary workflow because terminal input, file discovery, and network
completion can all become explicit events.

Important operational properties:

- Use one owner for terminal initialization and restoration.
- Handle only key-press events where appropriate; terminals can also emit
  repeats, releases, focus changes, paste, and resize events.
- Keep rendering free of filesystem and network calls.
- Run iCloud reads and Notion requests away from the render/input path, then
  send typed completion events back to application state.
- Exercise rendering with Ratatui's in-memory `TestBackend` rather than relying
  only on interactive tests.

Ratatui 0.30 provides `ratatui::run` for common initialization/restoration, but
panic and cancellation behavior still need an explicit terminal-restoration
test.

## Network and background work

There is no official Notion Rust SDK; Notion's official SDK is JavaScript. A
Rust application can call the documented REST API directly with:

- `reqwest` 0.13 with JSON support for HTTP and TLS.
- `serde` and `serde_json` for typed request and response bodies.
- `tokio` 1.x if the application adopts asynchronous events and HTTP.

Reqwest also offers a blocking client. That can reduce runtime complexity for a
small MVP only if every request runs on a dedicated worker thread; a blocking
request on the TUI loop would freeze input and prevent status updates. The
Ratatui async tutorial's action/message pattern is a natural fit if Tokio is
chosen.

Whichever model is selected, implement Notion-specific retry policy above the
HTTP client: respect `Retry-After`, do not blindly retry uncertain writes, and
pin `Notion-Version: 2026-03-11`.

## Markdown and metadata reading

The observed input only requires separating YAML front matter from a Markdown
body; it does not yet justify a complete Markdown syntax tree.

Candidate data stack:

| Concern | Candidate | Notes |
|---|---|---|
| Typed YAML front matter | `serde-saphyr` 2 | Maintained, typed, fuzz-tested deserialization with configurable resource budgets; Apache-2.0 OR MIT. Enable only deserialization unless writing YAML is required. |
| Timestamp parsing | `time` 0.3 | Parses offset-aware ISO-8601/RFC 3339 values and integrates with Serde. |
| Directory traversal | `walkdir` 2.5 | Recursive traversal with controllable link following; filter `.md` before parsing. |
| Full Markdown parsing, if later needed | `pulldown-cmark` | Add only when semantic Markdown conversion or rendering is actually required. |

Avoid `serde_yaml` and `serde_yml` for new code; their current documentation
marks that line unmaintained and points to maintained alternatives including
`serde-saphyr`.

Parse front matter into a narrow structure with optional forward-compatible
fields. Preserve the body as text until a confirmed feature requires semantic
Markdown parsing.

## macOS-specific integration

| Concern | Candidate | Tradeoff |
|---|---|---|
| Notion token storage | `keyring` 4 with the Apple-native store | Higher-level and portable; ensure only the macOS backend is enabled. |
| Direct Apple Security access | `security-framework` 3 | Lower-level, macOS-specific Keychain bindings with more control. |
| iCloud download-state APIs | Foundation bindings or a small platform adapter | Needed only if ordinary file reads cannot distinguish or activate cloud-only items. |
| Future sandboxed folder access | AppKit/Foundation adapter | A folder picker and security-scoped bookmark are Apple platform concepts, not portable Rust filesystem APIs. |

Keychain was the research recommendation, but the owner chose a plaintext
per-user global config file requiring owner-only permissions for Phase 1.
Keychain candidates and the disposable probe remain research evidence only.

### IME language indicator

Phase 1 requires research into showing the selected macOS input language in the
TUI while it runs in Apple Terminal. The requested indicator distinguishes
languages such as English and Japanese, not Japanese conversion sub-modes.

The [macOS IME language status research plan](macos-ime-language-status.md)
compares process-global Text Input Source Services with AppKit's
`NSTextInputContext`, using a disposable Rust probe before a production
dependency or interface is selected.

## Local output and reliability support

Useful supporting crates to evaluate only when their requirement is concrete:

- `tempfile` for a temporary file in the destination directory before an
  atomic same-filesystem rename.
- `thiserror` for typed domain and integration errors.
- `tracing` for structured diagnostics, configured to redact all content and
  credential fields.

Avoid adding a large application framework initially. Ratatui plus an explicit
state enum, action enum, and small background-work boundary is sufficient to
test the core workflow. Revisit only if focus management, routing, or component
composition becomes difficult in observed use.

## Compatibility and license notes

- Ratatui 0.30.2 defaults to Crossterm 0.29.
- `ratatui-textarea` 0.9.3 depends on the modular Ratatui 0.30-era crates.
- Ratatui, Crossterm, and `ratatui-textarea` are MIT licensed.
- Reqwest and `serde-saphyr` offer Apache-2.0 OR MIT terms.
- Confirm the complete transitive dependency license set with `cargo deny` or
  equivalent after dependencies are actually selected.
- `serde-saphyr` 2 currently requires Rust 1.89 or newer; the repository's
  eventual `rust-version` must satisfy the highest selected dependency MSRV.

## P1-M1 owner-confirmed technical direction

- Support Apple Silicon on macOS 26.0 or newer and Rust 1.89 or newer. macOS 26
  is required for the selected on-device Foundation Models path; Rust 1.89 is
  the highest current MSRV among the evaluated core candidates.
- Provide a full multiline plain-Markdown editor. Render a preview without
  requiring semantic Markdown editing.
- Keep the TUI loop single-owner and use dedicated bounded worker threads for
  coordinated filesystem reads, Apple model calls, and blocking Notion HTTP.
  Workers return typed events; they use cancellation flags and timeouts and
  never render directly. Tokio is not required for the Phase 1 workload.
- Use a small Foundation adapter to inspect iCloud status, start download for a
  selected cloud-only file, and coordinate its read. Never block the render
  loop on an ordinary read.
- Store the Notion token in a per-user global config file requiring owner-only
  permissions, rather than using a Keychain binding in this phase.
- Use process-global Text Input Source Services notifications for the
  language-only IME label. Poll at 500 ms only when notification registration
  fails; show `IME: ?` when no language is supplied. AppKit input context is
  not the primary source.

A disposable Cargo fixture combined Ratatui 0.30.2, Crossterm 0.29.0,
`ratatui-textarea` 0.9.3, and `security-framework` 3.7.0 with
`rust-version = "1.89"`. All targets and two in-memory editor tests passed, and
only one Crossterm version resolved. The Keychain create/read/delete probe also
passed, but Keychain storage was not selected. See [P1-M1 validation evidence](p1-m1-validation-evidence.md).

## Sources

- [Ratatui 0.30.2 documentation](https://docs.rs/crate/ratatui/0.30.2)
- [Ratatui backend comparison](https://ratatui.rs/concepts/backends/comparison/)
- [Ratatui Crossterm compatibility notes](https://ratatui.rs/concepts/backends/)
- [Ratatui application actions](https://ratatui.rs/tutorials/counter-async-app/actions/)
- [Ratatui testing backend](https://docs.rs/ratatui/latest/ratatui/backend/struct.TestBackend.html)
- [Crossterm 0.29 documentation](https://docs.rs/crate/crossterm/0.29.0)
- [`ratatui-textarea` 0.9.3](https://docs.rs/crate/ratatui-textarea/0.9.3)
- [Reqwest 0.13 documentation](https://docs.rs/crate/reqwest/latest)
- [Tokio documentation](https://docs.rs/crate/tokio/latest)
- [`serde-saphyr` documentation](https://docs.rs/serde-saphyr/latest/serde_saphyr/)
- [`time` ISO-8601 Serde support](https://docs.rs/time/latest/time/serde/iso8601/)
- [Notion's official JavaScript SDK](https://github.com/makenotion/notion-sdk-js)
- [Apple Keychain services](https://developer.apple.com/documentation/security/keychain-services)
- [`keyring` Rust crate](https://docs.rs/crate/keyring/latest)
- [`security-framework` Rust crate](https://docs.rs/security-framework/latest/security_framework/)

