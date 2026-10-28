# P1-M10: Notion publication architecture plan

Status: Planned

This plan implements the Notion `OutputConnector` behind the same preview,
approval, intent, receipt, and reconciliation rules used for local output.

## Affected modules and interfaces

- `diary-connectors`: Notion client, hierarchy resolver, block renderer,
  pagination, bounded retry, read-back, and reconciliation.
- `diary-core`: independent destination status and retry-plan policy.
- `diary-control` and contract: Notion preflight/preview and destination status.
- `diary-tui`: hierarchy preview, blocked/pending/conflict, and retry actions.
- `apps/nd`: owner-only Notion configuration loading.

## Approach and sequence

1. With owner authorization, run a read-only compatibility check against the
   configured root and keep the observed structure as acceptance evidence, not
   as a hard-coded assumption.
2. Implement a paginated, fail-closed hierarchy resolver and synthetic contract
   fixtures before any write path.
3. Render only the confirmed body into bounded Notion blocks and preview the
   exact existing path, proposed pages, heading, text, and final-state digest.
4. After approval and local verification, record an intent, revalidate the path,
   create missing pages in order, append at month-page bottom, and read back.
5. On timeout or lost response, inspect the relevant parent/month page before
   deciding resume, conflict, or retry; honor `Retry-After` within fixed bounds.
6. Validate real writes only in an owner-authorized disposable hierarchy, then
   exercise partial success and retry through the API and TUI.

Notion remains independently blockable: its preflight or write failure never
rolls back a verified local file or claims transactionality across destinations.
