# P1-M8: Draft assistants architecture plan

Status: Planned

This plan fills the `DraftAssistant` port with an extensible backend registry
only after its undecided hosted dependencies are owner-approved.

## Affected modules and interfaces

- `diary-core`: backend descriptors and selection, extraction, memory review,
  drafting, paragraph-review suggestions, validation, preferences, and
  freshness-check use cases bounded by `DraftAssistant`.
- `diary-connectors`: chosen hosted adapter and Swift helper client adapter.
- `helpers/apple-bridge`: versioned Foundation Models command group.
- `diary-control` and contract: collection-shaped backend capability state,
  extraction/draft/review jobs, and versioned suggestion accept/reject.
- `diary-tui`: backend selection, Memory review, assisted-editor, source
  inspection, and paragraph-suggestion states inside Compose.

## Backend and suggestion boundaries

Startup registers `Arc<dyn DraftAssistant>` implementations by stable backend
ID. Core and transport consume a list of descriptors and resolve an ID for each
assistant command; they never switch on a provider enum. A descriptor reports
its display label, `hosted` or `apple-on-device` composition path, availability,
supported languages, and `extract-memories`, `draft-entry`, and
`review-paragraph` capabilities. Manual composition is a workflow action, not a
registry member.

Extraction and drafting receive only server-owned confirmed source snapshots.
The TUI submits neither source bodies nor arbitrary paths, and the active source
in the Resources Sidebar has no effect on the request. The server performs the
freshness checkpoint, resolves exactly one backend ID, constructs the allowed
request type, and records the backend ID on the result for display and recovery.
Source text is untrusted. The port offers no tools or callback by which an
adapter can read another source, run a shell command, call an output connector,
or perform a workflow transition. Agent-style adapters must disable those
capabilities in their own process/API configuration as well as omit them from
the Rust port.

A paragraph target is the current entry-buffer version, the UTF-8 byte range of
the Markdown paragraph containing the editor cursor (blank-line delimited and
shown before submission), and a digest of those exact bytes. The assistant
returns a proposed replacement plus an optional bounded explanation. The server
stores it as an `AssistantSuggestion`; accept conditionally applies it and
advances the buffer version, while reject marks or removes it without mutating
the entry. Any edit that changes the target makes the suggestion stale. Neither
operation confirms the complete entry.

P1-M6 contract version 1 represents the manual editor as a `manual`
composition path and persists that value as the last path. P1-M8 makes a
versioned, fail-closed contract change: capabilities report
`manual_workflow_available` separately from `assistant_backends[]`, writing
settings carry an optional backend ID, and the backend descriptor owns the
hosted/on-device path. The database migration preserves existing workflows and
language/style settings, maps the old manual preference to no selected backend,
and lets normal default selection choose the hosted backend only after it is
available. Mixed-version client/server pairs stop at version mismatch.

## Approach and sequence

1. Resolve and document the provider, credential, and HTTP choices, including
   license and configuration consequences, before dependency changes.
2. Version the control contract and migrate stored manual/path preferences to
   the separated manual-availability, optional-backend-ID model. Add old-store
   and mixed-client/server tests before assistant calls exist.
3. Define backend descriptors and a registry keyed by stable ID. Prove with at
   least three fake adapters that capabilities, selection, persistence, and
   routing are collection-shaped rather than a binary hosted/Apple branch.
4. Define extraction, drafting, and paragraph-review request types constructible
   only from the approved data classes, plus versioned suggestion acceptance;
   centralize size, timeout, cancellation, response validation, and the
   no-tools/no-side-effects adapter boundary.
5. Implement hosted availability and calls without logging text or exposing its
   credential through core, database, API, process arguments, or errors.
6. Add the helper's on-device command group with the same structured outcomes
   and fake all model behavior in tests.
7. Wire extraction, draft, and paragraph-review jobs through freshness
   checkpoints and human memory/entry review. Route one command to one backend,
   retain manual text on failure, and refuse stale paragraph acceptance.
8. Extend the existing Compose ViewModel: show all registered backends, keep
   confirmed sources and the draft in the Resources Sidebar, render suggestions
   in the Main Pane, and expose explicit accept/reject actions.
9. Prove the production adapter contracts and a three-fake registry through
   synthetic contract tests and fake-backed vertical flows.

The hosted default is a preference, not a consent screen; the selected backend
and active path stay visible. No adapter receives confirmation, approval, or
publication capabilities. A future Codex-backed adapter uses the same registry
only after its own undecided operational choices are approved; a Codex MCP
client remains a separate future inbound client.
