# P1-M5: Source ingestion and Apple bridge overview

Status: Done

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

This checklist is the implementation completion contract. Items stay unchecked
until the corresponding synthetic test or full-gate evidence is recorded below.

### Implementation-review hardening

- [x] Discovery never follows a date-directory or source symlink, never reads a
  body outside the validated root, bounds the opened file even when it changes
  after metadata inspection, reports non-UTF-8 and unsupported entries without
  exposing private names, and returns stable deterministic candidates.
- [x] The shared Markdown parser validates the complete offset-aware
  `created_at` value without accepting a nested or duplicate field, preserves a
  safely separated body when front matter is malformed, accepts BOM/LF/CRLF
  envelopes while rejecting lone CR separators, and distinguishes an unsafe
  unclosed header from an empty readable body.
- [x] Audio filtering removes only standalone local audio links, preserves
  remote and ordinary links, and withholds multi-link, image, reference-style,
  or otherwise ambiguous attachment syntax without returning the attachment
  path in diagnostics.
- [x] Freshness stamps use a real filesystem identity and original-byte digest;
  selected reads detect metadata races and classify disappearance or failed
  revalidation as unverifiable rather than as an unknown selection.

### Discovery and source evidence

- [x] Synthetic-tree tests prove root-name validation, no symlink or root
  escape, regular `.md` filtering, per-file diagnostics, stable references,
  deterministic de-duplication, the `D-1`/`D`/`D+1` bound, and a single
  explicit extra-date query with no wider scan.
- [x] Candidate tests show directory and offset-local `created_at` dates as
  separate evidence, flag disagreements, and leave inclusion independent of
  either date. A cloud-only candidate remains discoverable with directory
  evidence when its header is unavailable, without triggering download.

### Parsing and privacy

- [x] Parser tests cover UTF-8 with and without BOM, LF and CRLF, empty and
  non-empty bodies, missing and closed-but-malformed front matter, an unclosed
  header, unknown or missing fields, invalid dates and offsets, and invalid
  UTF-8. A safely separated body stays selectable; unsafe or undecodable bytes
  never become empty readable text.
- [x] Synthetic Markdown tests prove standalone audio-link removal, preservation
  of ordinary prose and non-audio links, attachment-independent eligibility,
  and safe failure for ambiguous attachment syntax. Raw front matter, audio
  bytes, and attachment paths are absent from core-readable text, persisted
  snapshots, diagnostics, and composition-shaped requests.

### iCloud, snapshots, and recovery

- [x] Fake-helper and Swift fake tests cover availability, selected-only
  download, coordinated read, revalidation of the requested source, bounded
  source-byte handoff beyond the 64 KiB JSON-frame limit, and cleanup of
  isolated per-request handoff data on success, failure, cancellation, timeout,
  concurrent reads, and next start.
- [x] Tests distinguish cloud-only, permission denied, unreadable, missing
  helper, malformed protocol, crash, timeout, cancellation, and oversized
  source outcomes without leaking private paths or content. No failure is
  interpreted as an empty note.
- [x] Original-byte digest tests cover same-size and same-mtime edits, changed
  identity, removal, and unverifiable rereads. Integration tests exercise the
  post-read, pre-drafting-pass, pre-entry-confirmation, pre-preview, and
  pre-first-write checkpoints through P1-M4, proving the exact selected set,
  draft/preview/approval invalidation (including helper access failure), and
  successful reread plus reconfirmation.

### Evidence and gate

- [x] Affected source-format and architecture docs match the implemented
  contracts, including any source-size limit and handoff cleanup behavior.
- [x] The acceptance evidence below records focused tests and a dated full
  gate: `cargo fmt --all -- --check`, `cargo clippy --all-targets
  --all-features -- -D warnings`, `cargo test --all-features`, `swift build
  --package-path helpers/apple-bridge`, and `swift test --package-path
  helpers/apple-bridge`.

## Acceptance evidence

All fixtures are invented and all helper services or executables are fake. No
test reads the private Ayumi tree or calls a live iCloud service.

| Evidence | Result |
|---|---|
| Entry criteria and approved contract inspection | On 2026-10-29, implementation preserved the dependency-free Rust/Swift-helper contract, v1 envelope, 64 KiB frame, and one-process-per-request lifetime. It added no dependency, feature, build tool, provider decision, or private-data access. |
| Discovery, date evidence, and root containment tests | Seven synthetic Ayumi tests pass for directory-name validation, three-day and explicit-day bounds, year crossing, deterministic order, unsupported-entry diagnostics, exact reference grammar, directory/source symlink rejection, root containment, separate directory/header evidence, disagreement, and cloud-only discovery without a body read. |
| Parser, audio filtering, and data-boundary tests | Synthetic tests pass for UTF-8/BOM/LF/CRLF envelopes, lone-CR rejection, empty and non-empty bodies, absent, malformed, nested, duplicate, invalid, and unclosed front matter, offset semantics, invalid bytes, local standalone audio removal, remote and ordinary link preservation, multi-link/image/reference attachment withholding, selected sanitized snapshots, bounded opened-file reads, and content-free diagnostics. |
| Swift fake and Rust fake-helper failure/cleanup tests | Ten Swift tests cover the three iCloud commands, bounded payloads, ASCII calendar source IDs, fake service errors, correlation, frame limits, and content-free responses. Eight Rust client tests cover deterministic lookup, concurrency, missing/crashed/timed-out/cancelled/malformed helpers, strict response JSON, protocol-capped download waits, typed permission and oversize failures, a 70,000-byte private handoff, owner/mode/identity/size checks, isolated concurrent handoffs, and cleanup on success, failure, and next start. |
| Freshness and P1-M4 invalidation integration tests | Original-byte digests detect forced same-size/same-mtime edits; device/inode identity detects replacement; removal is unverifiable. Helper-backed tests confirm exactly one selected snapshot, all five checkpoints, source and helper-access invalidation with retained user edits, and successful reread plus human reconfirmation. SQLite v3 round-trips availability and upgrades v1 and v2. |
| Full Rust and Swift gate, with date and tool versions | Revalidated after implementation review on 2026-10-29: formatting, warnings-denied workspace Clippy, `cargo test --all-features` (23 connector, 17 core, 4 command-shell, 1 M4 assembly, and 1 M5 assembly test, plus doctests), Swift build, Swift test (10 cases), and `git diff --check`. Versions: rustc/Cargo 1.98.0, Swift 6.4 (swift-driver 1.168.6); the existing completion run recorded rustfmt 1.9.0, Clippy 0.1.98, and Xcode 27.1 (27A9275). Swift commands required the expected sandbox escalation for the user-level Clang module cache. |
