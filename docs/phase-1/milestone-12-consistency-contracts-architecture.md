# P1-M12: Consistency and contract readiness architecture plan

Status: Done

This plan revises the pre-client lower-layer contracts once, before P1-M7, and
then makes the active documentation describe the implemented result. It does
not select or implement an assistant provider.

## Affected modules and interfaces

- Active repository documentation: canonical UX terms, current architecture,
  milestone ownership, and explicit historical supersession.
- `diary-core`: assistant backend identity/selection, composition path,
  assistant-visible request and stale material, source-note naming metadata,
  and the explicit no-notes state transition.
- `diary-connectors`: Ayumi source-note names and a forward SQLite migration for
  the revised workflow, preference, source-candidate, and assistant field
  names.
- `diary-control-contract`: a breaking pre-client contract version with
  collection-shaped backend descriptors and optional selected backend IDs.
- `diary-control`: command validation, DTO mapping, capability advertisement,
  safe source detail, and a client-side runtime attachment loader in the
  lifecycle boundary.
- `apps/nd`: contract fixtures and assembly/process evidence only. P1-M12 does
  not start the terminal client.

## Canonical contract model

Manual editing is always available after source confirmation and is not an
assistant backend. `CompositionPath` therefore has only `hosted` and
`apple-on-device`. An optional `AssistantSelection` binds a stable
`AssistantBackendId` to one of those paths. The workflow persists either both
parts or neither; it never stores a path with no backend identity.

Preferences remember `last_language` and an optional
`last_assistant_backend_id`. They do not remember a path independently because
the registered backend descriptor owns that fact. P1-M12 has no production
assistant registry, so no backend is selected and every non-null backend ID
received through the M7-era API fails closed. P1-M8 later resolves a remembered
ID against its runtime registry and selects the hosted default only when the
chosen provider is implemented and available.

The revised wire shape is:

| Representation | P1-M12 contract |
|---|---|
| Capabilities | General authorization capabilities, `manual_workflow_available`, `assistant_backends[]`, and disabled later routes. Each future descriptor has stable ID, display label, composition path, availability/reason, supported languages, and supported assistant actions. The array is empty before P1-M8. |
| Preferences | Optional last language and optional last assistant backend ID. |
| Workflow | Optional selected assistant backend ID and optional derived composition path; both are absent in the P1-M7 manual-only slice. |
| Writing settings | Language, style, and optional assistant backend ID. A null ID selects manual editing without making manual a backend. |
| Setup check | Safe readiness facts remain separate from assistant capability descriptors; no credential, private path, or diary content crosses the API. |

Because no production client exists, increment the contract version and update
strict fixtures in place instead of retaining the obsolete v1 DTO shape. The
REST namespace remains `/v1`; `contract_version` is the native client
compatibility discriminator. Shared workflow DTO decoding also rejects a
partial or malformed backend-ID/composition-path pair instead of relying on
each future client to remember that cross-field invariant.

## Source-note identity boundary

`SourceRef` remains the connector-neutral command identity. A
`SourceCandidate` additionally carries a bounded `source_note_name` intended only
for the local UI. Ayumi sets it to the already-sanitized relative Markdown path
used inside its configured directory. Other connectors must supply a safe,
nonempty local label rather than expose an absolute path or vendor object. Core
validation rejects absolute, traversal-like, empty-component, oversized, and
control-character names before they can be persisted or returned to a client.

The server continues to derive an opaque `source_id` for commands. Candidate
and detail DTOs carry both that ID and `source_note_name`; preparation lookup stays
bound to the opaque ID. The source-note name is permitted only in local source
representations. It is absent from diagnostics, SSE, jobs, assistant request
types, and hosted-data egress types.

## Explicit no-notes transition

The dedicated continue-without-notes command accepts either `Started` or
`Discovered`, requires the current workflow revision, and advances directly to
`SourcesConfirmed` with an empty selection digest and the explicit-empty flag.
It does not synthesize discovery or candidates. The ordinary confirmation
command still requires a nonempty prepared selection, and a nonempty selection
cannot be confirmed from `Started`.

This lets the manual workflow continue when `source:read` is unavailable while
preserving the human confirmation and state-machine evidence required by the
product decision.

## Runtime attachment boundary

Add one read-only loader beside runtime publication. It receives the already
known runtime directory and expected UID; it never calls configuration loading,
creates a directory/file, takes the server lock, or changes permissions.

The loader:

1. lstat-checks the runtime directory as an owner-only, non-symlink directory,
   opens it, and verifies descriptor/path identity;
2. lstat-checks and opens `control-api.json`, requiring an owner-only regular
   file with one hard link and the frozen size bound;
3. compares opened-file and path device/inode metadata before and after the
   bounded read;
4. strictly decodes and validates runtime version, numeric loopback host/port,
   random ID/token grammar, and the exact supported contract version; and
5. returns a token-bearing attachment type that intentionally omits `Debug`
   and exposes only the fields the typed client needs.

Missing state, incompatible contract, and unsafe or malformed state are
distinct so P1-M7 can render `not running` or `version mismatch` without
treating a suspicious file as a normal absence.

## Persistence migration

Advance the schema in one forward migration. Add source-note names, optional
assistant backend identity/path fields, and assistant-named stale/draft fields;
remove the obsolete independent composition-path preference and manual-path
workflow representation. Existing source IDs seed source-note names because the
only current connector stores a bounded relative Ayumi path there. Existing
legacy independent-path preferences become no selected backend: no stable
backend ID existed, so inventing one would violate the revised contract.

The migration preserves all unrelated rows and child tables. Tests create and
upgrade every prior schema version, reopen full workflows, and verify receipts
and unresolved operation intents remain intact. Current-schema validation must
reject partial backend/path pairs and malformed source-note names.

## Documentation review method

Inventory every non-archived Markdown document. Review current product,
architecture, process, and milestone docs directly. For research docs, keep
historical observations and proper nouns, but repair broken links and add
clear supersession where a former conclusion conflicts with a current owner
decision. Apply these distinctions consistently:

- source note versus finished entry;
- active resource versus included source;
- assistant backend ID versus composition path;
- Draft editor/manual editing versus assistant actions;
- confirm versus approve versus publish; and
- canonical stage/chrome names versus server workflow states and code modules.

Use bounded repository searches after edits for old manual-path and generic
model-state identifiers, noncanonical stage/chrome aliases, stale contract
versions, broken local links, and required title/status/summary structure.

## Implementation and verification sequence

1. Add and link the M12 plans, mark P1-M12 active ahead of P1-M7, and record the
   owner-directed ordering and compatibility policy.
2. Review and update all active documentation, with the product architecture,
   UX terms, API document, M6-M8 plans, and user-facing journey as the primary
   cross-check set.
3. Revise core source and assistant-neutral types, workflow invariants, generic
   assistant terminology, and direct explicit no-notes behavior with focused
   unit tests.
4. Add the SQLite migration and Ayumi display-name mapping; prove every prior
   schema upgrade and full record recovery before changing transport mapping.
5. Revise the wire DTOs and contract version, then update command and server
   mappings, strict fixtures, capability/setup responses, and real-loopback
   tests. Do not add assistant action routes.
6. Add the read-only runtime attachment loader and its filesystem-race,
   permissions, malformed-input, size, and non-`Debug` evidence.
7. Review all code for remaining vocabulary or contract drift, update the M7
   entry gate to consume M12 rather than repair M6, and run focused checks.
8. Delegate the complete Rust/Swift milestone gate to the test runner, record
   evidence, and mark M12 `Done` only after every checklist item is supported.
