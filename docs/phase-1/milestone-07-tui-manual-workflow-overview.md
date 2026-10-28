# P1-M7: TUI and manual workflow overview

Status: Planned

This milestone delivers the first human-usable vertical slice: start or resume
a workflow, select sources, write manually, and confirm the complete entry in
the TUI. It deliberately stops before output writes.

## Goal

Let a person use `nd` as an HTTP client of `nd serve` to reach a confirmed,
recoverable diary entry with or without source notes.

## Entry criteria

- P1-M6 is `Done`, the phase plan marks P1-M7 `Active`, and its implemented
  contract, runtime-file lifecycle, conditional-write behavior, and event
  semantics have been inspected. The TUI must not guess planned DTO shapes.
- Before changing a Cargo manifest, feature, build tool, or the chosen terminal
  stack, prepare an owner-reviewed dependency packet covering the owning
  package, purpose, Rust 1.89 and Apache-2.0 compatibility, feature set,
  transitive concerns, and an already-approved or standard-library alternative.
- Use synthetic workflows and fake API/IME/terminal implementations in
  automated tests. The real-terminal check must use test text only; it must
  not read a private Ayumi directory or live Notion root.

## In scope

- Terminal ownership/restoration, MVVM client, typed API client, SSE reconnect,
  connection/version states, Setup check, and workflow list/discard. The client
  reads the owner-only runtime file, verifies unauthenticated health and the
  matching instance/version before it sends its bearer token, and never starts
  the server.
- Start, source selection/detail, selected-only preparation, empty-day
  confirmation, manual editor, entry review, and step invalidation UX.
- Multiline autosave, recovery, entry language/style selection and remembered
  preferences, Korean/English display width, focus navigation, and Command
  Line. Any inferred-reflection delimiter received from a future assistant is
  rendered for review and blocks entry confirmation until removed; this
  milestone does not run an assistant.
- TIS notification-based IME state with polling fallback and real-terminal
  acceptance for Korean IME behavior.

## Out of scope

- Model extraction/drafting, output preview, publication, amendments, and MCP.
- Bare-letter shortcuts, a TUI-side database/source access, or a second durable
  workflow authority. The TUI may retain unsent editor text in memory only.

## Completion checklist

### Client attachment and workflow authority

- [ ] `nd` attaches only through a validated runtime file and matching health
  response, never starts the server, never logs the token, and shows
  not-running, connecting, reconnecting, connected, and version-mismatch
  states. It fetches canonical state before re-enabling mutations after a
  reconnect or event gap.
- [ ] A person can start, resume, or discard a workflow; discover and inspect
  sources; include or exclude them; prepare selected cloud-only notes; review
  their readable snapshots; and confirm a fresh selection through API calls
  only. Conditional-write and job-conflict responses preserve the local intent,
  explain the conflict safely, and require a state reload rather than retrying
  blindly.
- [ ] The explicit no-notes path requires its confirmation, hides unavailable
  model actions, opens the manual editor, and refuses a blank confirmed entry.

### Editor and interaction

- [ ] The standard layout renders a persistent Workflow Sidebar with each
  stage's server-derived step state and a contextual Resources Sidebar for
  source notes, inputs, and later output resources. Both sidebars remain
  accessible through focus when terminal width collapses either one; resource
  state is explicit and trees retain their parent-child hierarchy.
- [ ] The multiline editor autosaves with the server's separate buffer version,
  retains unsent text during a transient disconnect, warns before an unsafe
  quit, and recovers a server draft without representing stale or recovered
  text as confirmed. A reload or conflict keeps local unsent text visible for
  deliberate resolution.
- [ ] Entry language and style are represented by the server-owned workflow and
  remembered preferences; the selected output language is visible in the editor
  and review. Reserved inferred-reflection delimiters, if present, identify the
  first unresolved span and refuse confirmation.
- [ ] ViewModel and renderer tests cover step states and impact notices,
  source warnings/date disagreements, server invalidation, conditional-save
  conflicts, `YYYY-MM-DD` and space-permitting `YYYY-MM-DD (Mon)` rendering,
  terminal-size fallback, grapheme/display width, and content-free errors using
  an in-memory terminal and fake API client.
- [ ] Focus/Enter and Ctrl-G (F2), Ctrl-Enter (F3), Ctrl-D (F10), and
  Ctrl-Left/Ctrl-Right (F6/F7) expose every manual-flow action. Command Line
  ASCII tokens work under English and 2-Set Korean input without forbidden
  chords, and every typed command has an equivalent focused control.
- [ ] TIS notification and 500 ms fallback behavior is tested. A real Apple
  Terminal check records terminal restoration after normal exit and failure,
  plus Korean-IME navigation, Command Line mapping, and `EN`/`KO`/unknown
  indicator evidence.

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
| P1-M6 entry inspection and owner-approved dependency packet, if needed | Pending |
| Runtime attachment, reconnect, and conditional-mutation tests | Pending |
| Source-selection, empty-day, autosave, recovery, and confirmation tests | Pending |
| In-memory rendering, accessibility, width, and Korean-command mapping tests | Pending |
| Real Apple Terminal restoration and IME check | Pending |
| Documentation inspection and full Rust/Swift gate, with date and tool versions | Pending |
