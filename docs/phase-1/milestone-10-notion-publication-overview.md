# P1-M10: Notion publication overview

Status: In progress

This milestone adds the second output destination without weakening P1-M9's
preview, approval, local-first, intent, receipt, and reconciliation guarantees.
The connector tolerates unrelated Notion content but fails closed whenever the
canonical diary path or same-day section is not uniquely safe.

## Goal

After the local file is verified, publish the confirmed entry under
`Root → YYYY → YYYY-MM` as a bottom-appended daily section, but only after
runtime inspection proves the destination is accessible, compatible, and
unambiguous.

## Owner decisions fixed by this plan

- A daily section starts with a native Notion `heading_2` formatted
  `YYYY-MM-DD (DDD)`. `DDD` is the diary day's fixed, locale-independent,
  uppercase English weekday (`MON` through `SUN`), including for Korean
  entries.
- The confirmed Markdown body is converted losslessly to native Notion blocks.
  No text, ordering, or supported Markdown meaning may be omitted, reordered,
  normalized away, or truncated. A construct without a lossless native mapping
  is preserved literally rather than approximated.
- Unrelated blocks and child pages are allowed beneath the root and year page.
  Only an inaccessible, duplicate, wrong-kind, or otherwise ambiguous
  canonical `YYYY` or `YYYY-MM` match blocks publication.
- Notion remains manually configured in the owner-only
  `config/global_config.yaml`. M10 adds setup diagnostics and documentation,
  not an interactive credential writer.
- Retry is bounded and cancellable. A request gets at most five total attempts;
  `Retry-After` is honored, but a wait longer than 60 seconds moves the
  destination to pending instead of holding the publication job. An ambiguous
  write is inspected before any further write.

These decisions supersede the earlier ISO-only heading and paragraph-only body
descriptions in the Phase 1 research.

## Decision and acceptance gates

Before implementation work relies on the configured root, obtain the owner's
authorization for a read-only compatibility check and record only a redacted
result. API responses may necessarily carry block text while classifying a
month page; the check must not persist, log, or display unrelated prose and may
return target-day content only through the authorized conflict view. Use a
separately authorized disposable hierarchy for live creation and append
acceptance; never experiment on the configured diary hierarchy.

Changing dependencies or build tooling remains a separate owner-approval gate.
Implementation must first inventory the workspace for a suitable HTTP client
and Markdown parser rather than assuming a new crate is permitted.

## In scope

- Strict manual configuration loading, owner-only permissions, normalized root
  page identity, pinned `2026-03-11` Notion API version, and safe setup
  diagnostics.
- Fully paginated, read-only inspection of the root, target year, target month,
  and same-day headings, with unrelated content allowed.
- One canonical Markdown-to-Notion renderer shared by preview, digest, write,
  read-back, conflict display, and reconciliation.
- Missing year/month page creation, bottom append, request-limit chunking,
  read-back, receipts, restart reconciliation, and same-day conflicts.
- A process-wide Notion request queue, bounded retry/backoff, cancellation at
  safe boundaries, and privacy-safe error mapping.
- Local-first partial success, Notion-only blocked/pending retry, Control API
  resources, and complete Preview/Publish TUI states.
- Synthetic connector, core, persistence, HTTP, and TUI coverage plus
  owner-authorized disposable-hierarchy acceptance.

## Out of scope

- Replacing or deleting a receipted Notion section; that belongs to P1-M11.
- Adopting an unreceipted same-day section without a matching unresolved intent,
  even when its visible text happens to match, or overwriting any same-day
  content.
- Reorganizing unrelated Notion content, recursively searching for a year or
  month outside the direct canonical path, or converting the hierarchy to a
  database.
- An interactive setup wizard, OAuth, token storage in SQLite, or YAML front
  matter in Notion.
- Proving compatibility from synthetic fixtures alone or writing to the
  configured diary hierarchy during acceptance.

## Completion checklist

- [ ] The owner-authorized compatibility check records whether the configured
  root and any existing target path are compatible; the evidence contains no
  token, page ID, title outside the canonical path, or diary prose.
- [x] Manual configuration validates token/root presence and root-ID syntax,
  pins the reviewed Notion API version, refuses unsafe file permissions, and
  exposes only typed setup reason codes.
- [x] Paginated resolver tests cover missing, unique, unrelated, duplicate,
  wrong-kind, trashed, inaccessible, forbidden, unreachable, and cursor/error
  states at every hierarchy level.
- [x] The renderer deterministically emits `heading_2: YYYY-MM-DD (DDD)` with
  the correct weekday across leap days, year boundaries, and both entry
  languages.
- [x] Markdown fixtures cover paragraphs, soft and hard breaks, emphasis,
  links, headings, lists, quotes, code, Unicode, empty structure, and an
  unsupported construct preserved literally; no fixture loses or truncates
  content.
- [x] Preview shows the human-readable root/year/month path, whether each page
  already exists or will be created, the exact ordered block projection,
  payload digest, and blocked/conflict reason without exposing record IDs.
- [x] Approval is bound to the entry, source snapshots, local receipt, Notion
  target identity, observed hierarchy/month state, renderer version, workflow
  revision, and canonical payload digest; any change requires revalidation.
- [x] Publication refuses to start Notion until the local receipt and local
  read-back match, then creates only missing canonical pages and re-inspects
  each parent after creation.
- [x] The section is appended at month-page bottom in API-sized chunks, later
  chunks are positioned after the last verified owned block, and every owned
  block is read back before a receipt is completed.
- [x] A same-day section without a matching receipt or unresolved intent is a
  Notion-only conflict; a matching receipt, or an exact active-intent
  reconciliation, resumes without appending duplicate content. ISO-only legacy
  headings and headings with a wrong weekday for the same date also conflict.
- [x] Timeout/lost-response tests cover uncertain year creation, month
  creation, first append, later append, read-back, and receipt persistence;
  every ambiguous write is inspected before another write.
- [x] Retry tests cover `429`/`529` with and without `Retry-After`, retryable
  idempotent `5xx`, non-retryable auth/permission/validation errors, five-total-
  attempt exhaustion, waits over 60 seconds, cancellation, and queue fairness.
- [x] Restart and multi-client tests prove one unresolved intent gates only its
  diary day, duplicate publication commands do not start a second write, and
  diagnostics remain content-free.
- [x] A blocked destination requires a new Notion-only preview and approval; a
  pending destination reuses approval only when reconciliation and re-preflight
  reproduce the same complete plan digest.
- [ ] API and TUI acceptance covers verified local/blocked Notion, later
  Notion-only approval, pending/reconciling/conflict, cancellation, restart,
  and successful partial-to-complete transition.
- [ ] Live create/append/read-back acceptance passes only in an
  owner-authorized disposable hierarchy and leaves the configured diary
  hierarchy unchanged.
- [x] README, setup example, user guide, terminal UX, architecture, decision
  index, and privacy/recovery documentation agree with the shipped behavior.
- [x] The standard Rust and Swift milestone gate passes and dated acceptance
  evidence is recorded below.

## Acceptance evidence

### Synthetic implementation evidence (2026-10-30)

- The dependency inventory found the existing pinned `ureq`, Serde, and JSON
  workspace dependencies and no Markdown parser. Owner approval added
  `pulldown-cmark` 0.13.5 and enabled the existing `ureq` Rustls transport for
  `diary-connectors`; build tooling is unchanged.
- The versioned renderer, hierarchy/same-day classifiers, bounded retry policy,
  redacted configuration, HTTPS adapter, intent-before-effect command path,
  contract-v4 resources, and TUI Notion-only flow are implemented. Synthetic
  tests cover fixed weekday boundaries, supported Markdown/Unicode, literal
  fallback, fail-closed hierarchy resolution, same-day legacy collisions,
  retry bounds, local-receipt gating through remote completion, a multi-request
  section whose later chunk uses the last read-back-verified owned block, and a
  committed append whose response is lost and then adopted through exact
  read-back without another append.
- The complete Rust gate passed on 2026-10-30: rustfmt check, Clippy for all
  targets/features with warnings denied, and all-feature tests. Swift build and
  all 13 helper tests also passed; the runner redirected only Swift/Clang module
  caches to a task-specific writable temporary directory because the execution
  sandbox denies the default user cache. The broader synthetic API/TUI matrix
  remains open.
- Scripted-service tests (2026-10-30) now cover wrong-kind, trashed,
  inaccessible, duplicate, and typed remote-error states at the year and month
  levels, unrelated siblings, lost year/month create replies, a lost read-back
  reply, unowned same-day conflicts, stale-state and cancellation voiding, and
  restart adoption without a second write. Later-chunk and receipt-persistence
  loss, queue fairness, and the API/TUI matrix remain open.
- Service and Control API tests (`apps/nd/tests/p1_m10_notion.rs`, 2026-10-30)
  prove that remote-state drift voids approval and publication before any
  write; blocked and conflict previews carry no plan and need a new approval; an
  ambiguous write gates only its diary day, rejects duplicate commands, and
  restart reconciliation adopts the section without a second write; no-effect
  and cancelled attempts stay pending and reuse the approval; a conflicting
  remote stops only Notion; and a real worker runs preview, approval, publish,
  and reload with the receipt surviving reopen. Two defects found by these tests
  were fixed: an unapproved stale preview can now be re-previewed, and
  publication revalidates the remote state in every workflow state, not only
  `partial`.
- The Rust gate (fmt, Clippy, 387 tests) and Swift gate (build, 13 tests) passed
  after these changes. Still open: later-chunk and receipt-persistence loss,
  queue fairness and the full HTTP retry matrix, the remaining API/TUI states,
  and both owner-authorized live checks.
- Failure-matrix evidence (2026-10-30): connector tests cover lost replies for
  year creation, month creation, the first append, a later chunk (committed and
  never-landed), and read-back, each inspected before any further write; the
  destination-agnostic coordinator tests in `diary-core` cover failure to
  persist a receipt, which keeps the intent unresolved and reconciles without
  rewriting. Retry tests cover `429`/`529` with and without `Retry-After`
  (including the 60-second ceiling), retryable `5xx` for idempotent reads only,
  non-retryable auth/permission/validation errors, five-total-attempt
  exhaustion, cancellation, and queue fairness. Writes are never blindly
  retried after `5xx` or a lost response.
- Final gate run (2026-10-30): rustfmt check and Clippy with warnings denied
  were clean and all 400 all-feature tests passed. The Swift helper is
  unchanged since its earlier passing build and 13 tests. Still open: TUI state
  coverage for the Notion-only flow (the TUI crate has client/worker plumbing
  but no Notion-specific state tests) and both owner-authorized live checks.
- No configured Notion hierarchy, token, private diary content, or live API was
  read. The compatibility and disposable-hierarchy acceptance items remain
  unchecked pending their separate owner authorizations.

Completion must record the redacted compatibility result,
synthetic test boundaries, disposable-hierarchy cleanup result, and dated Rust
and Swift gate results without copying Notion identifiers or content here.
