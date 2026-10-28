# Product behavior

Status: Proposal

This document summarizes the intended user-visible diary workflow. The
[architecture](planned-mvp-architecture.md) describes where the behavior lives;
the [terminal UX](ux-gui.md) describes how it appears to a person.

## Core journey

0. Start the server by hand with `nd serve` in its own terminal, then open the
   TUI with `nd`. The TUI never starts the server and says so when it cannot
   attach.
1. Choose a diary day; a new workflow defaults to today in the Mac's local time
   zone, and unfinished drafts are listed first. Choosing a day that already has
   a completed entry offers to amend it. Discover Ayumi Markdown notes only in
   that day's directory and the adjacent date directories. An explicitly chosen
   extra date can be loaded separately.
2. Review proposed notes. Both the directory date and a valid offset-aware
   `created_at` date can suggest a note for the chosen day. Show disagreements
   and warnings; the person decides which readable notes to include.
3. Acquire selected cloud-only notes when needed, then confirm a fresh source
   selection. A source change requires rereading and reconfirmation before any
   output. A day with no notes can continue through an explicit "continue
   without notes" confirmation, which leaves only the manual editor.
4. Write the entry in a multiline editor. Optional model assistance, hosted by
   default or Apple on-device, extracts source-linked memories for review, then
   drafts from confirmed memories. The manual path stays available after any
   model failure.
5. Review inferred reflections and confirm the complete edited entry. Choose
   US English or South Korean Korean as the language of the entry, and a
   concise, standard, or reflective style; remember the last language choice
   and start with concise style. Source notes are expected to be en-US, so a
   South Korean Korean entry is a translation.
6. Preview the local Markdown file and the Notion section, including their
   destination paths and text. Approve the exact output plan before writing.
7. Write the local file first. Publish the confirmed body to the configured
   Notion root's year and month hierarchy only after the read-only preflight
   and conflict checks. Show success, pending, or blocked state separately for
   each destination.

## Boundaries people can rely on

- Readable notes remain selectable when front matter is missing or malformed;
  invalid UTF-8 receives a warning and has no readable body.
- Japanese is not supported as a source or writing language. The app does not
  detect or block it.
- Audio and attachment paths do not enter composition or output, and attachment
  presence does not change eligibility.
- The complete edited entry requires confirmation, and a blank entry cannot be
  confirmed. Model-generated content is never accepted automatically.
- Drafting is hosted by default, so readable note bodies, memories, drafts, and
  the edited entry may be sent to the hosted provider, with no extra consent
  step. Note metadata (location, weather), attachment paths, audio, and
  credentials never leave the Mac. The screen labels which composition path is
  active.
- A different local file at the target path is never overwritten automatically.
  Conflicting same-day Notion content stops publication for resolution. A
  conflict shows both versions and offers only reread, discard the entry, or
  save under a different diary day.
- A completed entry can be amended: the editor opens with the saved text
  (including your own hand edits to the file, flagged), and approving the
  replacement plan replaces only content the app itself wrote. Anything else
  stays a conflict.
- If Notion is unconfigured, unreachable, or fails its preflight, the local
  entry can still be approved and written, and Notion shows blocked and can be
  retried later after a new Notion-only preview and approval. If Notion fails
  after the local write, the local entry remains available and Notion
  publication is pending; retrying reuses the approval only if the plan is
  unchanged. An uncertain Notion write is inspected before any retry.
- Draft and source text is purged 7 days after every output verifies and
  abandoned drafts after 30 days. Receipts and digests stay, and a draft can be
  discarded at any time.

These outcomes follow the owner decisions in the [research summary](phase-1/milestone-01-research-summary.md)
and the later decisions recorded in [AGENTS.md](../AGENTS.md).
