# Documentation index

Status: Active

This index lists the design and process docs, one line each. Read only the doc you need. The rules for writing docs are in [AGENTS.md](../AGENTS.md) under "Documentation".

| Doc | What it covers |
|-----|----------------|
| [roadmap.md](roadmap.md) | Product phases, their goals and status, and the sole authority for the active phase. |
| [phase-1/phase-1.md](phase-1/phase-1.md) | Phase 1 goal, scope, exit criteria, and milestone summary for the macOS TUI MVP. |
| [phase-1/milestone-01-research-overview.md](phase-1/milestone-01-research-overview.md) | Completed P1-M1 scope, completion checklist, and exit-gate exemption. |
| [phase-1/milestone-01-research-architecture.md](phase-1/milestone-01-research-architecture.md) | P1-M1 evidence rules, research workstreams, sequencing, and decision-record format. |
| [phase-1/milestone-01-research-summary.md](phase-1/milestone-01-research-summary.md) | P1-M1 owner decision-to-evidence map and remaining runtime checks. |
| [phase-1/milestone-02-software-architecture-overview.md](phase-1/milestone-02-software-architecture-overview.md) | P1-M2 goal, scope, completion checklist, and acceptance evidence. |
| [phase-1/milestone-02-software-architecture-architecture.md](phase-1/milestone-02-software-architecture-architecture.md) | P1-M2 affected boundaries and architecture planning sequence. |
| [phase-1/milestone-03-scaffolding-overview.md](phase-1/milestone-03-scaffolding-overview.md) | P1-M3 entry criteria, approval record, verifiable scaffold checklist, acceptance evidence, and gate. |
| [phase-1/milestone-03-scaffolding-architecture.md](phase-1/milestone-03-scaffolding-architecture.md) | P1-M3 workspace dependency map, command shells, Swift protocol seam, safe configuration, and implementation sequence. |
| [phase-1/milestone-04-core-workflow-persistence-overview.md](phase-1/milestone-04-core-workflow-persistence-overview.md) | P1-M4 entry gates, scoped core and SQLite work, verifiable checklist, and acceptance evidence. |
| [phase-1/milestone-04-core-workflow-persistence-architecture.md](phase-1/milestone-04-core-workflow-persistence-architecture.md) | P1-M4 transitions, concurrent revisions, durable records, fake publication, reconciliation, and retention sequence. |
| [phase-1/milestone-05-source-ingestion-apple-bridge-overview.md](phase-1/milestone-05-source-ingestion-apple-bridge-overview.md) | P1-M5 entry gates, bounded discovery, parsing, selected-only iCloud reads, freshness checklist, and acceptance evidence. |
| [phase-1/milestone-05-source-ingestion-apple-bridge-architecture.md](phase-1/milestone-05-source-ingestion-apple-bridge-architecture.md) | P1-M5 source identity, privacy filtering, Swift byte handoff, failure mapping, freshness, and implementation sequence. |
| [phase-1/milestone-06-control-api-server-overview.md](phase-1/milestone-06-control-api-server-overview.md) | P1-M6 entry gates, manual-flow server scope, security and recovery checklist, and acceptance evidence. |
| [phase-1/milestone-06-control-api-server-architecture.md](phase-1/milestone-06-control-api-server-architecture.md) | P1-M6 route and DTO boundary, conditional writes, loopback lifecycle, bounded jobs, and SSE sequence. |
| [phase-1/milestone-07-tui-manual-workflow-overview.md](phase-1/milestone-07-tui-manual-workflow-overview.md) | P1-M7 terminal client, source selection, manual editor, and IME checklist. |
| [phase-1/milestone-07-tui-manual-workflow-architecture.md](phase-1/milestone-07-tui-manual-workflow-architecture.md) | P1-M7 API boundary, client recovery rules, MVVM, editor, Command Line, and IME sequence. |
| [phase-1/milestone-08-draft-assistants-overview.md](phase-1/milestone-08-draft-assistants-overview.md) | P1-M8 decision gate and hosted/Apple reviewed-drafting checklist. |
| [phase-1/milestone-08-draft-assistants-architecture.md](phase-1/milestone-08-draft-assistants-architecture.md) | P1-M8 data-bounded assistant adapters and integration sequence. |
| [phase-1/milestone-09-local-publication-recovery-overview.md](phase-1/milestone-09-local-publication-recovery-overview.md) | P1-M9 local preview, approval, no-clobber output, conflict, and recovery checklist. |
| [phase-1/milestone-09-local-publication-recovery-architecture.md](phase-1/milestone-09-local-publication-recovery-architecture.md) | P1-M9 local output intent, receipt, reconciliation, and TUI sequence. |
| [phase-1/milestone-10-notion-publication-overview.md](phase-1/milestone-10-notion-publication-overview.md) | P1-M10 Notion compatibility gate, publication, partial-success, and retry checklist. |
| [phase-1/milestone-10-notion-publication-architecture.md](phase-1/milestone-10-notion-publication-architecture.md) | P1-M10 fail-closed hierarchy, append, read-back, and reconciliation sequence. |
| [phase-1/milestone-11-amendments-release-qualification-overview.md](phase-1/milestone-11-amendments-release-qualification-overview.md) | P1-M11 amendment, end-to-end acceptance, and Phase 1 qualification checklist. |
| [phase-1/milestone-11-amendments-release-qualification-architecture.md](phase-1/milestone-11-amendments-release-qualification-architecture.md) | P1-M11 guarded replacement and release-evidence sequence. |
| [planned-mvp-architecture.md](planned-mvp-architecture.md) | Local control server, TUI client, clean-architecture boundaries, connector contracts, workflow states, amendments, hosted drafting and data egress, process lifecycle, files, REST, streaming, and future MCP design including agent-client context cost. |
| [product-behavior.md](product-behavior.md) | Intended user journey, confirmed diary behavior, and product boundaries. |
| [ux-terms.md](ux-terms.md) | Proposed vocabulary for visible terminal components and code-facing names. |
| [ux-information-architecture.md](ux-information-architecture.md) | Proposed terminal screen containment, step states, connection and error states, and chrome layout. |
| [ux-gui.md](ux-gui.md) | Terminal UX: approved behavior, the Korean-IME input constraint, and proposed presentation details, kept separate. |
| [phase-1/changelog.md](phase-1/changelog.md) | Phase 1 owner decisions and material plan changes. |
| [research/markdown-input-format.md](research/markdown-input-format.md) | P1-M1 iCloud directory layout, Markdown/YAML format, attachment format, time semantics, and privacy constraints. |
| [research/notion-diary-section.md](research/notion-diary-section.md) | P1-M1 Notion authentication, block APIs, limits, and owner publication contract. |
| [research/mvp-operational-constraints.md](research/mvp-operational-constraints.md) | P1-M1 privacy, two-output consistency, failure handling, and local/sync-state contracts. |
| [research/agent-assisted-diary-composition.md](research/agent-assisted-diary-composition.md) | P1-M1 diary drafting options and Apple on-device two-pass/manual-fallback workflow. |
| [research/rust-tui-stack.md](research/rust-tui-stack.md) | P1-M1 Rust TUI, editor, HTTP, parsing, Keychain, compatibility, licensing, and technical direction. |
| [research/macos-ime-language-status.md](research/macos-ime-language-status.md) | P1-M1 probe results and recommendation for a Rust-backed input-language indicator in Apple Terminal. |
| [research/p1-m1-validation-evidence.md](research/p1-m1-validation-evidence.md) | P1-M1 synthetic source, composition, Notion, TUI, Keychain, and input-language probe evidence. |
| [development-process.md](development-process.md) | Phases, milestones, IDs (`P1-M2`), definition of done, where plans live, and doc status values. |
| [contribution-guide.md](contribution-guide.md) | Local setup, required validation, commit message style, and pull request conventions. |
