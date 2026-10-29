# P1-M2: Software architecture overview

Status: Done

This milestone defines a reviewable software architecture for the macOS diary
application. Its product architecture is recorded separately so that it can be
read without the milestone plan.

## Goal

Specify the boundaries, workflows, and interfaces needed for a local control
server, a TUI client, and extensible input and output connectors.

## In scope

- Research the relevant architecture principles and current HTTP/MCP transport
  contracts.
- Define dependency direction, module responsibilities, connector ports, and
  workflow state and failure semantics.
- Specify the local control API, TUI MVVM role, event streaming, and the future
  MCP integration boundary.
- Preserve the owner-approved source, composition, output, and privacy rules.
- Review the terminal UX documents ([terms](../ux-terms.md),
  [information architecture](../ux-information-architecture.md),
  [behavior](../ux-gui.md)) against the architecture and record the owner
  decisions that review produced.

## Out of scope

- Production Rust code or dependencies; those belong to subsequent implementation.
- Inspecting or changing the configured Notion root page.
- Publishing an entry or modifying private source data.

## Completion checklist

- [x] The architecture document states the dependency rule and assigns each
  concern to a module with a testable boundary.
- [x] Input and output connector contracts support additional implementations
  without changing diary policy or the TUI.
- [x] The control server contract covers REST resources, streaming, concurrency,
  authorization, process ownership, and ports beginning at 8771.
- [x] The TUI's MVVM responsibilities, the workflow pipeline, and a future MCP
  Streamable HTTP endpoint are specified without bypassing review or consent.
- [x] Source freshness, draft recovery, local-first publication, Notion
  reconciliation, and the unresolved root-page preflight are preserved.
- [x] The architecture has current primary-source references, is indexed, and
  contains no phase or milestone references.
- [x] The architecture states workflow states with voiding rules, the process
  lifecycle and startup order, the authorization trust boundary, write-ahead
  operation intents, the Swift helper contract, and the files on disk.
- [x] The UX documents agree with the architecture and the owner decisions of
  2026-10-28: vocabulary, step states, connection and error states, conflict and
  blocked-destination behavior, and the Korean-IME input constraint.
- [x] The review's owner decisions are recorded in [AGENTS.md](../../AGENTS.md)
  and the [changelog](changelog.md).
- [x] The standard gate is replaced by the exemption below, with its alternative
  evidence recorded.
- [x] The second review round's owner decisions are recorded: keybindings and
  Command Line mapping, empty days, amendments, retry approval, the Swift helper
  in the gate, and the P1-M1 documents returned to Active with stale rows
  corrected.
- [x] The hosted-default composition and data-egress rule replaces the
  on-device-only rule across the decided list, product behavior, UX documents,
  and research notes, and the hosted provider is recorded as Undecided.
- [x] The architecture records agent-client context cost for Codex and Claude
  Code, per-day reconciliation, entry-buffer versioning, and the REST resources
  the UX requires.

## Acceptance evidence

The checked design items are evidenced by the
[architecture proposal](../planned-mvp-architecture.md), its
[index entry](../README.md), the revised UX documents, and the owner direction
in [AGENTS.md](../../AGENTS.md). A local script resolved every relative Markdown
link and heading anchor in all 28 tracked Markdown files and found no problems.
The terminology check found no phase or milestone references in the
architecture proposal, and `git diff --check` passed. After the second review
round the same script reported only three template placeholders inside code
blocks, which are not links. The Notion block API facts (`after_block` position,
trash-on-delete) and the Claude Code and Codex MCP behavior were checked against
current documentation on 2026-10-28.

At milestone completion on 2026-10-28, the alternative gate was rerun against
the expanded Phase 1 documentation set: relative links and heading anchors
resolved, the architecture proposal remained free of phase and milestone
references, and `git diff --check` passed. The phase plan and changelog record
the completion transition; no production manifest or Apple helper exists yet.

Carried forward: the hosted provider, its credential storage, and the HTTP
dependency stay Undecided in [AGENTS.md](../../AGENTS.md) and must be settled
before the hosted adapter is built. The 33 KB architecture proposal and the
219-line `AGENTS.md` are long for agent context; splitting either is a
restructuring that needs owner approval.

## Exit-gate exemption

P1-M2 is a documentation-only milestone, and the Cargo manifest it would need
for the standard commands (`cargo fmt --all -- --check`, `cargo clippy
--all-targets --all-features -- -D warnings`, and `cargo test --all-features`)
arrives in P1-M3. The owner approved an exemption on 2026-10-28. The alternative
completion evidence is the acceptance evidence above: link and anchor
validation, the terminology check, and `git diff --check`.

This exemption applies only to P1-M2. It does not change the Phase 1 exit
criteria or pre-authorize an exemption for any later milestone.
