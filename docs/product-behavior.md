# Product behavior

Status: Proposal

This document summarizes the intended user-visible diary workflow. The
[architecture](planned-mvp-architecture.md) describes where the behavior lives;
the [terminal UX](ux-gui.md) describes how it appears to a person.

## Core journey

1. Choose a diary day. Discover Ayumi Markdown notes only in that day's
   directory and the adjacent date directories. An explicitly chosen extra
   date can be loaded separately.
2. Review proposed notes. Both the directory date and a valid offset-aware
   `created_at` date can suggest a note for the chosen day. Show disagreements
   and warnings; the person decides which readable notes to include.
3. Acquire selected cloud-only notes when needed, then confirm a fresh source
   selection. A source change requires rereading and reconfirmation before any
   output.
4. Write the entry in a multiline editor. Optional Apple on-device assistance
   extracts source-linked memories for review, then drafts from confirmed
   memories. The manual path stays available after any model failure.
5. Review inferred reflections and confirm the complete edited entry. Choose
   US English or South Korean Korean and a concise, standard, or reflective
   style; remember the last language choice and start with concise style.
6. Preview the local Markdown file and the Notion section, including their
   destination paths and text. Approve the exact output plan before writing.
7. Write the local file first. Publish the confirmed body to the configured
   Notion root's year and month hierarchy only after the read-only preflight
   and conflict checks. Show success or pending state separately for each
   destination.

## Boundaries people can rely on

- Readable notes remain selectable when front matter is missing or malformed;
  invalid UTF-8 receives a warning and has no readable body.
- Audio and attachment paths do not enter composition or output, and attachment
  presence does not change eligibility.
- The complete edited entry requires confirmation. Model-generated content is
  never accepted automatically and never sent to a hosted composition service.
- A different local file at the target path is never overwritten automatically.
  Conflicting same-day Notion content stops publication for resolution.
- If Notion fails after the local write, the local entry remains available and
  Notion publication is pending. An uncertain Notion write is inspected before
  any retry.

These outcomes follow the owner decisions in the [research summary](phase-1/milestone-01-research-summary.md).
