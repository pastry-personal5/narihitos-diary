# P1-M9: Local publication and recovery architecture plan

Status: Done

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

The contract must expose the rendered local bytes (or losslessly equivalent
text), target path, payload digest, intended directory/file changes, plan ID,
workflow revision, and per-destination status. Mutating commands carry the
expected revision and approved plan identity. Keep diary prose and absolute
source paths out of operation records and diagnostics.

## Invariants

- Preview and write use one canonical renderer. Its UTF-8 bytes include a
  fixed YAML schema with `date` then `language` and a newline policy; the
  owner's key choice is recorded in the
  [overview](milestone-09-local-publication-recovery-overview.md#output-format-decision).
  The digest is over those bytes, not a second rendering of the entry.
- Preflight is read-only. It resolves the configured entry root and target
  without writing, records whether the target is absent, identical, or
  different, and treats an unreadable or unverifiable target as blocked.
  Path validation must reject traversal and unsafe filesystem redirection.
- Approval authorizes only the displayed plan at its workflow revision.
  Immediately before the first write, recheck source freshness and target
  state. A changed binding voids approval; no filesystem mutation follows
  until a new preview and approval.
- Only a verified local receipt allows later destinations to publish. Notion
  is represented as blocked throughout P1-M9, including when configured.
  A future Notion retry follows P1-M10's new-preview rule.

## Approach and sequence

1. Add the canonical local renderer and read-only preflight. A plan binds the
   confirmed entry, selected source digests, output settings, target identity
   and observed state, rendered payload digest, and explicit side effects.
   Return typed blocked/conflict outcomes instead of guessing at an inaccessible
   target. Add exact-byte and invalidation tests.
2. Expose the plan through the Control API and TUI Preview stage. Show the
   complete file, target path, digest, directory creation, local file creation,
   and blocked Notion status. Approval checks the plan ID/digest and current
   revision. Ensure keyboard and API publication routes both require it.
3. Revalidate the bindings, then durably record a local intent before external
   effects. Create only required parent directories, create a unique temporary
   file beside the target, write and sync it, and finalize with an atomic
   no-clobber operation. Sync the parent directory where supported. Never use
   an existence check followed by an overwriting rename. On failure, preserve
   the intent if the final write may have happened; clean up only a known
   orphaned temporary file.
4. Read the final target back and compare the exact bytes/digest before writing
   a versioned receipt. If the target is already identical, reconcile it to the
   existing intent/plan rather than adding a second entry. If it differs,
   report a local conflict with both texts for the conflict view. An
   unreadable target or failed read-back remains pending/reconciling.
5. On startup, inspect unresolved intents by diary day. Matching bytes produce
   a receipt; an absent target becomes safe to retry only after reconciliation;
   differing bytes become a conflict; unavailable inspection stays
   reconciling. Gate commands for that day while an intent is unresolved and
   leave other days usable. A cancellation request stops work at the next safe
   boundary and never claims to undo a completed write.
6. Implement only reread, discard, and save-under-another-day resolutions.
   The last carries selection and entry as unconfirmed to a new day only when
   that day has no unfinished workflow, then reruns its existing-entry check.
   Cover these rules in core, API, and TUI tests.
7. Exercise crash points around intent, temporary write, finalization,
   read-back, and receipt; lost replies and duplicate commands; filesystem
   failures and changed targets; and a synthetic REST/TUI restart journey.
   Update user-facing documentation and run the complete milestone gate.

The local file becomes the durable source for later Notion retries. This
milestone does not implement the amendment exception to no-clobber behavior.
