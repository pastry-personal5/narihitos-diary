# Phase 1: macOS TUI MVP

Status: Active

Goal: Make remembering a day feel effortless through a macOS terminal user
interface that turns existing Markdown notes into a final diary entry and
publishes its body to a Notion daily section.

## Scope

- Run as a TUI application on macOS.
- Read Markdown (`.md`) files from a mounted iCloud directory.
- Use the Ayumi directory date and offset-aware Markdown-header `created_at`
  date to propose source notes for a diary day, visibly flag date disagreements,
  and let the user include or exclude discovered notes regardless of their
  metadata dates.
- Ignore Ayumi audio attachments; they are not played, copied, uploaded,
  decoded, transcribed, or supplied to diary composition.
- Produce one final diary entry as a Markdown file on the local filesystem.
- Use the configured Notion root page with `YYYY` year and `YYYY-MM` month
  subpages; append the confirmed Markdown body as a daily section at the
  bottom of its month page.

## Exit criteria

- A user can run the application on macOS and interact with it through a TUI.
- The application can read Markdown source files from a mounted iCloud
  directory.
- Before composition, the TUI shows the source-note selection and any directory
  versus header date disagreement; the user can include or exclude discovered
  notes even when their metadata date does not match the diary day.
- The final diary entry is composed from exactly the confirmed source-note
  selection.
- Ayumi entries remain eligible with or without audio attachments, and neither
  audio bytes nor attachment paths appear in composition input or output.
- A completed run writes the final diary entry as a local Markdown file.
- The completed run creates missing Notion year/month subpages as needed and
  publishes the confirmed entry body under its diary-day heading at the
  bottom of the target month page.

## Milestones

### P1-M1: MVP research and decisions

Status: Done

Goal: Turn the existing MVP research into an evidence-backed, decision-ready
foundation for later Phase 1 planning.

Plan: [overview](../archive/phase-1/milestone-01-research-overview.md),
[research approach](../archive/phase-1/milestone-01-research-architecture.md)

Notes: The owner interview, Notion hierarchy evidence, and remaining runtime
checks are in the [research summary](../archive/phase-1/milestone-01-research-summary.md). This
research-only milestone used its documented exit-gate exemption. No other
Phase 1 milestones are planned yet.

