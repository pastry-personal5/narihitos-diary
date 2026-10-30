# P1-M13: Release qualification architecture plan

Status: Planned

This plan tests the assembled product at its real process, transport, and
terminal boundaries using only automated, synthetic fixtures.

## Affected modules and interfaces

- Integration tests in `apps/nd/tests/` named `p1_m13_<theme>.rs`, following
  the existing `p1_mN_*.rs` files, which already start the real server and
  assemble the Notion, iCloud, and local paths. Reuse their helpers; do not
  fork a second fixture layer.
- Terminal tests in `crates/diary-tui`, rendering through ratatui's in-memory
  test backend and feeding scripted crossterm events and a fake
  `InputLanguageNotifications` implementation (`ime.rs`).
- The fake Notion server and iCloud simulation stay with the connector test
  support in `diary-connectors`; extend them for rate limits, ambiguous
  writes, and cloud-only placeholders rather than adding new fakes.
- A fake `codex app-server` executable (a small test binary or script that
  speaks the JSON-RPC subset over stdio and records its argv and requests) for
  the hosted journey and the sandbox-argument assertions.
- Packaging, setup, troubleshooting docs, and the evidence matrix. Troubleshooting
  goes in `user-guide-tui.md` or a new indexed doc, not `README.md`.

## Terminal test approach (decision needed)

No pty or terminal-emulator crate is a dependency today, and adding one
conflicts with "no new dependencies" in `AGENTS.md`. Default plan: in-memory
backend plus scripted events, which covers layout, width, multiline editing,
2-Set mapping, and IME gating deterministically. If the owner approves a
dev-dependency for a real pty harness, add it as a separate, reviewed change
first; otherwise the "Known limits" entry stands.

## Approach and sequence

1. Build the evidence matrix from the `phase-1.md` exit criteria and the
   supplemental rows, listing the existing test, or a gap, for each. Search
   existing `p1_m*` tests and unit tests before declaring a gap. Commit the
   matrix before writing tests so gaps are reviewable.
2. Add the matrix check so named tests cannot silently disappear.
3. Close each gap with the smallest synthetic test, in this order: end-to-end
   journeys, fake-boundary behavior (Notion, iCloud, Codex), terminal
   behavior, then security checks.
4. Fix only scope-conforming defects found by those tests. Each fix gets its
   regression test and a one-line changelog entry; a defect that needs new
   behavior or a decided-item change stops for an owner decision.
5. Update documentation and record known limits.
6. Run the complete gate from a clean checkout (a fresh clone with its own
   `target/`): `cargo fmt --all -- --check`,
   `cargo clippy --all-targets --all-features -- -D warnings`,
   `cargo test --all-features`, `swift build`, `swift test`. Record dated
   results and toolchain versions in the overview, and link the evidence from
   the phase plan before proposing Phase 1 completion.

## Test rules

- Deterministic: no wall-clock sleeps or real network. Inject clocks and use
  the fake servers' ephemeral loopback ports. A test that needs a timeout
  asserts on the injected clock.
- Synthetic data only. Diary text, notes, and credentials are generated
  fixtures with recognizable canary strings.
- Security scan method: seed canary credential, audio-byte, and attachment-path
  values into config and source fixtures, run each journey, then assert the
  canaries are absent from the composition input captured by the fake Codex and
  Apple adapters, from the assistant output, and from every file under the
  test's `control-api` log directory. Diagnostics assertions follow the
  `ndlog-v1` privacy rule in `AGENTS.md`.
- Bounded input and jobs: assert the 512 KiB entry bound, request size limits,
  and job cancellation/cleanup (the fake Codex process is killed and not left
  running) with named boundary cases.
- Isolation: every test uses a temp state, config, and iCloud root. None reads
  the Application Support runtime directory, the control token, or
  `diagnostic-tag-key`.
- Gate cost: keep the added tests within the existing `fast-test` and
  `rust-test` structure; mark nothing `#[ignore]`.

Release qualification does not authorize new dependencies, broad restructuring,
private-data access, human-run gates, or work from later phases.
