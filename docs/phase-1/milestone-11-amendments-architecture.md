# P1-M11: Amendments architecture plan

Status: Done

This plan adds the architecture's one intentional replacement path with
digest-guarded, interruption-safe writes at both destinations.

## Affected modules and interfaces

- `diary-core`: persisted amendment seed (file digest, receipt version),
  confirmation, replacement plan, approval binding, receipt version, retention
  restart, and conflict policy. `Workflow::seed_amendment` today stores only the
  text; it gains the seed digests.
- `diary-connectors`: digest-guarded local replace, receipted Notion section
  replacement and reconciliation, and persistence of replacement intent.
- `diary-control` and `diary-control-contract`: `POST
  /v1/workflows/{id}/amendments` per [architecture.md](../architecture.md), the
  replacement status, the capabilities list, and a contract version bump with
  forward migration. The `/v1/amendments` entry in the disabled-routes list and
  the contract test that rejects any route containing "amend" are updated.
- `diary-tui`: amendment offer, changed-since-publication banner, replacement
  preview, Conflict view (reread or abandon), partial and completion states.

## Approach and sequence

1. Confirm what M10 receipts carry. If the heading block ID, body block IDs and
   digests, or local file digest is missing, add them with a forward migration
   before anything else.
2. Extend the amendment seed to persist the seed-time local-file digest and
   receipt version. Seed the editor from the file body only, keep original
   source digests, skip source reads and freshness checks, and detect hand edits
   (body or front matter) by comparing against the receipt.
3. Build the replacement plan: per destination, current and replacement
   content, with approval bound to the seed-time and current destination
   digests. Any later edit or digest change voids the approval.
4. Implement atomic local replacement: temp file in the destination directory,
   flush, recheck the digest immediately before rename, rename. A mismatch
   refuses and leaves the file untouched.
5. Implement Notion replacement: verify heading and body blocks against the
   receipt, insert new blocks after the heading (`after_block`, at most 100 per
   request), read back, then trash the old body blocks. Persist intent with the
   old block IDs so reconciliation trashes whichever set is stale. Duplicates,
   never losses, remain after interruption.
6. Apply local-first partial semantics: the local file is replaced first; a
   Notion failure or uncertain write leaves the workflow `Partial` and Notion
   pending, retried alone under the usual unchanged-plan approval rule.
7. Add conflict handling that offers reread and abandon-amendment only.
   Abandon restores no files; it discards the amendment revision and returns to
   the `Completed` state with its receipts.
8. Add the API route, contract bump, and TUI states.
9. Add deterministic crash-boundary tests with fake ports and a fake Notion
   server that models block ordering and trash, driving every interruption
   point and asserting no loss, no blind replay, and no unrelated deletion.
10. Update affected docs and run the standard gate.

## Verification boundaries

Use invented, synthetic content only. Core tests cover seed digests, hand-edit
detection, approval voiding, retention restart with an injected clock, and the
conflict choices. Connector tests cover the digest guard race, the atomic
rename, and Notion ownership mismatches. HTTP and TUI tests exercise the route,
version negotiation, and rendered states. No live Notion, iCloud, model, or
human run is part of this milestone.

This milestone does not authorize new dependencies, broad restructuring,
private-data access, or work from later phases.
