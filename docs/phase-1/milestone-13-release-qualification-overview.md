# P1-M13: Release qualification overview

Status: Planned

This final Phase 1 milestone proves the assembled macOS TUI MVP against every
Phase 1 exit criterion using automated tests only. It fixes scope-conforming
integration defects and does not broaden product scope. The number 13 follows
P1-M12, which ran earlier; P1-M13 runs after P1-M11.

## Goal

Link every Phase 1 exit criterion to named automated evidence produced by a
clean-checkout gate run, with known limits documented instead of gated on a
human run.

## Entry criteria

- P1-M11 is `Done`; P1-M7 through P1-M10 are `Done`.

## In scope

- An evidence matrix in this overview mapping each Phase 1 exit criterion to the
  named automated test or tests that prove it.
- Synthetic end-to-end tests over real REST and rendered TUI state for the
  manual, hosted-fake, Apple-fake, empty-day, source-change, restart,
  blocked/pending Notion, conflict, different-day save, and amendment journeys.
- Pty-driven TUI tests: startup, restoration, multiline English and Korean
  input, display width, 2-Set mapping, and IME status through a fake
  input-source notifier.
- A fake Notion HTTP server modelling pagination, block ordering, and trash, and
  a temp-directory iCloud simulation including cloud-only placeholders.
- Security tests: owner-only file modes, loopback and origin enforcement,
  content-free diagnostics, bounded input and jobs, and a scan proving no
  credential, audio byte, or attachment path reaches composition input,
  output, or logs.
- Packaging, setup, and troubleshooting documentation; a clean-checkout gate
  run recording results and toolchain versions.
- Defect fixes needed to satisfy existing Phase 1 scope.

## Out of scope

- Any checklist item that requires a human to run a terminal, an input method,
  or a live Notion or iCloud environment.
- New product behavior, new dependencies, MCP, remote access, additional
  connectors, or iOS.
- Live model calls or private diary prose in tests.

## Known limits (documented, not gating)

- Real macOS input-source behavior (Korean IME) is exercised only through the
  fake notifier. A human spot check is welcome but does not gate completion.
- Live Notion and live iCloud behavior is covered by fakes and by the P1-M10
  disposable-hierarchy evidence where it exists. The docs state this plainly.

## Completion checklist

- [ ] The evidence matrix lists every Phase 1 exit criterion with a named
  automated test, and each named test exists and passes.
- [ ] End-to-end journey tests cover all journeys listed under "In scope".
- [ ] Pty-driven terminal tests cover the listed input, width, and IME-status
  behavior.
- [ ] Fake Notion and temp-directory iCloud tests cover create, append, amend,
  reconcile, and cloud-only acquisition.
- [ ] Security tests cover every item listed under "In scope".
- [ ] README, setup, troubleshooting, privacy, known limits, and all Phase 1
  planning docs reflect shipped behavior.
- [ ] The standard Rust and Swift gate passes from a clean checkout, and the
  results and supported toolchain versions are recorded here.
