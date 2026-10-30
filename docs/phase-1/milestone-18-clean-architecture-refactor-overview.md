# P1-M18: Clean architecture refactor overview

Status: Done

This milestone refactors the Phase 1 implementation around clear ownership of
domain rules, use cases, adapters, transport, and terminal presentation. It
targets the current multi-thousand-line files whose unrelated responsibilities
make changes hard to review, without imposing a line-count or complexity limit.

## Goal

A contributor can locate and change one workflow rule, server command, storage
mapping, Notion operation, or TUI interaction in a cohesive module without
tracing an all-purpose file or duplicating policy across layers.

## Entry criteria and order

- P1-M17 is `Done`.
- P1-M10's implementation and synthetic gate are stable. Its owner-only live
  compatibility and disposable-hierarchy checks may remain open; M18 does not
  inspect the configured Notion root or perform live writes.
- Run M18 before P1-M13 so release qualification covers the refactored code.

## In scope

- Split the TUI session loop into session state, event/result reduction, command
  dispatch, and rendering, leaving `apps/nd` as the binary composition root.
- Separate Control API transport, authentication/validation, jobs/SSE, DTO
  mapping, and application use cases. Keep workflow decisions in the core or
  command layer and terminal decisions in `diary-tui`.
- Group core workflow transitions and invalidation rules by invariant while
  retaining one authoritative workflow state and revision model.
- Split SQLite schema/migrations, codecs, workflow storage, and operation log;
  split Notion projection, hierarchy, HTTP/retry, and publication/reconciliation.
- Remove dead abstractions and duplicated mapping or branching found while
  making these boundaries. Internal APIs and the Control API may change, with
  the server, TUI, fixtures, and docs updated together. The owner has approved
  breaking backward compatibility and API changes for M18.
- Move tests to the behavior they protect; add characterization coverage only
  where a behavior crossing a new boundary lacks evidence.

## Out of scope

- New product behavior, assistant providers, connectors, UI stages, or live
  Notion qualification. Do not change the decided privacy, egress, publication,
  recovery, source, or IME rules as a side effect of restructuring.
- Deleting diary data or resetting SQLite to simplify the refactor. A needed
  schema change uses a forward migration and round-trip/reopen evidence.
- New dependencies or build tools. If the implementation finds one necessary,
  obtain the separate approval required by `AGENTS.md` first.
- Cosmetic churn unrelated to a boundary being moved. No numeric file-length,
  function-length, or complexity target; split when ownership becomes clearer.

## Completion checklist

- [x] Inventory the current responsibility map and record the before/after
  module ownership, public interfaces, and tests for each slice in the
  architecture plan; identify any proposed wire or schema break before making
  it.
- [x] `apps/nd/src/main.rs` owns startup and assembly, while TUI session
  state, reduction, dispatch, and rendering have cohesive owners; TUI, client,
  and process tests cover editor recovery, IME, Sources, assistant settings,
  and reconnection.
- [x] Control API handlers only authenticate, validate, map, and dispatch;
  command/core tests prove source, assistant, publication, and amendment rules
  without HTTP. Jobs and SSE retain cancellation, revision, and reload behavior.
- [x] Workflow transition and invalidation rules have one owner and focused
  tests; SQLite reopen and migration tests preserve durable workflows,
  preferences, intents, receipts, and amendments.
- [x] Notion rendering, hierarchy classification, transport/retry, and write
  reconciliation are separate, with existing lossless-projection and
  ambiguous-write tests still passing against the same fixtures.
- [x] Any wire break has a deliberate contract-version change, strict DTO
  fixtures, simultaneous server/TUI updates, and a documented compatibility
  note. Any schema break has an append-only forward migration; no existing
  diary data is discarded.
- [x] Review remaining large production files by responsibility and record why
  any one of them still needs to hold multiple related operations. Remove
  unused public APIs and duplicate tests only after their behavior is covered.
- [x] Update affected architecture, API, contributor, and user docs; the
  release-qualification matrix still names tests that exist.
- [x] Gate passes: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`, plus `swift build` and `swift test` in
  `helpers/apple-bridge`.

## Acceptance evidence

The [ownership inventory](milestone-18-clean-architecture-refactor-architecture.md#implemented-ownership-2026-10-30)
records every moved seam and the large-file rationale. Public wire DTOs and
route paths, contract version, SQLite schema, and Notion projection did not
change, so no migration or compatibility note is needed. The internal TUI
session entry point moved to `diary-tui`, with the paragraph digest supplied by
the binary; saved assistant selection at workflow creation moved from the
HTTP route to `CommandService`.

Behavior evidence includes `result_overflow_disables_mutations_until_explicit_recovery_reads_finish`,
`no_key_is_interpreted_unless_the_ime_reads_english`,
`sources_screen_keeps_inclusion_separate_from_the_active_read_only_body`,
`start_and_writing_settings_carry_session_choices`,
`reconnect_uses_the_last_completely_decoded_event_id`,
`assistant_choice_survives_restart_and_new_workflows_inherit_session_settings`,
`empty_database_migrates_reopens_and_every_prior_schema_upgrades`, and
`ambiguous_write_gates_its_day_and_restart_adopts_without_a_second_write`.
P1-M13's evidence matrix names no relocated test module paths; its named tests
still exist. Architecture and contributor docs were updated; API and user docs
need no change because the wire contract and product behavior stayed the same.

On 2026-10-30, the final gate passed: Rust formatting check, Clippy with all
targets/features and warnings denied, all-feature Rust tests, Swift build, and
Swift tests (13 passed). `git diff --check` passed. Synthetic loopback tests
ran; no configured Notion root or private runtime directory was accessed.
