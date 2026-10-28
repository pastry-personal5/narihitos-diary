# P1-M5: Source ingestion and Apple bridge overview

Status: Planned

This milestone makes Ayumi Markdown notes discoverable and readable through
the source ports, including selected-only iCloud acquisition through the Swift
helper. It stops at a confirmed fresh source snapshot.

## Goal

Reliably discover, prepare, parse, filter, and freshness-check exactly the
source notes a person selects, without exposing private metadata or audio.

## In scope

- Bounded `D-1`, `D`, `D+1` discovery and one explicit extra-date query.
- Directory-date and offset-aware `created_at` evidence, warnings, selection,
  UTF-8/BOM/line-ending handling, and tolerant front matter.
- Audio-link and attachment-path removal before bodies enter composition types.
- Swift helper commands for iCloud availability, selected-only download, and
  coordinated reads, with size, timeout, cancellation, and fake tests.
- File identity, metadata, original-byte digest, and checkpoint freshness.

## Out of scope

- HTTP or terminal presentation, drafting, output publication, and live Notion.
- Language detection or metadata-based source rejection.
- Scanning outside the approved dates or reading unselected cloud-only notes.

## Completion checklist

- [ ] Synthetic catalog tests prove the three-day automatic bound, explicit
  extra-date behavior, stable opaque source IDs, and both date-evidence rules.
- [ ] Parser contract tests cover UTF-8 with and without BOM, LF and CRLF,
  malformed or missing front matter, unknown fields, invalid dates, invalid
  UTF-8, and readable empty/non-empty bodies.
- [ ] Audio and attachment references never enter the readable body supplied to
  the core, while attachment presence never changes eligibility.
- [ ] Only selected cloud-only notes trigger helper acquisition, and helper
  crash, timeout, cancellation, missing binary, and unreadable outcomes remain
  distinct and content-free.
- [ ] Coordinated-read snapshots digest original bytes and freshness tests cover
  changed and unverifiable files at every core checkpoint.
- [ ] No test reads private diary content; macOS-specific code is isolated from
  portable connectors and exercised through a fake helper.
- [ ] Affected source-format and architecture documentation matches the adapter
  contract and observed limitations.
- [ ] The standard Rust and Swift milestone gate passes and acceptance evidence
  is recorded.
