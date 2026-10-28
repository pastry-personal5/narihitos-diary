# P1-M5: Source ingestion and Apple bridge overview

Status: Planned

This milestone connects the P1-M4 source ports to Ayumi Markdown and the
single Swift helper. It ends when the core can confirm a fresh, sanitized
snapshot of exactly the selected notes, without an HTTP or terminal UI.

## Goal

Discover bounded source candidates, read selected notes safely through iCloud,
and detect source changes before composition or output.

## Entry criteria

- P1-M3 and P1-M4 are `Done`, and the phase plan marks P1-M5 `Active` before
  implementation. Use their actual source-port and helper-envelope contracts.
- The P1-M3 helper uses one UTF-8 JSON request and response frame per process,
  each capped at 64 KiB, with request IDs capped at 128 UTF-8 bytes.
  Source-byte handoff must preserve that envelope; changing it or adding a
  dependency or build tool follows the approval rule in
  [AGENTS.md](../../AGENTS.md).
- Use only synthetic fixtures. A read of the private Ayumi tree requires
  separate owner authorization and is not a milestone exit requirement.

## In scope

- Validate the configured iCloud directory name and discover only `D-1`, `D`,
  `D+1`, or one separately requested date. Return stable connector-scoped
  references, availability, date evidence, and per-file warnings.
- Parse UTF-8 Markdown with optional BOM and LF or CRLF; tolerate missing or
  malformed front matter when the body boundary is clear. Use only valid,
  offset-aware `created_at` as header-date evidence.
- Remove supported standalone audio-attachment links from readable bodies;
  withhold ambiguous attachment syntax that could expose a path. Never inspect
  attachment files or treat their presence as an eligibility condition.
- Add Swift-helper iCloud availability, selected-only download, and coordinated
  read commands, with bounded input, output, time, and cancellation.
- Stamp selected snapshots with file identity, size, modification time, and an
  original-byte digest. Recheck freshness at the P1-M4 checkpoints and return
  `Changed` or `Unverifiable` rather than silently using stale text.

## Out of scope

- HTTP resources and jobs (P1-M6), TUI source presentation (P1-M7), drafting
  (P1-M8), output publication (P1-M9 and P1-M10), and amendments (P1-M11).
- Language detection, metadata-based language rejection, archive-wide scans,
  automatic discovery beyond the three-day window, and reading unselected
  cloud-only bodies.
- Live Notion, real model calls, and private diary fixtures.

## Completion checklist

### Discovery and source evidence

- [ ] Synthetic-tree tests prove root-name validation, no symlink or root
  escape, regular `.md` filtering, per-file diagnostics, stable references,
  deterministic de-duplication, the `D-1`/`D`/`D+1` bound, and a single
  explicit extra-date query with no wider scan.
- [ ] Candidate tests show directory and offset-local `created_at` dates as
  separate evidence, flag disagreements, and leave inclusion independent of
  either date. A cloud-only candidate remains discoverable with directory
  evidence when its header is unavailable, without triggering download.

### Parsing and privacy

- [ ] Parser tests cover UTF-8 with and without BOM, LF and CRLF, empty and
  non-empty bodies, missing and closed-but-malformed front matter, an unclosed
  header, unknown or missing fields, invalid dates and offsets, and invalid
  UTF-8. A safely separated body stays selectable; unsafe or undecodable bytes
  never become empty readable text.
- [ ] Synthetic Markdown tests prove standalone audio-link removal, preservation
  of ordinary prose and non-audio links, attachment-independent eligibility,
  and safe failure for ambiguous attachment syntax. Raw front matter, audio
  bytes, and attachment paths are absent from core-readable text, persisted
  snapshots, diagnostics, and composition-shaped requests.

### iCloud, snapshots, and recovery

- [ ] Fake-helper and Swift fake tests cover availability, selected-only
  download, coordinated read, revalidation of the requested source, bounded
  source-byte handoff beyond the 64 KiB JSON-frame limit, and cleanup of
  temporary handoff data on success, failure, and next start.
- [ ] Tests distinguish cloud-only, permission denied, unreadable, missing
  helper, malformed protocol, crash, timeout, cancellation, and oversized
  source outcomes without leaking private paths or content. No failure is
  interpreted as an empty note.
- [ ] Original-byte digest tests cover same-size and same-mtime edits, changed
  identity, removal, and unverifiable rereads. Integration tests exercise the
  post-read, pre-drafting-pass, pre-entry-confirmation, pre-preview, and
  pre-first-write checkpoints through P1-M4, proving the exact selected set,
  draft/preview/approval invalidation, and successful reread plus
  reconfirmation.

### Evidence and gate

- [ ] Affected source-format and architecture docs match the implemented
  contracts, including any source-size limit and handoff cleanup behavior.
- [ ] The acceptance evidence below records focused tests and a dated full
  gate: `cargo fmt --all -- --check`, `cargo clippy --all-targets
  --all-features -- -D warnings`, `cargo test --all-features`, `swift build
  --package-path helpers/apple-bridge`, and `swift test --package-path
  helpers/apple-bridge`.

## Acceptance evidence

Fill this in during implementation. Leave checklist items unchecked until
their results are observed; record concise synthetic evidence, not raw logs.

| Evidence | Result |
|---|---|
| Entry criteria and approved contract inspection | Pending |
| Discovery, date evidence, and root containment tests | Pending |
| Parser, audio filtering, and data-boundary tests | Pending |
| Swift fake and Rust fake-helper failure/cleanup tests | Pending |
| Freshness and P1-M4 invalidation integration tests | Pending |
| Full Rust and Swift gate, with date and tool versions | Pending |
