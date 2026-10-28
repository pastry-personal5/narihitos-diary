# P1-M7: TUI and manual workflow overview

Status: Planned

This milestone delivers the first human-usable vertical slice: start or resume
a workflow, select sources, write manually, and confirm the complete entry in
the TUI. It deliberately stops before output writes.

## Goal

Let a person use `nd` as an HTTP client of `nd serve` to reach a confirmed,
recoverable diary entry with or without source notes.

## In scope

- Terminal ownership/restoration, MVVM client, typed API client, SSE reconnect,
  connection/version states, Setup check, and workflow list/discard.
- Start, source selection/detail, selected-only preparation, empty-day
  confirmation, manual editor, entry review, and step invalidation UX.
- Multiline autosave, recovery, language/style preferences, inferred-marker
  refusal, Korean/English display width, focus navigation, and Command Line.
- TIS notification-based IME state with polling fallback and real-terminal
  acceptance for Korean IME behavior.

## Out of scope

- Model extraction/drafting, output preview, publication, amendments, and MCP.
- Bare-letter shortcuts or any TUI-side database/source access.

## Completion checklist

- [ ] `nd` attaches through the runtime file, never starts the server, and shows
  not-running, connecting, reconnecting, connected, and version-mismatch states.
- [ ] A person can start/resume/discard a workflow, discover and inspect sources,
  include or exclude them, prepare selected cloud notes, and confirm a fresh
  selection through API calls only.
- [ ] The explicit no-notes path disables model actions, opens the manual editor,
  and refuses a blank confirmed entry.
- [ ] The multiline editor autosaves by buffer version, preserves unsent text
  through transient disconnects, warns before unsafe quit, and recovers server
  drafts without treating stale text as confirmed.
- [ ] ViewModel tests cover navigation states, invalidation impact notices,
  warnings, date disagreements, terminal-size fallback, display width, and
  content-free errors with an in-memory terminal and fake API client.
- [ ] Focus/Enter and chosen Ctrl chords expose every action; Command Line ASCII
  tokens work under English and 2-Set Korean input without forbidden chords.
- [ ] TIS notification and 500 ms fallback behavior is tested, and a real Apple
  Terminal check records restoration and Korean IME evidence.
- [ ] User-facing and UX documentation matches the implemented manual slice.
- [ ] The standard Rust and Swift milestone gate passes and acceptance evidence
  is recorded.
