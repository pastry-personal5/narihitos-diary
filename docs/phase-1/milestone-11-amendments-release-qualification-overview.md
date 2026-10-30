# P1-M11: Amendments and release qualification overview

Status: Planned

This final Phase 1 milestone adds safe amendments and qualifies the complete
macOS TUI MVP against the phase exit criteria. It fixes integration defects but
does not broaden product scope.

## Goal

Prove that a person can complete, recover, retry, and amend the full local and
Notion workflow without overwriting content the app does not own.

## In scope

- Amendment workflow seeded from the current local file, hand-edit detection,
  replacement preview/approval, atomic digest-guarded local replacement, and
  versioned receipts/retention restart.
- Receipted Notion-section replacement by block IDs, read-back-before-trash,
  interrupted-replacement reconciliation, and conflict handling.
- End-to-end recovery, privacy/security, accessibility, terminal, packaging,
  setup, and exit-criteria acceptance using synthetic data and authorized
  disposable external fixtures.
- Defect fixes and documentation needed to satisfy existing Phase 1 scope.

## Out of scope

- MCP, remote access, additional connectors, iOS, or new product behavior.
- Re-selecting sources during amendment or overwriting unreceipted content.
- Live model calls or private diary prose in automated tests.

## Completion checklist

- [ ] Choosing a `Completed` day offers amendment, seeds the editor from the
  current local body, preserves original source digests, waives source freshness,
  and identifies hand edits without treating them as a conflict.
- [ ] Local amendment rechecks the seed-time digest immediately before atomic
  replacement and refuses any intervening change.
- [ ] Notion amendment locates only the receipted section, verifies its blocks,
  inserts and reads back replacement body blocks before trashing old body blocks,
  and conflicts on any ownership mismatch.
- [ ] Tests reconcile interruption at every Notion replacement boundary without
  data loss, blind replay, or deletion of unrelated blocks.
- [ ] Completed amendment creates a receipt version and restarts the seven-day
  text-retention interval.
- [ ] Synthetic end-to-end tests cover manual, hosted-fake, Apple-fake, empty-day,
  source-change, restart, blocked/pending Notion, conflict, different-day save,
  and amendment journeys through real REST and rendered TUI state.
- [ ] Security acceptance covers owner-only files, loopback/origin enforcement,
  content-free diagnostics, bounded input/jobs, and credential/data-egress
  exclusions.
- [ ] Real-terminal acceptance on the supported macOS baseline covers startup,
  restoration, multiline English/Korean input, 2-Set Command Line mapping, IME
  status, display width, and the complete manual local-output journey.
- [ ] Authorized disposable-fixture acceptance covers iCloud coordination and
  Notion create/append/amend/reconcile without using private diary content.
- [ ] README, setup, troubleshooting, privacy, and all Phase 1 planning docs
  reflect the shipped behavior; every Phase 1 exit criterion has linked evidence.
- [ ] The standard Rust and Swift gate passes from a clean checkout, and the
  results and supported toolchain versions are recorded.
