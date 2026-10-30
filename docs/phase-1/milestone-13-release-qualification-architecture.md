# P1-M13: Release qualification architecture plan

Status: Planned

This plan tests the assembled product at its real process, transport, and
terminal boundaries using only automated, synthetic fixtures.

## Affected modules and interfaces

- Test harnesses in `diary-control`, `diary-tui`, and `apps/nd` for end-to-end,
  pty, and security coverage; the fake Notion server and iCloud simulation live
  with the existing connector test support.
- Packaging, setup, user documentation, and the evidence matrix.

## Approach and sequence

1. Build the evidence matrix from `phase-1.md` exit criteria, listing the
   existing test, or a gap, for each.
2. Close each gap with the smallest synthetic test: end-to-end journeys, pty
   terminal behavior, fake Notion and iCloud boundaries, and security checks.
3. Fix only scope-conforming defects found by those tests.
4. Update documentation and record known limits.
5. Run the complete gate from a clean checkout, record dated results and
   toolchain versions, and link the evidence from the phase plan before
   proposing Phase 1 completion.

Release qualification does not authorize new dependencies, broad restructuring,
private-data access, human-run gates, or work from later phases.
