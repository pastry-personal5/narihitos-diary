# P1-M1: MVP research and decisions approach

Status: Archived

This document defines how P1-M1 research is gathered, reviewed, and converted
into decisions. It is a research approach, not an implementation architecture
or a plan for another milestone.

## Evidence rules

- Prefer synthetic fixtures. Use private source files only with explicit owner
  approval and never copy their contents or metadata values into the repository,
  prompts, logs, screenshots, or test artifacts.
- Separate an observation from an inference and from an owner decision. Record
  the date and source for observations that can change, such as provider terms,
  API behavior, supported versions, or crate compatibility.
- Reproduce operational claims with a synthetic fixture, disposable credential,
  or disposable Notion page where practical. Research against the configured
  diary page is read-only.
- Treat a recommendation as provisional until the owner-dependent product choice
  is recorded in the Phase 1 changelog.
- Do not add a Rust dependency merely to conduct research. Candidate dependency
  approval belongs to later implementation planning.

## Workstreams

| Workstream | Existing basis | Evidence to add | Decision output |
|---|---|---|---|
| Source notes and date | Markdown format, operational-constraints research, and the owner-decided selection and audio rules | Boundary-format matrix, cross-date discovery/selection scenarios, ignored-attachment cases, and iCloud failure observations | Selection-state contract, discovery bounds, accepted text/front-matter variants, and cloud-only behavior |
| Composition and review | Agent-assisted composition research | Synthetic comparison scored with the documented rubric | Manual/model boundary, editing needs, privacy and fallback policy |
| Local output | Operational-constraints research | Synthetic path, collision, atomic-write, and retry-state scenarios | Output contract and local sync-record contract |
| Notion publication | Notion and operational-constraints research | Authorized page-shape observation plus disposable-page API cases | Section schema, insertion/duplicate behavior, selected text contract |
| macOS TUI foundation | Rust TUI stack and macOS IME language-status research plans | Supported-version, interoperability, Apple Terminal input-language, terminal-restoration, concurrency, and Keychain probes | Minimum environment and technical direction for later planning |

## Sequence

1. Complete the source/date semantics first: preserve both Ayumi metadata dates,
   exercise the owner-decided include/exclude model in synthetic scenarios, and
   define the diary-day identity shared by filenames, Notion headings, and retry
   state.
2. Evaluate composition and review with that normalized source model, keeping a
   manual path available in every experiment.
3. Define local output before Notion publication so the local file remains the
   durable source of truth after partial success.
4. Inspect and probe Notion only after the content boundary and same-day identity
   are explicit.
5. Finish the supported-environment decisions after the required TUI behavior
   and background work are known.
6. Reconcile the research notes, Phase 1 changelog, `AGENTS.md`, and overview
   checklist before marking P1-M1 Done.

This order controls research dependencies only; it does not define later
implementation milestones.

## Decision record shape

Each resolved item in `docs/phase-1/changelog.md` should identify:

- the decision and date;
- the evidence or research note supporting it;
- rejected alternatives and the material tradeoff;
- privacy, reliability, and compatibility consequences;
- what remains uncertain and why that uncertainty does not block later Phase 1
  planning.

If evidence is insufficient, keep the overview item unchecked and record the
next bounded research action instead of filling the gap with an implementation
assumption.

## Milestone outputs

P1-M1 changes documentation and may create synthetic, non-sensitive research
artifacts. It does not create production modules, stable interfaces, dependency
selections, or a task breakdown for subsequent milestones.
