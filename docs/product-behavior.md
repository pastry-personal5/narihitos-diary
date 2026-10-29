# Product behavior

Status: Active

This document summarizes the intended user-visible diary workflow. The
[architecture](architecture.md) describes where the behavior lives;
the [terminal UX](ux-tui.md) describes how it appears to a person.

P1-M7 implements the manual journey through complete-entry Review. P1-M8 is
active for assistant planning, with production work gated on its owner decision
packets; Preview, Publish, and amendments remain planned for later milestones.
The implemented client keeps unavailable later behavior visible but locked, so
the stage model below remains stable as it arrives.

## Core journey

The left Workflow Sidebar always shows the six stages `Start`, `Sources`,
`Compose`, `Review`, `Preview`, and `Publish`. The middle Resources Sidebar
shows the sources, draft, assistant suggestions, or destinations for the active
stage, and activating one displays its content in the right Main Pane.

1. **Start:** Start the server by hand with `nd serve` in its own terminal,
   then open the TUI with `nd`. The TUI never starts the server and says so when
   it cannot attach. Choose a diary day; a new workflow defaults to today in the
   Mac's local time zone, and unfinished drafts are listed first. Choosing a day
   that already has a completed entry offers to amend it.
2. **Sources:** Discover Ayumi Markdown notes only in the diary day's directory
   and the adjacent date directories; an explicitly chosen extra date can be
   loaded separately. Review proposed notes. Both the directory date and a
   valid offset-aware `created_at` date can suggest a note for the chosen day.
   Show disagreements and warnings; the person decides which readable notes to
   include. Activating a source note in the Resources Sidebar shows its readable text in
   the Main Pane without changing whether it is included. Acquire included
   cloud-only notes when needed, then confirm a fresh source selection. A source
   change requires rereading and reconfirmation before any output. A day with no
   notes can continue through an explicit "continue without notes"
   confirmation, which leaves only manual editing in the Draft editor.
3. **Compose:** Write the entry in a multiline editor. Optional assistance is
   selected from the server's available backend list. The first backends are
   the chosen hosted default and Apple on-device, and the architecture accepts
   more without changing the workflow. The assisted flow sends only the
   confirmed readable source bodies to exactly one selected backend for
   extraction. After the person reviews the source-linked memories, drafting
   sends those confirmed memories—not the source bodies—to one selected backend.
   When the draft job
   completes, it conditionally populates the unconfirmed editor only if the
   buffer has not changed since the request, then becomes the active editable
   resource. A late result never overwrites newer text. The Draft editor
   stays available for manual editing after any assistant failure. The confirmed
   source notes remain in the Resources Sidebar for read-only inspection. Choose US
   English or South Korean Korean as the entry language and a concise, standard,
   or reflective style; remember the last language choice and start with concise
   style. Source notes are expected to be en-US, so a South Korean Korean entry
   is a translation.
4. Still in **Compose**, focus a paragraph and ask an available backend to
   review it. The Main Pane shows the current paragraph and suggestion. Accept
   applies a still-current suggestion; reject leaves the entry unchanged.
5. **Review:** Review inferred reflections and confirm the complete edited
   entry.
6. **Preview:** Preview the local Markdown file and the Notion section,
   including their destination paths and text. Approve the exact output plan
   before writing.
7. **Publish:** Write the local file first, at the configurable default path
   `~/Documents/narihitos-diary/entries/YYYY/MM/YYYY-MM-DD.md`. Publish the
   confirmed Markdown body, with YAML front matter stripped, under a
   `heading_2: YYYY-MM-DD` section at the bottom of the configured Notion
   root's `YYYY` year and `YYYY-MM` month subpages, creating missing subpages
   as needed, only after the read-only preflight and conflict checks. Show
   success, pending, or blocked state separately for each destination.

## Boundaries people can rely on

- Readable notes remain selectable when front matter is missing or malformed;
  unknown front-matter fields are ignored and missing non-date fields are
  tolerated. Only a valid offset-aware `created_at` contributes the header
  date; otherwise the directory date is kept with a warning.
- Source files are read as UTF-8, with or without a BOM, and accept both LF
  and CRLF line endings; invalid UTF-8 receives a warning and has no readable
  body.
- Japanese is not supported as a source or writing language. The app does not
  detect or block it.
- Audio and attachment paths do not enter composition or output, and attachment
  presence does not change eligibility.
- The complete edited entry requires confirmation, and a blank entry cannot be
  confirmed. Assistant drafts and paragraph reviews are never accepted
  automatically.
- Drafting is hosted by default, so readable note bodies, memories, drafts, and
  the edited entry may be sent to the hosted provider, with no extra consent
  step. Note metadata (location, weather), attachment paths, audio, and
  credentials never leave the Mac. The screen labels the selected assistant
  backend and whether its composition path is hosted or Apple on-device.
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
and the later decisions recorded in the [Phase 1 changelog](phase-1/changelog.md).
