# P1-M13: Release qualification overview

Status: Planned

This final Phase 1 milestone proves the assembled macOS TUI MVP against every
Phase 1 exit criterion and every shipped Phase 1 scope item using automated
tests only. It fixes scope-conforming integration defects and does not broaden
product scope. The number 13 follows P1-M12, which ran earlier; P1-M13 runs
after P1-M11, P1-M14, P1-M15, and P1-M16 so it qualifies the final layout.

## Goal

Link every Phase 1 exit criterion to named automated evidence produced by a
clean-checkout gate run, with known limits documented instead of gated on a
human run.

## Entry criteria

- P1-M7, P1-M8, P1-M9, P1-M11, P1-M14, P1-M15, P1-M16, and P1-M17 are `Done`.
- P1-M10 is `Done`, or its only open items are the owner-authorized live
  checks (configured-root compatibility and disposable-hierarchy acceptance).
  Those are recorded under "Known limits" and never gate this milestone.
  Resolve which applies, with the owner, before starting.
- The "Undecided" Notion root-structure item in `AGENTS.md` is not built on:
  tests use only a fake Notion server, and nothing here touches the configured
  root.

## In scope

- An evidence matrix in this overview. It has one row per Phase 1 exit
  criterion plus one row per shipped scope item that no exit criterion covers
  (see "Supplemental rows"). Each row names the automated test or tests that
  prove it, or a gap that this milestone closes.
- A matrix check: a test (or `make` target run by the gate) that reads the
  matrix, lists tests with `cargo test --all-features -- --list` and
  `swift test list`, and fails if any named test does not exist. Passing
  results are verified by the gate itself, not by this check.
- Synthetic end-to-end tests over the real REST server (loopback process, not
  in-process mocks) and rendered TUI state for these journeys: manual,
  hosted-fake (a fake `codex app-server` process), Apple-fake, empty-day,
  source-change, restart mid-publication, blocked and pending Notion, conflict,
  different-day save, amendment, and iCloud mirror (success, failure isolation,
  retry).
- Terminal tests of startup, restoration, multiline English and Korean input,
  display width, 2-Set mapping, the IME-gated source-list shortcuts, and IME
  status through a fake input-source notifier. Real-binary tests run `nd`
  under a pseudo-terminal (`portable-pty`, `vt100`; owner-approved dev-only
  dependencies); fine-grained cases render through ratatui's in-memory backend
  with scripted crossterm events. See the architecture plan.
- A fake Notion HTTP server modelling pagination, block ordering, rate limits
  and `Retry-After`, ambiguous writes, and trash, and a temp-directory iCloud
  simulation including cloud-only placeholders.
- Security tests: owner-only file modes, loopback and origin enforcement,
  token handling, content-free diagnostics, bounded input and jobs, the
  restricted Codex per-turn sandbox arguments, and a scan proving no
  credential, audio byte, or attachment path reaches composition input,
  output, or logs.
- Packaging, setup, and troubleshooting documentation; a clean-checkout gate
  run recording results and toolchain versions.
- Defect fixes needed to satisfy existing Phase 1 scope.

## Out of scope

- Any checklist item that requires a human to run a terminal, an input method,
  or a live Notion or iCloud environment.
- New product behavior, new dependencies beyond the approved `portable-pty` and
  `vt100` dev-dependencies, MCP, remote access, additional
  connectors, or iOS.
- Live model calls or private diary prose in tests. Tests never read the
  Application Support directory, its control token, or `diagnostic-tag-key`;
  every server a test starts uses a temporary state directory.

## Evidence matrix

Filled in during step 1 of the architecture plan. `Status` is `covered`
(test exists and passes), `gap` (test to write), or `limit` (documented
limit, no test possible). Replace `TBD` with the test path and name.

| # | Exit criterion (from [phase-1.md](phase-1.md)) | Evidence | Status |
|---|---|---|---|
| 1 | Start with `nd serve`, interact through the `nd` TUI | TBD | gap |
| 2 | Choose among assistant backends or write manually; drafts and reviews never auto-accepted; blank entry cannot be confirmed; two or more backends without provider-specific stages | TBD | gap |
| 3 | Amend a completed entry without overwriting content the app did not write | TBD | gap |
| 4 | Read Markdown source notes from a mounted iCloud directory | TBD | gap |
| 5 | Source selection and date disagreement shown before composition; include or exclude regardless of metadata date | TBD | gap |
| 6 | Entry composed from exactly the confirmed selection | TBD | gap |
| 7 | Ayumi eligible with or without audio; no audio bytes or attachment paths in composition input or output | TBD | gap |
| 8 | Completed run writes a local Markdown file | TBD | gap |
| 9 | Missing Notion year/month pages created; body published under the diary-day heading at the bottom of the month page | TBD | gap |

### Supplemental rows

Shipped Phase 1 scope that no exit criterion above states.

| # | Scope item | Evidence | Status |
|---|---|---|---|
| S1 | iCloud mirror is byte-identical, isolated from local and Notion, and absent when unconfigured (P1-M14) | TBD | gap |
| S2 | Codex runs only through `codex app-server` with the restricted per-turn sandbox | TBD | gap |
| S3 | Apple on-device adapter covers all four actions for en-US and ko-KR | TBD | gap |
| S4 | Utility Sidebar and flat Sources menu behave per [ux-tui.md](../ux-tui.md) (P1-M15, P1-M16) | TBD | gap |
| S5 | Real Korean IME behavior | fake notifier only | limit |
| S6 | Live Notion and live iCloud behavior | fakes only | limit |

## Known limits (documented, not gating)

- Real macOS input-source behavior (Korean IME) is exercised only through the
  fake notifier. A human spot check is welcome but does not gate completion.
- Live Notion and live iCloud behavior is covered by fakes and by the P1-M10
  disposable-hierarchy evidence where it exists. The docs state this plainly.
- Pty tests use an emulated screen (`vt100`), not a real terminal application,
  so terminal-emulator-specific rendering is not covered.

## Completion checklist

- [ ] Entry criteria hold, and the P1-M10 resolution is recorded in the phase
  changelog.
- [ ] The evidence matrix has no `TBD` and no `gap`; every `covered` row names
  an existing passing test, and the matrix check passes.
- [ ] End-to-end journey tests cover every journey listed under "In scope".
- [ ] Terminal tests cover the listed input, width, shortcut, and IME-status
  behavior.
- [ ] Fake Notion and temp-directory iCloud tests cover create, append, amend,
  reconcile, retry limits, and cloud-only acquisition.
- [ ] Security tests cover every item listed under "In scope".
- [ ] README, setup, troubleshooting, privacy, known limits, and all Phase 1
  planning docs reflect shipped behavior; `docs/README.md` indexes any new doc.
- [ ] The standard Rust and Swift gate passes from a clean checkout, and the
  dated results and supported toolchain versions (`rustc`, `cargo`, `swift`,
  macOS) are recorded here.
- [ ] No dependency beyond `portable-pty` and `vt100` (dev-only), no
  state-directory read, and no live service call was introduced.

## Gate record

Filled in at completion: date, commit, `rustc`/`cargo`/`swift`/macOS versions,
and the result of each gate command.
