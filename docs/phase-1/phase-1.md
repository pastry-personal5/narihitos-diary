# Phase 1: macOS TUI MVP

Status: Active

Goal: Make remembering a day feel effortless through a macOS terminal user
interface that turns existing Markdown source notes into a finished diary entry and
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
  drafting and paragraph review through a selectable assistant-backend
  registry: Codex through `codex app-server` by default, Apple on-device, and
  future approved adapters without a workflow-layout change.
- Produce one finished diary entry as a Markdown file on the local filesystem.
- Amend a completed entry, replacing only content the app itself wrote.
- Use the configured Notion root page with `YYYY` year and `YYYY-MM` month
  subpages; append the confirmed Markdown body as a daily section at the
  bottom of its month page.

## Exit criteria

- A user can start the server with `nd serve` and interact with it through the
  `nd` TUI on macOS.
- A user can choose among the available assistant backends or write manually;
  assistant drafts and paragraph reviews are never accepted automatically, and
  a blank entry cannot be confirmed. The backend list supports two or more
  registered implementations without provider-specific workflow stages.
- A completed entry can be amended without overwriting content the app did not
  write.
- The application can read Markdown source notes from a mounted iCloud
  directory.
- Before composition, the TUI shows the source-note selection and any directory
  versus header date disagreement; the user can include or exclude discovered
  notes even when their metadata date does not match the diary day.
- The finished diary entry is composed from exactly the confirmed source-note
  selection.
- Ayumi entries remain eligible with or without audio attachments, and neither
  audio bytes nor attachment paths appear in composition input or output.
- A completed run writes the finished diary entry as a local Markdown file.
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

Status: Done

Goal: Define the Phase 1 software architecture.

Plan: [overview](milestone-02-software-architecture-overview.md),
[architecture plan](milestone-02-software-architecture-architecture.md),
[product architecture](../architecture.md).

Notes: Completed under the documentation-only gate exemption recorded in the
overview; link, anchor, terminology, and diff validation supplied the
alternative acceptance evidence. P1-M3 remains planned pending its required
owner-reviewed dependency and toolchain approval packet.

### P1-M3: Scaffolding and tools

Status: Done

Goal: Establish the Rust project scaffold and development tools.

Plan: [overview](milestone-03-scaffolding-overview.md),
[architecture plan](milestone-03-scaffolding-architecture.md).

Notes: The P1-M2 entry gate is satisfied. The owner approved P1-M3's
dependency, Cargo-feature, build-tool, Swift-toolchain, and scaffold decision
packet on 2026-10-28 before manifest or tooling changes began. The six-package
Rust workspace, one `nd` binary, Swift protocol helper, safe configuration
baseline, and contributor workflow are implemented; the full Rust and Swift
gate passed on 2026-10-28.

### P1-M4: Core workflow and persistence

Status: Done

Goal: Implement the diary state machine and recoverable private state behind
inward-facing ports.

Plan: [overview](milestone-04-core-workflow-persistence-overview.md),
[architecture plan](milestone-04-core-workflow-persistence-architecture.md).

Notes: The standard-library-only core state machine, independent editor
versions, exact publication bindings, fake recovery orchestration, and
forward-migrated SQLite workflow/operation stores are implemented. Synthetic
tests cover concurrency, reopen, reconciliation, and retention; the full Rust
and Swift gate passed on 2026-10-28.

### P1-M5: Source ingestion and Apple bridge

Status: Done

Goal: Discover and read confirmed Ayumi source snapshots, including selected-only
iCloud acquisition through the Swift helper.

Plan: [overview](milestone-05-source-ingestion-apple-bridge-overview.md),
[architecture plan](milestone-05-source-ingestion-apple-bridge-architecture.md).

Notes: The core source ports, candidate availability persistence, bounded Ayumi
catalog and parser, selected-only helper client, Swift iCloud commands, private
byte handoff, and freshness checkpoint integration are implemented with
synthetic fakes. The full Rust and Swift gate passed on 2026-10-29.

### P1-M6: Control API Server

Status: Done

Goal: Expose the core through the secure, loopback-only `nd serve` REST, event,
job, and lifecycle contract.

Plan: [overview](milestone-06-control-api-server-overview.md),
[architecture plan](milestone-06-control-api-server-architecture.md).

Notes: The owner-approved Tokio/Axum server, initial strict `/v1` DTOs, command/CAS layer,
bounded jobs and SSE, private lifecycle/runtime advertisement, loopback
authorization, and content-free rotating diagnostics are implemented. Synthetic
command, real-socket, restart, permission, retention, and privacy tests pass;
the full Rust and Swift gate passed on 2026-10-29. A post-completion review then
hardened prepared-source review, job/SSE recovery, request diagnostics,
filesystem safety, and retention; its focused regressions and the complete gate
passed again on 2026-10-29. P1-M12 revised the unshipped client contract
before P1-M7 without changing the REST namespace.

### P1-M12: Consistency and contract readiness

Status: Done

Goal: Reconcile every active document and implemented lower-layer contract with
the canonical UX vocabulary and revised assistant architecture before P1-M7.

Plan: [overview](milestone-12-consistency-contracts-overview.md),
[architecture plan](milestone-12-consistency-contracts-architecture.md).

Notes: This owner-directed milestone executes after P1-M6 and before P1-M7
despite its number. It may break the unshipped Control API contract, but it must
forward-migrate durable workflow and publication state. The documentation and
codebase consistency review, contract version 2 migration, M7 lower-layer
readiness work, and complete Rust/Swift gate passed on 2026-10-29.

### P1-M7: TUI and manual workflow

Status: Done

Goal: Let a person reach a confirmed, recoverable entry through the `nd`
terminal client and the always-available Draft editor.

Plan: [overview](milestone-07-tui-manual-workflow-overview.md),
[architecture plan](milestone-07-tui-manual-workflow-architecture.md).

Notes: The automated manual slice, restart/recovery acceptance, documentation,
and full Rust/Swift gate passed on 2026-10-29. The owner then confirmed the
primary Wave Terminal qualification and secondary Apple Terminal regression
run recorded in the overview, completing P1-M7.

### P1-M8: Draft assistants

Status: Done

Goal: Add an extensible assistant-backend registry with hosted-default and Apple
on-device two-pass drafting and paragraph review, without bypassing human review
or the data-egress boundary.

Plan: [overview](milestone-08-draft-assistants-overview.md),
[architecture plan](milestone-08-draft-assistants-architecture.md).

Notes: The original assistant slice passed its gate on 2026-10-29. P1-M8 was
reopened for the owner-approved Sources-stage refine/edit/delete subflow.
Contract v3/schema v5, strict multi-profile Codex
configuration, confined per-job `codex app-server`, Apple Foundation Models,
durable bound artifacts, assistant jobs/resources, and the Compose TUI vertical
slice passed the full Rust and Swift milestone gate. Synthetic tests use no live
model, account, private diary, or iCloud tree; detailed evidence is in the
overview.

### P1-M9: Local publication and recovery

Status: Done

Goal: Preview, approve, write, verify, and recover the local Markdown output
without clobbering different content.

Plan: [overview](milestone-09-local-publication-recovery-overview.md),
[architecture plan](milestone-09-local-publication-recovery-architecture.md).

Notes: Canonical local preview/approval, no-clobber file creation, receipts,
restart reconciliation, conflict day carry-over, and TUI controls are complete.
Synthetic adapter, SQLite, and real loopback worker acceptance passed; the
standard Rust and Swift gate passed on 2026-10-30. Notion remains blocked for
P1-M10.

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

