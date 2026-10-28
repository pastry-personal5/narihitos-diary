# Phase 1: macOS TUI MVP

Status: Active

Goal: Make remembering a day feel effortless through a macOS terminal user
interface that turns existing Markdown notes into a final diary entry and
publishes its body to a Notion daily section.

## Scope

- Run as a TUI application (`nd`) on macOS, a client of a local Control API
  Server that a human starts with `nd serve`, plus a Swift helper for iCloud and
  the optional on-device model.
- Read Markdown (`.md`) files from a mounted iCloud directory.
- Use the Ayumi directory date and offset-aware Markdown-header `created_at`
  date to propose source notes for a diary day, visibly flag date disagreements,
  and let the user include or exclude discovered notes regardless of their
  metadata dates.
- Ignore Ayumi audio attachments; they are not played, copied, uploaded,
  decoded, transcribed, or supplied to diary composition.
- Compose the entry in a multiline editor, with optional reviewed two-pass
  drafting: hosted by default (provider undecided) or Apple on-device.
- Produce one final diary entry as a Markdown file on the local filesystem.
- Amend a completed entry, replacing only content the app itself wrote.
- Use the configured Notion root page with `YYYY` year and `YYYY-MM` month
  subpages; append the confirmed Markdown body as a daily section at the
  bottom of its month page.

## Exit criteria

- A user can start the server with `nd serve` and interact with it through the
  `nd` TUI on macOS.
- A user can draft with a model adapter or write manually; model output is never
  accepted without review, and a blank entry cannot be confirmed.
- A completed entry can be amended without overwriting content the app did not
  write.
- The application can read Markdown source files from a mounted iCloud
  directory.
- Before composition, the TUI shows the source-note selection and any directory
  versus header date disagreement; the user can include or exclude discovered
  notes even when their metadata date does not match the diary day.
- The final diary entry is composed from exactly the confirmed source-note
  selection.
- Ayumi entries remain eligible with or without audio attachments, and neither
  audio bytes nor attachment paths appear in composition input or output.
- A completed run writes the final diary entry as a local Markdown file.
- The completed run creates missing Notion year/month subpages as needed and
  publishes the confirmed entry body under its diary-day heading at the
  bottom of the target month page.

## Milestones

### P1-M1: MVP research and decisions

Status: Done

Goal: Turn the existing MVP research into an evidence-backed, decision-ready
foundation for later Phase 1 planning.

Plan: [overview](milestone-01-research-overview.md),
[research approach](milestone-01-research-architecture.md)

Notes: The owner interview, Notion hierarchy evidence, and remaining runtime
checks are in the [research summary](milestone-01-research-summary.md). This
research-only milestone used its documented exit-gate exemption.

### P1-M2: Software architecture

Status: Active

Goal: Define the Phase 1 software architecture.

Plan: [overview](milestone-02-software-architecture-overview.md),
[architecture plan](milestone-02-software-architecture-architecture.md),
[product architecture](../planned-mvp-architecture.md).

### P1-M3: Scaffolding and tools

Status: Planned

Goal: Establish the Rust project scaffold and development tools.

Plan: [overview](milestone-03-scaffolding-overview.md),
[architecture plan](milestone-03-scaffolding-architecture.md).

### P1-M4: Core workflow and persistence

Status: Planned

Goal: Implement the diary state machine and recoverable private state behind
inward-facing ports.

Plan: [overview](milestone-04-core-workflow-persistence-overview.md),
[architecture plan](milestone-04-core-workflow-persistence-architecture.md).

### P1-M5: Source ingestion and Apple bridge

Status: Planned

Goal: Discover and read confirmed Ayumi source snapshots, including selected-only
iCloud acquisition through the Swift helper.

Plan: [overview](milestone-05-source-ingestion-apple-bridge-overview.md),
[architecture plan](milestone-05-source-ingestion-apple-bridge-architecture.md).

### P1-M6: Control API Server

Status: Planned

Goal: Expose the core through the secure, loopback-only `nd serve` REST, event,
job, and lifecycle contract.

Plan: [overview](milestone-06-control-api-server-overview.md),
[architecture plan](milestone-06-control-api-server-architecture.md).

### P1-M7: TUI and manual workflow

Status: Planned

Goal: Let a person reach a confirmed, recoverable entry through the `nd` TUI
and the always-available manual path.

Plan: [overview](milestone-07-tui-manual-workflow-overview.md),
[architecture plan](milestone-07-tui-manual-workflow-architecture.md).

### P1-M8: Draft assistants

Status: Planned

Goal: Add hosted-default and Apple on-device two-pass drafting without bypassing
human review or the data-egress boundary.

Plan: [overview](milestone-08-draft-assistants-overview.md),
[architecture plan](milestone-08-draft-assistants-architecture.md).

Notes: Implementation waits for owner decisions on the hosted provider,
credential storage, and HTTP dependency.

### P1-M9: Local publication and recovery

Status: Planned

Goal: Preview, approve, write, verify, and recover the local Markdown output
without clobbering different content.

Plan: [overview](milestone-09-local-publication-recovery-overview.md),
[architecture plan](milestone-09-local-publication-recovery-architecture.md).

### P1-M10: Notion publication

Status: Planned

Goal: Publish local-first to a runtime-validated Notion hierarchy with safe
conflict and retry behavior.

Plan: [overview](milestone-10-notion-publication-overview.md),
[architecture plan](milestone-10-notion-publication-architecture.md).

Notes: Work against the configured root requires an owner-authorized read-only
compatibility check; live writes use an authorized disposable hierarchy.

### P1-M11: Amendments and release qualification

Status: Planned

Goal: Add safe amendment replacement and prove the complete macOS TUI MVP
against every Phase 1 exit criterion.

Plan: [overview](milestone-11-amendments-release-qualification-overview.md),
[architecture plan](milestone-11-amendments-release-qualification-architecture.md).

