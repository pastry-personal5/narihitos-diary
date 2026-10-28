# P1-M9: Local publication and recovery overview

Status: Planned

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
