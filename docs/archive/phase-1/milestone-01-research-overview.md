# P1-M1: MVP research and decisions overview

Status: Archived

This milestone consolidates the research already completed for the macOS TUI
MVP and closes only the questions needed to make later Phase 1 planning
concrete. It does not plan or implement another milestone.

## Goal

Produce an evidence-backed set of Phase 1 product and architecture decisions
covering the source notes, diary composition workflow, local output, Notion
publication, and supported macOS environment.

## In scope

- Consolidate the existing research into a coherent MVP decision set.
- Validate assumptions that currently rest on only two private source samples.
- Inspect the configured Notion page with owner-authorized, read-only access
  when a credential is available; otherwise preserve the unknown and define a
  fail-closed first-run preflight.
- Evaluate the smallest composition workflow that preserves source grounding,
  privacy, and manual recovery.
- Decide the macOS TUI capabilities and technical constraints that affect later
  dependency and architecture choices.
- Record decisions and their evidence without selecting or implementing a later
  milestone.

## Out of scope

- Production Rust code, dependency changes, or build-tool changes.
- Writing implementation architecture for a later milestone.
- Modifying source diary files or publishing content to Notion.
- Research for the standalone iOS phase except where an irreversible Phase 1
  choice would otherwise prevent a shared core later.
- Planning Phase 1 milestones after P1-M1.

## Research already completed

- [x] The observed iCloud directory layout, Markdown/YAML fields, attachment
  links, timestamp discrepancy, privacy boundary, and malformed-input behavior
  are documented in [Markdown input format research](../research/markdown-input-format.md).
- [x] Notion authentication choices, block and Markdown APIs, append limits,
  duplicate hazards, retry behavior, and a preferred block-API direction are
  documented in [Notion daily diary section research](../research/notion-diary-section.md).
- [x] The non-atomic local-file and Notion workflow, partial-success states,
  source immutability, sensitive-data boundary, and explicit publication
  confirmation are documented in [MVP operational constraints](../research/mvp-operational-constraints.md).
- [x] Manual and model-assisted composition boundaries, grounded two-pass
  drafting, provider and on-device options, privacy considerations, and an
  evaluation method are documented in [Agent-assisted diary composition research](../research/agent-assisted-diary-composition.md).
- [x] Candidate Rust TUI, editing, parsing, HTTP, concurrency, Keychain, and
  testing components are documented in [Rust TUI stack research](../research/rust-tui-stack.md).
- [x] The owner decided that Ayumi source notes are proposed for a day from both
  their directory date and offset-aware Markdown-header `created_at` date. Date
  disagreements remain visible, and the user's include/exclude confirmation is
  authoritative even when a source date does not match the diary day.
- [x] The owner decided that Phase 1 ignores Ayumi audio attachments. Audio and
  attachment paths are excluded from composition, and attachment presence does
  not affect source-note eligibility.

Checked items above mean the research note exists and contains supporting
evidence. They do not pre-approve a product choice or a dependency.

## Evidence completed; owner interview recorded

The evidence requirements below are covered by the updated topic notes,
[validation evidence](../research/p1-m1-validation-evidence.md), and
[research summary](milestone-01-research-summary.md). The owner confirmed or
replaced the recommendations in an interview on 2026-10-28. The configured
Notion page and a live Japanese switch were unavailable; those evidence limits
are explicitly recorded in the summary. The owner later defined the configured
Notion page as a root with year/month subpages and daily entries appended to
the month page. A follow-up synthetic hierarchy fixture covers year/month path
resolution and ambiguous creation responses. The owner reaffirmed the
runtime-only read-only root preflight on 2026-10-28.

### Source and date semantics

- Obtain synthetic or owner-approved boundary examples that cover local
  midnight, missing or extra front matter, and a source file changing during a
  read. Synthetic cases should also confirm that attachment presence or count
  does not alter source-note eligibility or leak paths into composition.
- Define and test the discovery range and filters that make cross-date source
  selection practical without loading the entire private archive into the TUI.
- Specify the selection-screen states for matching metadata, directory/header
  disagreement, manual inclusion from another day, and manual exclusion; keep
  the confirmed selection stable through composition and output preview.
- Use the diary day selected in the workflow consistently for local output,
  Notion headings, duplicate detection, and retry state, without relabeling an
  included source note's original metadata.
- Determine whether ordinary filesystem reads provide adequate diagnostics for
  cloud-only files, or whether the TUI needs a macOS iCloud availability adapter.

### Composition and review workflow

- Compare manual editing with the smallest viable model-assisted prototype on
  synthetic or expressly consented fixtures using the existing evaluation
  rubric.
- If hosted inference remains a candidate, document its privacy and operating
  terms. The owner instead selected on-device composition for Phase 1.
- Decide whether the TUI needs multiline editing or only selection, preview,
  and confirmation; also decide the default language, length, tone, and policy
  for inferred reflections.
- Decide whether Notion receives the full confirmed entry or a user-selected
  subset, and make that selection visible before publication.

### Local and Notion outputs

- Decide the local output directory, file envelope and naming rule, collision
  behavior, and the format and location of pending/completed Notion sync state.
- No credential was available, so preserve the configured Notion page's
  top-level structure and permissions as unknown and require a fail-closed
  first-run read-only preflight without copying content into logs.
- Decide the canonical Notion heading, insertion order, paragraph mapping, and
  behavior when a same-day section already exists.
- Exercise a synthetic Notion page or equivalent fixture for pagination,
  ambiguous writes, content limits, and read-before-retry behavior; do not write
  to the owner's diary page during research.

### Supported environment and technical direction

- Decide the minimum macOS version and Rust version for Phase 1.
- Execute the [macOS IME language status research plan](../research/macos-ime-language-status.md):
  verify that a disposable Rust probe can show English/Japanese input-language
  changes in Apple Terminal, then recommend one integration and one fallback
  for later architecture planning.
- Decide between Tokio tasks and dedicated worker threads for filesystem,
  composition, and Notion work while keeping the TUI responsive.
- Decide credential storage. The owner chose a plaintext per-user global
  config file with owner-only permissions; the disposable Keychain probe
  remains research evidence rather than an implementation requirement.
- Confirm the TUI/editor candidates interoperate on the supported toolchain and
  that terminal restoration, cancellation, and paste behavior can be tested
  without depending on a user's real diary.

## Deliverables

- Updated research notes containing the new evidence, dates, and sources.
- A compact decision record in the Phase 1 changelog for every resolved choice,
  including the reason and affected research note.
- Updates to `AGENTS.md` if a decision changes the project's explicit decided or
  undecided set.
- A final research summary that maps each decision to evidence and names any
  remaining uncertainty that does not block later Phase 1 planning.

## Completion checklist

- [x] Existing Phase 1 research is inventoried and linked from this plan.
- [x] The owner decision for metadata-based day proposals and user-controlled
  cross-date source selection is recorded in the Phase 1 plan, research, and
  `AGENTS.md`.
- [x] The owner decision to ignore Ayumi audio in Phase 1 is recorded in the
  Phase 1 plan, research, and `AGENTS.md`.
- [x] The source/date research items have evidence and their blocking choices
  are decided.
- [x] The composition/review research items have evidence and their blocking
  choices are decided.
- [x] The local/Notion output research items have evidence and their blocking
  choices are decided.
- [x] The supported-environment research items have evidence and their blocking
  choices are decided.
- [x] The macOS IME language-status probe is complete and its preferred
  integration, fallback, permissions, and version constraints are documented.
- [x] Every decision is recorded in `docs/phase-1/changelog.md`; affected
  research notes, `AGENTS.md`, and the documentation index agree.
- [x] The final research summary contains no real diary prose, precise location,
  audio, credential, or other sensitive fixture data.
- [x] No later Phase 1 milestone has been planned as part of this work.

## Exit-gate exemption

P1-M1 is a research-only milestone and is explicitly exempt from the standard
milestone commands (`cargo fmt --all -- --check`, `cargo clippy
--all-targets --all-features -- -D warnings`, and `cargo test --all-features`).
Those commands are not completion evidence for this milestone, and an inability
to run them does not block P1-M1 from becoming Done.

This exemption applies only to P1-M1. It does not change the Phase 1 exit
criteria or pre-authorize an exemption for any later milestone.
