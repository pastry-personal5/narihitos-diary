# P1-M10: Notion publication overview

Status: Planned

This milestone adds the second output destination with fail-closed hierarchy
inspection, conflict detection, and read-before-retry recovery. It does not
assume that the configured root matches synthetic fixtures.

## Goal

Publish the confirmed body under `Root → YYYY → YYYY-MM → YYYY-MM-DD` only when
runtime preflight proves the existing hierarchy is compatible and unambiguous.

## Decision gate

Before implementation work that relies on the real configured root, obtain the
owner's authorization for a read-only compatibility check and record its
result. Use a separately authorized disposable hierarchy for live write
acceptance; never experiment on the configured diary hierarchy.

## In scope

- Owner-only Notion configuration, token redaction, paginated root/year/month
  inspection, permissions, and hierarchy compatibility.
- Exact preview of existing/created pages and bottom-appended daily section.
- Page creation, Markdown-body mapping, API-limit splitting, rate limiting,
  append, read-back, receipts, conflicts, and ambiguous-write reconciliation.
- Local-first partial success, blocked and pending retry rules, and TUI states.

## Out of scope

- Amendment replacement of a receipted Notion section.
- Writing to an ambiguous/incompatible hierarchy or overwriting same-day content.
- Storing the Notion token in SQLite or sending YAML front matter to Notion.

## Completion checklist

- [ ] The owner-authorized read-only check records whether the configured root
  and any existing year/month path are compatible; incompatible or ambiguous
  results fail closed and keep local output available.
- [ ] Configuration permissions are enforced, and token, root IDs, diary text,
  and request payloads do not appear in logs or client-visible errors.
- [ ] Contract tests cover paginated missing, existing, duplicate, ambiguous,
  incompatible, forbidden, and unreachable root/year/month states.
- [ ] The exact preview distinguishes read-only inspection from pages/blocks to
  be created and contains the heading plus complete body with no YAML.
- [ ] Publication creates only missing unambiguous pages, appends the ISO-date
  `heading_2` section at month-page bottom, splits to API limits, and reads back
  every written block before receipt completion.
- [ ] Same-day conflicting content blocks only Notion; matching receipted content
  resumes, and uncertain creation/append is inspected before any retry.
- [ ] Rate-limit and transient-failure tests cover `Retry-After`, bounded retry,
  local-written/remote-pending, remote-blocked, cancellation, and reconciliation.
- [ ] Blocked retry requires a new Notion-only preview and approval; pending
  retry reuses approval only when the rechecked plan digest is unchanged.
- [ ] Live write acceptance passes only in an owner-authorized disposable
  hierarchy and leaves the configured diary hierarchy unchanged.
- [ ] User-facing Notion setup, privacy, partial-success, and retry documentation
  is complete.
- [ ] The standard Rust and Swift milestone gate passes and acceptance evidence
  is recorded.
