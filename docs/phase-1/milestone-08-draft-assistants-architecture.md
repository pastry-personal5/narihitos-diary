# P1-M8: Draft assistants architecture plan

Status: Planned

This plan fills the `DraftAssistant` port only after its undecided hosted
dependencies are owner-approved.

## Affected modules and interfaces

- `diary-core`: extraction, memory-review, drafting, validation, preference, and
  freshness-check use cases already bounded by `DraftAssistant`.
- `diary-connectors`: chosen hosted adapter and Swift helper client adapter.
- `helpers/apple-bridge`: versioned Foundation Models command group.
- `diary-control` and contract: extraction/draft jobs and capability state.
- `diary-tui`: Memory review and assisted-editor states.

## Approach and sequence

1. Resolve and document the provider, credential, and HTTP choices, including
   license and configuration consequences, before dependency changes.
2. Define request types constructible only from the approved data classes and
   centralize size, timeout, cancellation, and response validation.
3. Implement hosted availability and calls without logging text or exposing its
   credential through core, database, API, process arguments, or errors.
4. Add the helper's on-device command group with the same structured outcomes
   and fake all model behavior in tests.
5. Wire extraction and draft jobs through freshness checkpoints and human
   memory/entry review, retaining manual text on any failure.
6. Add ViewModels and labels, then prove both adapters through synthetic
   contract tests and one fake-backed vertical flow.

The hosted default is a preference, not a consent screen; the active path stays
visible. Neither adapter receives approval or publication capabilities.
