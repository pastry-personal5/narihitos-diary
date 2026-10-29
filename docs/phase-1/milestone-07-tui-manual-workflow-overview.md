# P1-M7: TUI and manual workflow overview

Status: Planned

This milestone delivers the first human-usable vertical slice: start or resume
a workflow, select sources, write manually, and confirm the complete entry in
the TUI. It deliberately stops before output writes.

## Goal

Let a person use `nd` as an HTTP client of `nd serve` to reach a confirmed,
recoverable diary entry with or without source notes.

## Entry criteria

- P1-M12 is `Done`, the phase plan marks P1-M7 `Active`, and M12's revised
  contract, runtime-attachment loader, source-note names, explicit
  no-notes transition, conditional-write behavior, and event semantics have
  been inspected. The client must not guess DTO shapes or compensate for a
  missing server operation.
- The owner approves the exact dependency and macOS FFI packet below before any
  manifest, workspace-member, feature, or crate-level unsafe-code-policy change.
- Use synthetic workflows and fake API/IME/terminal implementations in
  automated tests. The real-terminal check must use test text only; it must
  not read a private Ayumi directory or live Notion root.

## Pre-implementation decision and contract gate

P1-M12 owns and requalifies the three lower-layer corrections found by the
2026-10-29 M7 review: the safe runtime-advertisement loader, source-note
names, and direct explicit no-notes transition. M7 consumes those implemented
contracts and must not add a TUI-side file probe, source-name cache, fake
discovery result, or alternate no-notes transition.

The recommended M7 packet is pending owner approval:

Version requirements are exact (`=` in Cargo manifests), and default features
are off unless the table says otherwise.

| Owning package | Exact direct dependency or boundary | Purpose, features, and alternative |
|---|---|---|
| `diary-tui` | `ratatui = 0.30.2`, default features off, `crossterm_0_29` only | Rendering, layout, and in-memory test backend on Rust 1.89; MIT. Avoids calendar, macros, serialization, and unstable features. A handwritten renderer would duplicate tested width/layout machinery. |
| `diary-tui` | `crossterm = 0.29.0`, default features off, `events` and `bracketed-paste` only | Apple Terminal ownership and input; MIT. Pin the same 0.29 family selected by Ratatui and verify one resolved Crossterm version. |
| `diary-tui` | `ratatui-textarea = 0.9.3`, default features off | Multiline buffer, cursor, selection, undo/redo, and wrapping without its backend or regex feature; MIT. M7 supplies its own key dispatch so editor defaults cannot consume global chords. |
| `diary-tui` | `ureq = 3.4.2`, default features off | Blocking HTTP/1.1 on bounded worker threads with no TLS, proxy, compression, cookie, JSON, or async-runtime feature; MIT OR Apache-2.0 and Rust 1.85 MSRV. Existing Serde/JSON dependencies do bounded encoding/decoding. A handwritten HTTP/SSE parser is security-sensitive; Reqwest would add an async runtime and unused loopback TLS surface. |
| proposed `diary-macos` boundary crate | No third-party dependency; minimal Carbon/HIToolbox and Core Foundation linkage | Isolate the audited unsafe TIS query/notification/CF-ownership code from `diary-tui`, which keeps `#![forbid(unsafe_code)]`. The alternative is weakening that invariant inside the large TUI crate. No second helper process or build script is introduced. |

The same approval sets one ordinary HTTP worker with a 16-command queue, one
SSE worker with one active stream, and a 32-result channel. Connect, send, and
response-header phases are capped at 2, 5, and 5 seconds; ordinary response
bodies at 30 seconds; and each SSE body window at 45 seconds, after which it
reconnects with `Last-Event-ID`. Reconnect backoff starts at 250 ms and doubles
to a 4-second ceiling. All pins are compatible with Rust 1.89 and the repository
license policy based on their published metadata. Approval authorizes only
these manifest/workspace changes; immediately after resolution and before
implementation code, verify the graph, features, duplicate Crossterm count,
MSRVs, and licenses with Cargo metadata/tree inspection. Stop and return to the
owner if the lockfile does not match the packet.

## In scope

- Terminal ownership/restoration, MVVM client, typed API client, SSE reconnect,
  connection/version states, Setup check, and workflow list/discard. The client
  safely reads the owner-only runtime advertisement, verifies unauthenticated
  health and the matching instance/version before it attaches the bearer token
  to any request, and never starts the server.
- The six canonical stages (`Start`, `Sources`, `Compose`, `Review`, `Preview`,
  `Publish`) in the left Workflow Sidebar. M7 enables the manual flow through
  Review and renders later stages as locked; later milestones add behavior
  without renaming or inserting stages.
- A full-width Work Status pane above the Workflow Sidebar, Resources Sidebar,
  and Main Pane. Its four content rows explain current work and retain useful
  safe messages; the compact bottom Status Line remains and may repeat the most
  important state.
- Source selection/detail, selected-only preparation, empty-day confirmation,
  Draft editor, entry review, and stage invalidation UX. The middle Resources
  Sidebar distinguishes the active source from the included set; activating a
  prepared or confirmed source renders its readable text in the Main Pane.
- Multiline autosave, recovery, per-entry language/style selection, remembered
  last language, concise style default, Korean/English display width, focus
  navigation, and Command Line. Any inferred-reflection delimiter received from
  a future assistant is rendered for review and blocks entry confirmation until
  removed; this milestone does not run an assistant.
- TIS notification-based IME state with polling fallback and real-terminal
  acceptance for Korean IME behavior.

## Out of scope

- Assistant extraction, drafting, paragraph review, output preview,
  publication, amendments, and MCP. P1-M8 adds assistant behavior inside the
  existing Compose stage and shared layout.
- Bare-letter shortcuts, a TUI-side database/source access, or a second durable
  workflow authority. The TUI may retain unsent editor text in memory only.

## Completion checklist

### Client attachment and workflow authority

- [ ] P1-M12 is `Done` with its lower-layer contract evidence, and no client
  workaround duplicates it. The owner-approved M7 dependency/FFI packet matches Cargo metadata, the
  lockfile, and the workspace structure before implementation begins.
- [ ] `nd` attaches only through a validated runtime file and matching health
  response, never starts the server, never logs the token, and shows
  not-running, connecting, reconnecting, connected, and version-mismatch
  states. A new server instance causes a complete reattachment with a new token
  and event cursor. It fetches canonical state before re-enabling mutations
  after a reconnect, event reset/gap, unknown event, or local queue overflow.
- [ ] A person can start, resume, or discard a workflow; discover and inspect
  sources; maintain a local prospective include/exclude set; prepare selected
  cloud-only notes; review their readable snapshots; and confirm a fresh
  selection through server commands. Starting with no diary-day value uses the
  server's Mac-local default. Conditional-write and job-conflict responses
  preserve the local intent, explain the conflict safely, and require a state
  reload rather than retrying blindly.
- [ ] The explicit no-notes path requires its confirmation, hides unavailable
  assistant actions, opens the Draft editor, and refuses a blank confirmed
  entry.

### Layout, editor, and interaction

- [ ] The standard layout has a full-width Work Status pane above a left
  Workflow Sidebar, middle Resources Sidebar, and right Main Pane. Work Status
  has exactly four content rows excluding its border and title, shows current
  work plus useful safe messages, and may intentionally duplicate important
  state from the compact bottom Status Line. The Workflow Sidebar always uses
  the canonical `Start`, `Sources`, `Compose`, `Review`, `Preview`, and
  `Publish` order with server-derived progress state and one presentation-only
  current marker; Preview and Publish are visibly locked in M7.
  Both sidebars remain accessible through focus when terminal width collapses
  either one; resource state is explicit and trees retain their hierarchy. The
  minimum supported terminal size is 80 columns by 30 rows; below it the client
  shows only the terminal-too-small notice.
- [ ] In Sources, active focus and inclusion are independent: activating a
  prepared source renders its sanitized readable text in the Main Pane without
  changing inclusion, while include/exclude changes only the prospective set.
  In Compose, the Resources Sidebar lists the draft and confirmed sources;
  moving between a read-only source and the editable draft preserves unsent
  text and editor position.
- [ ] The multiline editor autosaves with the server's separate buffer version,
  permits at most one save in flight, never lets an older response replace newer
  keystrokes, retains unsent text during a transient disconnect, warns before an
  unsafe quit, and recovers a server draft without representing stale or
  recovered text as confirmed. A reload or conflict keeps both local unsent and
  canonical server text visible for deliberate resolution.
- [ ] Entry language and style are represented by the server-owned workflow;
  only the last language and optional assistant backend ID are remembered, while every new
  workflow defaults style to concise. The selected output language is visible
  in the editor and review. Reserved inferred-reflection delimiters, if present,
  identify the first unresolved span and refuse confirmation.
- [ ] ViewModel and renderer tests cover canonical stage mapping and impact
  notices, Work Status priority and four-row rendering, intentional Status Line
  duplication, active-resource versus included-source behavior, prepared and
  confirmed source-body rendering, source warnings/date disagreements, server
  invalidation, conditional-save conflicts, `YYYY-MM-DD` and space-permitting
  `YYYY-MM-DD (Mon)` rendering, terminal-size fallback, grapheme/display width,
  safe visible rendering of terminal control characters, entry byte limits,
  and content-free errors using an in-memory terminal and fake API client.
- [ ] Focus/Enter and Ctrl-G (F2), Ctrl-Enter (F3), Ctrl-D (F10), and
  Ctrl-Left/Ctrl-Right (F6/F7) expose every manual-flow action. Command Line
  ASCII tokens work under English and 2-Set Korean input without forbidden
  chords, and every typed command has an equivalent focused control.
- [ ] TIS notification and 500 ms fallback behavior is tested. A real Apple
  Terminal check records terminal restoration after normal exit, handled
  failure, and panic unwind, plus Korean-IME navigation, exact chord/alias
  recognition, Command Line mapping, and `EN`/`KO`/unknown indicator evidence.

### Evidence and gate

- [ ] User-facing, UX, and client-attachment documentation matches the
  implemented manual slice, including the actual chosen chords.
- [ ] Acceptance evidence below records focused synthetic tests and a dated full
  gate: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`, `swift build --package-path helpers/apple-bridge`,
  and `swift test --package-path helpers/apple-bridge`.

## Acceptance evidence

Fill this in during implementation. Leave checklist items unchecked until their
results are observed; record concise synthetic evidence, not private diary text
or raw logs.

| Evidence | Result |
|---|---|
| P1-M12 lower-layer readiness evidence and owner-approved M7 dependency/FFI packet | Pending |
| Runtime attachment, reconnect, and conditional-mutation tests | Pending |
| Source-selection, empty-day, autosave, recovery, and confirmation tests | Pending |
| In-memory Work Status, rendering, accessibility, width, and Korean-command mapping tests | Pending |
| Real Apple Terminal restoration and IME check | Pending |
| Documentation inspection and full Rust/Swift gate, with date and tool versions | Pending |
