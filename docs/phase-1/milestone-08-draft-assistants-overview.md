# P1-M8: Draft assistants overview

Status: Planned

This milestone adds optional reviewed two-pass composition behind the existing
`DraftAssistant` port while preserving the always-available manual editor. It
cannot start until the hosted-provider decisions in `AGENTS.md` are resolved.

## Goal

Offer hosted-default and Apple on-device memory extraction and drafting without
allowing model output to bypass human review or the approved data-egress limit.

## Decision gate

Before implementation, the owner must choose the hosted provider, credential
storage, and HTTP dependency. Record those decisions in `AGENTS.md` and the
Phase 1 changelog; do not infer them from this plan.

## In scope

- Availability and capability reporting for hosted and Apple assistants.
- Source-linked memory extraction, memory review, drafting, inferred-reflection
  markers, validation, cancellation, timeouts, and manual fallback.
- Swift helper model commands and the selected hosted adapter.
- Composition-path and language preferences and visible path labels.
- Type-level and runtime enforcement of the hosted-data egress envelope.

## Out of scope

- Automatic confirmation, output approval, publication, and agent-client MCP.
- Sending front matter, metadata, paths, audio, credentials, or Notion details.
- Model use for a confirmed empty source selection.

## Completion checklist

- [ ] The hosted provider, credential location, and HTTP dependency have explicit
  owner approval and are documented before manifests or adapters change.
- [ ] Both adapters implement the same `DraftAssistant` contract and report
  availability, language support, timeouts, cancellation, and invalid output as
  typed outcomes that preserve the manual path.
- [ ] Tests prove that hosted requests can contain only readable bodies,
  memories, drafts, edited text, language, and style—not excluded data classes.
- [ ] Extraction returns source-linked memories for review, and drafting consumes
  only confirmed memories and never confirms its own result.
- [ ] Inferred reflections remain visibly delimited and block entry confirmation
  until reviewed; malformed or oversized output is rejected safely.
- [ ] Apple helper tests cover availability, unsupported locale, crash, timeout,
  cancellation, and schema-invalid output without calling a live model.
- [ ] The TUI labels the active path in Setup, editor, and Status Line and offers
  the other path or manual editing after failure.
- [ ] A synthetic end-to-end composition test covers both adapters and preserves
  source freshness checks before each pass.
- [ ] User-facing privacy/composition documentation names the chosen provider
  behavior and the exact egress boundary.
- [ ] The standard Rust and Swift milestone gate passes and acceptance evidence
  is recorded.
