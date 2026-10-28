# P1-M11: Amendments and release qualification architecture plan

Status: Planned

This plan adds the architecture's one intentional replacement path and then
tests the assembled product at its real process, transport, terminal, and
external-adapter boundaries.

## Affected modules and interfaces

- `diary-core`: amendment start, seed, confirmation, replacement plan, approval,
  receipt version, retention restart, and conflict policy.
- `diary-connectors`: digest-guarded local replace and receipted Notion section
  replacement/reconciliation.
- `diary-control` and contract: amendment resource and replacement status.
- `diary-tui`: amendment offer, changed-since-publication banner, replacement
  preview, Conflict view, and completion state.
- Packaging, setup, user documentation, and acceptance evidence.

## Approach and sequence

1. Seed amendments from the current local file while retaining receipt source
   digests and bypassing source reads and freshness checks.
2. Build a replacement plan showing current and proposed content per destination
   and bind approval to the seed-time and current destination digests.
3. Implement atomic local replacement only after an immediate digest guard.
4. Implement Notion replacement as insert-after-heading, read-back, then trash
   old receipted paragraphs; persist enough intent data to reconcile duplicates
   after interruption.
5. Add API/TUI amendment states and run deterministic crash-boundary tests.
6. Exercise every Phase 1 exit criterion across real process boundaries with
   synthetic data, real terminal checks, fake models, and only explicitly
   authorized disposable external fixtures.
7. Fix only scope-conforming defects, rerun the complete gate, and link evidence
   from the phase plan before proposing Phase 1 completion.

Release qualification does not authorize new dependencies, broad restructuring,
private-data access, or work from later phases.
