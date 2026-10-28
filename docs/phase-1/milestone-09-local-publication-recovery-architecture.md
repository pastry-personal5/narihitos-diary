# P1-M9: Local publication and recovery architecture plan

Status: Planned

This plan activates the publication pipeline with the local output connector
as the first durable destination and keeps Notion independently blocked.

## Affected modules and interfaces

- `diary-core`: preflight, output-plan, approval, publication, conflict
  resolution, and retry use cases.
- `diary-connectors`: local Markdown `OutputConnector`.
- `diary-control` and contract: output-plan, approvals, publications, conflict
  resolutions, receipts, and publication jobs.
- `diary-tui`: Output preview, Publication status, and Conflict ViewModels.
- SQLite operation intents and receipts from P1-M4.

## Approach and sequence

1. Render exact local payloads and compute plan digests from immutable confirmed
   entry and source revisions plus freshly inspected destination state.
2. Present the plan through the API/TUI and bind an approve-capable action to
   that exact digest; any relevant change voids it.
3. Record an intent before writing, then use same-directory temporary creation,
   flush, and no-clobber finalization followed by read-back verification.
4. Map identical content to resume and different content to conflict; implement
   only the three approved resolutions.
5. Reconcile unresolved intents on startup without blocking unrelated days and
   distinguish cancellation from rollback.
6. Exercise crash points and duplicate client commands, then document the
   complete local-only journey while leaving Notion blocked.

The local file becomes the durable source for later Notion retries. This
milestone does not implement the amendment exception to no-clobber behavior.
