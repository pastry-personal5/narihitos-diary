# P1-M9: Local publication and recovery overview

Status: Active

This milestone completes a safe local-only diary journey: preview, approve,
write, verify, recover, and resolve local conflicts. Notion may appear as
blocked, but no Notion write occurs yet.

## Goal

Produce the approved Markdown entry locally without clobbering unrelated
content and with enough intent/receipt evidence to recover after interruption.

## In scope

- Read-only local preflight, exact Markdown/YAML rendering, output plan digest,
  preview, approval, operation intent, atomic no-clobber write, read-back, and
  versioned receipt.
- Local conflicts, identical-content resume, idempotency, cancellation boundary,
  startup reconciliation, and per-destination publication status.
- TUI Output preview, approval, Publication status, and Conflict view.
- Notion shown as unavailable/blocked without preventing an approved local write.

## Out of scope

- Creating or changing Notion pages and local amendment replacement.
- Overwriting different local content or publishing before exact-plan approval.

## Completion checklist

- [x] Sources-stage text inspection keeps the Resources Sidebar active marker
  on the source whose body is displayed, including across asynchronous preview
  completion and canonical workflow reloads; a regression test covers reordered
  candidates.
- [x] An SSE-driven workflow-list or source reload no longer moves the focused
  Resources/Main row of the stage being shown (focus is bounded only by the
  current stage's per-frame clamp); a regression test covers a reload outside
  Sources.
- [x] Job polls are serialized and each reply is routed by the polled job ID,
  so a recovery read can no longer finish a still-running assistant job; a
  `job-not-found` reply releases the pending job. Covered by poll-order and
  forgotten-job tests.
- [x] Editing a source body uses its own editor bound to the source opened
  for editing, so neither a canonical reload nor a finishing preview can write
  the diary entry over a source note (or a source body over the entry).
- [x] An unresolved Draft conflict blocks autosave until the person keeps the
  local or the server text; a regression test covers the blocked snapshot.
- [x] A healthy SSE stream's deliberate 45-second reconnect with an event
  cursor no longer reports a transport loss (which paused actions and reloaded
  canonical state); an unchanged canonical reload no longer resets the Draft
  editor's cursor. Covered by event-worker rollover tests.
- [ ] The preview shows the exact target path, YAML front matter, body, digest,
  and intended filesystem changes before approval.
- [ ] Approval is accepted only for the current entry, source snapshots,
  destination state, workflow revision, and plan digest.
- [ ] The adapter writes a same-directory temporary file, flushes it, and uses
  no-clobber finalization; different existing content is always a conflict.
- [ ] Same-path identical content resumes safely and read-back produces a
  durable receipt with no duplicate diary prose in the operation log.
- [ ] Tests simulate crashes before and after the external write, lost responses,
  cancellation, duplicate commands, startup reconciliation, and per-day gating.
- [ ] A local conflict stops the plan and exposes only reread, discard, or save
  under another day; the carry-over rules are covered by core and TUI tests.
- [ ] With Notion unconfigured or unimplemented, the preview identifies it as
  blocked and still permits the local-only plan.
- [ ] The TUI has no command path that bypasses the Output preview approval, and
  publication status distinguishes verified, pending, blocked, and conflict.
- [ ] User-facing output/recovery documentation matches the local behavior.
- [ ] The standard Rust and Swift milestone gate passes and acceptance evidence
  is recorded.
