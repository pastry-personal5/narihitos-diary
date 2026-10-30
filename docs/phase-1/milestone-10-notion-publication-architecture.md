# P1-M10: Notion publication architecture plan

Status: In progress

This plan adds a Notion adapter to the existing publication pipeline. It does
not create a second approval or recovery model: P1-M9's destination plans,
intents, receipts, statuses, and per-day reconciliation remain authoritative.

## Affected modules and interfaces

- `diary-core`: canonical remote-plan bindings, independent destination
  execution scope, blocked/pending retry policy, and receipt validation needed
  by the later amendment milestone.
- `diary-connectors`: deterministic Markdown-to-Notion rendering, Notion HTTP
  transport, hierarchy resolver, pagination, request queue, retry policy,
  append/read-back, and ambiguous-write reconciliation.
- `diary-control` and `diary-control-contract`: typed Notion preflight/preview,
  Notion-only approval and retry, safe reason codes, destination progress, and
  size-bounded conflict content available only to authorized local clients.
- `diary-tui`: exact hierarchy/block preview and distinct blocked, pending,
  reconciling, conflict, cancelled, and verified presentations.
- `apps/nd`: strict owner-only manual configuration, secret-bearing client
  composition, and setup capability reporting. Tokens never cross into domain,
  persistence, DTO, diagnostic, or TUI types.
- SQLite operation intents and receipts: renderer/API versions, canonical
  payload digest, target page identities, owned block IDs, per-block digests,
  and enough phase progress to reconcile without storing prose.

Any required schema or Control API contract revision is versioned and migrated
in the same change. Existing clients must fail clearly on an incompatible
contract rather than silently dropping Notion state.

## Canonical rendering contract

One pure renderer accepts the confirmed Markdown body and diary day and returns
an ordered, versioned block plan. Preview renders this plan; publication sends
it; read-back and reconciliation compare against it. The payload digest covers
the renderer version and a canonical serialization of block kinds, nesting,
text, annotations, links, language/code metadata, and order. It is not a digest
of incidental HTTP JSON field ordering.

The first block is always a `heading_2` whose plain text is
`YYYY-MM-DD (DDD)`. The weekday is derived from the workflow's calendar day,
not the current clock or entry language, and uses fixed English uppercase
abbreviations. The body is parsed as Markdown and mapped to native Notion
blocks only where the mapping preserves content and meaning. The renderer must:

- preserve every Unicode scalar, significant break, inline annotation, link
  destination, block order, and supported nesting;
- split rich text, child arrays, and requests below the pinned API limits on
  valid Unicode boundaries without changing the logical block plan;
- preserve an unsupported construct as literal Markdown in a paragraph/code
  representation instead of dropping it or guessing;
- reject an entry that cannot be represented within documented page/workspace
  limits with a typed blocked reason before approval; and
- expose the exact ordered native-block projection in Preview so approval is
  meaningful even though the local destination remains Markdown.

Renderer fixtures, not the Notion response shape, define the stable product
mapping. Adding or changing a mapping later bumps the renderer version and
invalidates old unexecuted plans.

## Hierarchy compatibility and preflight

All child-list reads consume every page and treat cursors as opaque. For diary
day `D`, the resolver inspects only direct children along
`configured root → D.YYYY → D.YYYY-MM`; it never recursively guesses.

Unrelated blocks and differently titled child pages are compatible. At each
canonical level:

- zero exact, non-trashed child-page title matches means the page is missing;
- one exact match means it is reusable after its identity and accessibility
  are verified;
- multiple exact matches, a canonical-title object of the wrong kind, an
  incomplete page listing, or an unverifiable/trashed match fails closed.

The month scan paginates all top-level blocks and classifies same-day
`heading_2` blocks. The canonical text is the exact `YYYY-MM-DD (DDD)` value.
An ISO-only `YYYY-MM-DD` heading or `YYYY-MM-DD (...)` with a different weekday
is also a same-day collision, but never an adoptable canonical match. Zero
same-day matches permits append. One canonical match is resumable when a stored
receipt identifies every owned block and read-back matches, or when a
still-active intent for this exact plan proves a unique matching full/prefix
write during reconciliation. Without either ownership record it is a conflict,
even if the visible text matches; a noncanonical or multiple match is always a
conflict. Section display for a conflict runs from the matching heading to the
next peer-or-higher heading or page end, with size-bounded transport rules
already used by the Control API.

Preflight returns labels and proposed effects to clients but retains page/block
IDs server-side. A plan binds the normalized root identity, resolved or
proposed canonical titles, observed child-list/month-state digests, renderer
and pinned API versions, body block digest, local receipt digest, and workflow
revision. Immediately before each first effect, revalidation must reproduce
the binding or void approval.

## Write and reconciliation protocol

Notion executes only after the local destination is verified and its current
bytes still match its receipt.

1. Record the Notion intent before the first remote effect. The intent tracks
   the current phase (`year`, `month`, `section`, or `read-back`) and returned
   stable IDs/digests, but no diary prose.
2. Re-inspect the parent, then create a missing year page. Whether the response
   succeeds, is lost, or is ambiguous, paginate the parent again and require
   exactly one canonical match before proceeding. Repeat for the month page.
3. Re-scan the month immediately before append. Append the heading and as many
   initial body blocks as fit in one request at explicit end position. Chunk
   the remainder within API limits and place each chunk immediately after the
   last read-back-verified owned block; do not blindly append a later chunk.
4. Persist progress between chunks. After every lost or ambiguous response,
   inspect the month and compare the expected owned prefix. A unique matching
   prefix resumes at the first missing block; divergence or an unowned same-day
   heading becomes conflict; inability to decide stays reconciling.
5. Retrieve every returned block and compare its stable ID, kind, order,
   content, and digest. Only then replace the intent with a versioned receipt.

Cancellation is observed before an effect and between requests/waits. Once an
effect may have happened, cancellation leaves the destination reconciling; it
never claims rollback. P1-M10 never deletes blocks or pages.

## Retry and error policy

All requests share one server-owned queue so concurrent workflows cannot each
consume the connection budget. One request has at most five total attempts and
a 30-second transport timeout. The queue is a first-in-first-out ticket line
entered once per request, so a job issuing back-to-back requests rejoins the
back of the line and cannot starve a waiting job. A request keeps its turn
through its own retry waits, which is how a rate limit pauses the queue. Waits
run on the job's worker thread, poll for cancellation, and a cancelled waiter
leaves the line without blocking those behind it.

| Outcome | Policy |
|---|---|
| `429` or `529` with `Retry-After ≤ 60s` | Pause the queue for at least the supplied delay, use the deterministic backoff below on repeats, then retry if attempts remain. |
| `429` or `529` with `Retry-After > 60s` | Persist pending state and finish the job; a later retry starts with reconciliation/preflight. |
| `429` or `529` without valid `Retry-After` | Use deterministic exponential backoff (1, 2, 4, 8 seconds) capped at 60 seconds, within the same attempt bound. |
| `500`, `502`, `503`, or `504` on a read | Retry within the same bound using the same backoff. |
| Timeout/transport error or `5xx` on create/append | Treat as ambiguous; inspect the relevant parent/month before deciding whether any write is safe. |
| `401` | Blocked: credential invalid or expired. |
| `403` | Blocked: permission/capability or workspace-limit reason, classified by stable code. |
| `404` | Blocked/unverifiable: inaccessible or missing configured/resolved object; never reveal which case through raw server text. |
| `400`, `409`, unsupported response, malformed pagination | Typed blocked/conflict as appropriate; no blind retry. |

Parse stable status/error codes and documented structured fields; never persist
or expose arbitrary remote messages. Ignore unknown response fields, cap all
response bodies, and keep tokens, request bodies, page/block IDs, titles outside
the canonical path, and diary content out of diagnostics.

Every request sends `Notion-Version: 2026-03-11`. A future version change is a
reviewed connector change with updated fixtures and acceptance evidence, not a
runtime opt-in or silently inherited default.

## Destination state and retry rules

- `blocked`: no approved executable Notion plan exists. Local publication may
  finish. Recovery requires a new Notion-only preview and approval.
- `pending`: an approved plan exists but no remote effect is known to have
  occurred, or bounded retry deferred it. Reconcile and re-preflight; reuse
  approval only if the complete plan digest is unchanged.
- `reconciling`: an effect may have occurred or an intent lacks a receipt.
  Gate commands for that diary day until inspection classifies it.
- `conflict`: hierarchy or same-day content cannot be safely adopted. Stop only
  Notion and expose the existing/planned projections with the established
  reread, discard, or save-under-another-day resolutions.
- `verified`: receipt IDs and all block digests match read-back. Duplicate
  commands and restart reconciliation return this state without another write.

A Notion-only retry reads the entry from the verified local file, strips the
fixed local front matter through the canonical local parser, and requires the
file/entry digest to match the local receipt. A mismatch returns to entry
reconfirmation; it is never treated as a remote retry payload.

## Implementation sequence

1. Record the owner-authorized read-only compatibility result and inventory
   existing dependencies. Obtain separate approval before adding a crate or
   changing build tooling.
2. Add renderer fixtures and canonical block-plan/digest types. Freeze the
   mapping and renderer version before implementing HTTP writes.
3. Add the bounded transport, redaction tests, opaque-cursor pagination, request
   queue, retry clock abstraction, and hierarchy/same-day resolver against a
   scripted fake service.
4. Extend persisted intents/receipts and core destination transitions, then
   cover blocked/pending/reconciling/conflict/verified restart paths and
   Notion-only approvals.
5. Expose redacted preflight and exact block projection through the versioned
   Control API; implement TUI Preview, Publish, retry, cancellation, and
   conflict states without a bypass around approval.
6. Implement create/append/read-back in intent-before-effect order. Inject lost
   replies at every remote boundary and prove prefix reconciliation before
   enabling a real transport.
7. Run synthetic REST/TUI restart acceptance, then live acceptance only in the
   owner-authorized disposable hierarchy. Record redacted evidence and cleanup.
8. Update user/setup/privacy/recovery documentation and run the complete Rust
   and Swift milestone gate.

The configured diary hierarchy is read-only during compatibility acceptance.
Amendment replacement remains P1-M11 work, but M10 receipts must retain the
heading and body block identities/digests that P1-M11 will need.
