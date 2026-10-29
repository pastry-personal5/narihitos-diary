# Source refine, edit, and delete — design

Status: Draft

This spec adds AI-assisted refinement, direct editing, and hard deletion of
Ayumi source notes (with cascading deletion of their linked audio) to the
existing Sources stage, as new scope inside the active P1-M8 milestone.

## Context

The Sources stage (`docs/product-behavior.md`) already lets a person discover
Ayumi notes, inspect a note's readable text read-only, and include/exclude it
from a diary day's composition. It never mutates or deletes the underlying
Ayumi files. This spec extends that stage — no new workflow stage is added —
so a person can, per inspected source:

1. Ask an assistant backend to refine the note's body into a cleaner source
   snippet.
2. Approve or reject the suggested refinement.
3. Edit the note's body directly.
4. Delete the note, which also deletes the audio file(s) its body links to.

## Owner decisions carried into this spec

These were confirmed in brainstorming and are binding for this feature, each
overriding or narrowing a previously decided boundary as noted:

- **Hard delete.** Deleting a source in this stage deletes the real Ayumi
  `.md` file (and its linked audio) from the mounted iCloud directory, not a
  soft in-app exclusion. This is a deliberate, owner-approved exception to
  AGENTS.md's general "ask before deleting user data" rule — approval is
  recorded here, not re-asked per call.
- **Write-back on edit/approve.** Approving a refinement, or saving a direct
  edit, overwrites the real Ayumi `.md` file. `nd` does not keep a
  silently-diverging app-side copy of a source's text.
- **Scope: folded into P1-M8**, not a separate milestone. This spec's
  checklist items are added to the M8 overview's completion checklist
  alongside its existing contract/registry/adapter work.
- **No new workflow stage.** The Sources stage gains these actions in place;
  the canonical six-stage list (`Start, Sources, Compose, Review, Preview,
  Publish`) is unchanged.
- **Refine goal (corrected from an earlier draft of this spec):** grammar/
  spelling cleanup, and correcting words or sentences that are artifacts of
  transcription error and would not plausibly appear in a human diary entry.
  Fixing a transcription error can legitimately change the surface meaning of
  the affected span, because the error itself already distorted that
  meaning — so refinement is **not** constrained to "never change meaning."
  It **is** constrained to:
  - Only touch spans that are clearly transcription/grammar artifacts
    (nonsensical words/phrases, homophone slips, broken sentence structure) —
    not stylistic rewrites of sentences that already read as coherent,
    meaningful diary prose.
  - When correcting one, infer the most plausible intended word or phrase
    from surrounding context, without adding new information, events, or
    details beyond what that context already implies.
  - Leave everything else — the person's own voice, phrasing, and anything
    that is simply true-but-unusual — untouched.
- **Audio deletion boundary, narrowed.** This reverses two currently-written
  statements, as a deliberate amendment rather than an error:
  - `docs/research/markdown-input-format.md`: "Phase 1 ignores audio
    attachments. It does not play, copy, upload, decode, transcribe,
    **delete**, or use audio for composition." The "delete" clause no longer
    holds for this feature's cascading delete; the rest still holds —
    deletion only ever acts on a file *path*, never audio bytes.
  - `docs/product-behavior.md`: "Audio and attachment paths do not enter
    composition or output" remains true (deletion is not composition or
    output), but the implicit "audio is never acted on" reading is narrowed
    to "audio is never read, played, decoded, transcribed, or sent to any
    backend." Deleting an audio file by its already-recognized path is now
    the one exception, and only as a direct consequence of deleting its
    source note.

## Approach

A fourth per-source assistant action, modeled on M8's existing paragraph-
review pattern (one durable suggestion, accept/reject with staleness
bindings) — not a reuse of the extraction or draft actions, and not a
client-only feature that bypasses the Control API's job lifecycle.

**Rejected alternative:** client-side-only refinement (TUI sends the raw body
straight to a backend with a hardcoded prompt, no server-side job or
contract). Rejected because it bypasses the job lifecycle every other
assistant action already goes through — cancellation, the one-active-job-
per-workflow rule, durable results, SSE progress — creating an inconsistent,
unrecoverable path inconsistent with the rest of M8.

**Rejected alternative:** reuse the extraction or draft action's contract for
refinement. Rejected because refinement's input/output shape (source text →
source text) differs from extraction (source → memory) and draft (memory →
entry), and overloading an existing action would make the already-resolved
M8 bounds/problem-codes tables ambiguous about which action a request
belongs to.

## Components

- **Core (`diary-core`):**
  - A per-source editable body, distinct from the immutable discovery
    snapshot used for change detection (directory/header date, content
    digest). The editable body is what refine/edit act on; the discovery
    snapshot still drives staleness detection against the on-disk file.
  - A refinement-suggestion record: original text, suggested text, the
    content-digest binding it was generated against, and a resolution
    (pending/accepted/rejected).
  - A `delete_source` operation that: resolves the note's already-recognized
    audio link(s) from its body, validates each resolved path stays inside
    that note's own directory (no traversal), returns the exact set of paths
    to delete, then removes the source from candidates and the confirmed
    selection, and marks dependent artifacts (memories, draft, suggestions)
    stale per the existing M8 per-artifact invalidation matrix — no new
    invalidation rule is needed.
  - Reuses the existing one-active-job-per-workflow rule: refine requests,
    approve/reject, direct edit, and delete are all blocked while any
    assistant job is active for the workflow, same as the other three
    actions.
- **Connectors (`diary-connectors`):** write operations on the Ayumi
  adapter — overwrite a note's body text (approved refinement or direct
  edit), and delete a note file plus its linked audio files — each bounded
  to the note's own directory with the same path-validation discipline as
  the existing read-side adapter.
- **Control API (`diary-control`):** new routes alongside M8's others:
  - `POST /v1/workflows/{id}/sources/{source_id}/refine-requests` — starts a
    job; returns a job reference resolved at `/v1/jobs/{id}`.
  - `GET /v1/workflows/{id}/sources/{source_id}/refine-suggestion` — the
    durable current suggestion, if any.
  - `PUT /v1/workflows/{id}/sources/{source_id}/refine-suggestion/resolution`
    — accept or reject; body names the resolution and the content-digest
    binding it must still match (same pattern as paragraph-suggestion
    resolution).
  - `PUT /v1/workflows/{id}/sources/{source_id}/body` — direct edit; body
    carries the new text and the digest it was edited from (same optimistic-
    concurrency shape as the Draft editor's versions); a mismatch is a typed
    conflict.
  - `DELETE /v1/workflows/{id}/sources/{source_id}` — hard delete; requires
    an explicit `confirm: true` in the request body so no client can delete
    by accident.
  - New problem codes, following the existing short/snake_case/resource-
    scoped convention: `source_refine_already_active` (reuses the one-
    active-job rule's existing conflict shape), `source_binding_stale`,
    `source_suggestion_already_resolved`, `source_delete_not_confirmed`,
    `source_delete_partial_failure` (names which step — audio or note file —
    failed).
  - New bounds, sized the same way as M8's other per-action bounds:
    `MAX_SOURCE_REFINE_INPUT_BYTES` / `MAX_SOURCE_REFINE_OUTPUT_BYTES` (one
    source body each way; reuse the existing per-note size limit already
    enforced on read), reusing `MAX_REFLECTION_RANGES`-style range bounds if
    the refine result highlights changed spans.
- **TUI (`diary-tui`):** in the Sources stage's existing per-source
  inspection view, add actions: **Refine** (start job, show original vs.
  suggested text), **Approve/Reject** suggestion, **Edit** (open the source
  body in the same editor component the Draft stage already uses), **Delete**
  (a confirmation prompt that names the file and any audio paths it will
  remove, then calls delete with `confirm: true`).

## Data flow & error handling

- **Refine:** TUI calls the refine-requests route with the source's current
  content-digest → job runs against the workflow's selected backend with a
  dedicated system prompt implementing the corrected refine-goal constraints
  above → durable suggestion resource holds original vs. suggested text.
  **Approve** writes the suggested text back to the real Ayumi file,
  binding-checked against the same digest; a changed file since the
  suggestion was generated fails closed with `source_binding_stale` rather
  than silently overwriting. **Reject** discards the suggestion; the file is
  untouched.
- **Direct edit:** the body route takes new text plus the digest it was
  edited from → writes back to the real Ayumi file on match, else
  `source_binding_stale` so the person can reload and retry.
- **Delete:** the delete route, given `confirm: true` → core resolves and
  validates the note's audio link(s), deletes the audio file(s) first, then
  the note file, then removes the source from candidates/selection and
  stales dependents. A failure partway (e.g. audio deleted, note delete
  fails) returns `source_delete_partial_failure` naming which step failed —
  core does not report success it cannot back up.
- **Safety invariant carried through from existing decisions:** no audio
  bytes are ever read, decoded, or sent anywhere; delete only ever needs the
  audio file's path.

## Testing

- Core: a fake writable-source port exercises the refine-suggestion
  lifecycle (stale binding, accept, reject), direct-edit conflict, and
  delete (including audio-link resolution and the partial-failure case)
  without touching a real filesystem.
- Connectors: real-filesystem tests for the Ayumi write/delete adapter cover
  path-traversal rejection, multi-audio-link notes, and notes with zero
  audio links.
- Control API: synthetic command tests for the five new routes, reusing the
  existing job/binding/conflict test patterns from paragraph-review and
  draft.
- TUI: scripted client tests for the new Sources-stage actions and the
  destructive-delete confirmation path.
- No automated test deletes or rewrites a real Ayumi/iCloud file outside a
  disposable fixture directory.

## Documentation impact

- `AGENTS.md`: add a one-line pointer recording the hard-delete and
  write-back exceptions (this spec is the owning detail).
- `docs/phase-1/milestone-08-draft-assistants-overview.md`: add this spec's
  scope to the M8 completion checklist, decision/approval gate, and in/out of
  scope sections; add a sixth required decision packet or fold into the
  existing contract/migration packet (implementation-plan decision).
- `docs/product-behavior.md`: update the Sources-stage step and the
  audio/attachment boundary bullet per "Audio deletion boundary, narrowed"
  above.
- `docs/research/markdown-input-format.md`: amend the "does not... delete...
  audio" sentence per the same narrowing.
- `docs/phase-1/changelog.md`: record this as an owner decision entry once
  approved.

## Open items for the implementation plan (not blocking spec approval)

- Exact DTO field names/shapes for the five new routes, following the
  existing `diary-control-contract` naming conventions.
- Whether refine's result includes highlighted changed-span ranges (reusing
  a `MAX_REFLECTION_RANGES`-shaped bound) or is plain before/after text only.
- Whether this needs its own contract-version bump (v3 → v4) or lands inside
  the still-unreleased v3 since M8 overall hasn't shipped yet.
