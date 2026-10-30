# P1-M9: Local publication and recovery overview

Status: Done

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
- Synthetic end-to-end acceptance through the Control API and TUI, including
  restart after an uncertain local write.

## Out of scope

- Creating or changing Notion pages and local amendment replacement.
- Overwriting different local content or publishing before exact-plan approval.
- Inspecting or writing the configured Notion hierarchy; its compatibility
  check and connector belong to P1-M10.

## Output format decision

- The owner chose YAML keys `date` and `language` on 2026-10-30. The renderer
  must fix key order, quoting, and final-newline behavior so preview, digest,
  write, and read-back use identical bytes.

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
- [x] The preview shows the exact target path, YAML front matter, body, digest,
  and intended filesystem changes before approval; exact bytes are covered by
  a fixture. The local adapter's exact-render test and the Preview DTO/TUI
  supply the evidence.
- [x] Approval is accepted only for the current entry, source snapshots,
  destination state, workflow revision, and plan digest; a changed setting,
  target, or file state forces a new preview. The synthetic local journey checks
  wrong digests and intervening file changes before approval and publication.
- [x] The adapter writes a same-directory temporary file, flushes it, and uses
  no-clobber finalization without a check-then-rename race; different existing
  content is always a conflict, and temporary files are cleaned up safely.
  Exact-output, competing-writer, and symlink tests cover the local adapter.
- [x] Same-path identical content resumes safely and read-back produces a
  durable receipt only after byte/digest verification, with no duplicate diary
  prose in the operation log or diagnostics. Connector and SQLite integration
  tests cover identical resume and one receipt after a lost response.
- [x] Tests simulate crashes before and after the external write, lost responses,
  cancellation before/after intent, duplicate commands, startup reconciliation,
  unavailable read-back, and per-day gating without blocking unrelated days.
  `local_publication` covers each of these boundaries with a temporary SQLite store.
- [x] A local conflict stops the plan and exposes only reread, discard, or save
  under another day; the carry-over rules are covered by core and TUI tests.
  The integration test proves different content blocks approval and the target
  stays intact; TUI rendering tests show both texts and the day-carry control.
- [x] With Notion unconfigured or unimplemented, the preview identifies it as
  blocked and still permits the local-only plan; no Notion write is reachable.
- [x] The TUI has no command path that bypasses the Output preview approval, and
  publication status distinguishes verified, pending, blocked, conflict, and
  reconciliation in progress after restart or cancellation. Core approval
  checks gate the publication job and the workflow DTO carries both statuses.
- [x] Synthetic real-API/TUI acceptance publishes a manual entry, restarts the
  server, shows a verified local receipt and blocked Notion status, then proves
  that a conflicting file cannot be overwritten. The loopback worker test runs
  with synthetic content and a temporary configured output root.
- [x] User-facing output/recovery documentation matches the local behavior.
  See README, the TUI user guide, and terminal UX.
- [x] The standard Rust and Swift milestone gate passes and acceptance evidence
  is recorded. On 2026-10-30, rustfmt, Clippy with denied warnings, all-feature
  Rust tests, Swift build, and 13 Swift tests passed. The loopback API/worker
  acceptance test also passed with network access enabled.

## Acceptance evidence

The local renderer, no-clobber adapter, preview/approval/publication-job API,
and Preview/Publish TUI controls are in place. The local adapter tests cover
exact bytes, identical resume, concurrent no-clobber writes, different-file
protection, and symlink refusal. Lifecycle coverage verifies that the shipped
`~/` output-root form resolves under the user's home. SQLite integration tests
exercise cancellation before commit, binding changes, duplicate publication
commands, lost replies, and restart at three write boundaries. A real loopback
API/worker test covers preview through receipt, server restart, conflict
refusal, and different-day carry-over with Notion blocked. The full Rust and
Swift milestone gate passed on 2026-10-30; SwiftPM needed access to its module
cache.
