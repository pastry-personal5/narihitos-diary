# Development process

Status: Active

Work is organized into numbered phases and milestones. General documentation rules are in [AGENTS.md](../AGENTS.md).

## Structure and status

- A **phase** is a product increment with a goal and exit criteria. Status: `Planned`, `Active`, or `Done`.
- A **milestone** is a small, verifiable step that fits one reviewable change or short series. Numbering restarts per phase. Status: `Planned`, `Active`, `Done`, or `Dropped`.
- IDs use `P<phase>-M<milestone>` (for example, `P1-M2`).
- Keep one active phase and follow its milestones in order unless its plan says otherwise.
- Phase and milestone numbers may change. Update IDs, filenames, and links together. Mark a dropped milestone `Dropped` with a one-line reason.

## Definition of done

A milestone is done when:

1. Its completion checklist is checked with acceptance evidence.
2. The gate passes: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-features`, plus `swift build` and `swift test` for the Apple helper once `helpers/apple-bridge` exists, unless the milestone overview records an explicit owner-approved exemption and its alternative completion evidence.
3. Affected docs and its phase-doc status are updated in the same change.

A phase is done when every milestone is `Done` or `Dropped` and its exit criteria hold.

## Plan files

Create a phase directory when planning that phase.

| File | Purpose |
|---|---|
| `docs/roadmap.md` | All phases with ID, title, status, and phase-doc link; the sole authority for the active phase. |
| `docs/phase-N/phase-N.md` | Phase goal, exit criteria, and brief milestone entries. |
| `docs/phase-N/milestone-NN-<theme>-overview.md` | Milestone goal, in/out scope, and completion checklist. |
| `docs/phase-N/milestone-NN-<theme>-architecture.md` | Affected modules, interfaces, approach, and sequencing. |
| `docs/phase-N/changelog.md` | Reverse-chronological decisions and plan changes not clear from commits. |

`N` is the phase number. `NN` is the zero-padded milestone number matching its ID; `<theme>` is kebab-case. Do not use a generic `phase.md` filename.

Keep phase-doc milestone entries short:

```markdown
# Phase 1: <title>

Status: Active
Goal: <one sentence>

## Exit criteria
- <observable outcome>

## Milestones

### P1-M1: <title>
Status: Done
Goal: <one sentence>
Plan: [overview](milestone-01-<theme>-overview.md), [architecture](milestone-01-<theme>-architecture.md)
Notes: <optional decisions or links>
```

Before implementation, create and link `milestone-NN-<theme>-overview.md` and `milestone-NN-<theme>-architecture.md`. Resolve any related "Undecided" item in `AGENTS.md` first.

Record owner decisions, departures from the architecture plan, decided open questions, and reordered/dropped milestones in `changelog.md`:

```markdown
# Phase 1 changelog

Status: Active

## Entries

- 2026-09-28 — <who> — <what changed and why; optional link>
```

## Document lifecycle

- Other docs use `Draft`, `Proposal`, `Active`, `Superseded by <link>`, or `Archived`. Proposals are not decisions.
- Archive completed phase plans, superseded designs, and concluded research; fix their links in `docs/roadmap.md`.
- Record resolved open questions under "Decided" in `AGENTS.md`.
