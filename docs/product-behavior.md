# Product behavior

Status: Proposal

This document summarizes the intended user-visible diary workflow. The
[architecture](planned-mvp-architecture.md) describes where the behavior lives;
the [terminal UX](ux-gui.md) describes how it appears to a person.

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
   include. Activating a source in the middle sidebar shows its readable text in
   the Main Pane without changing whether it is included. Acquire included
   cloud-only notes when needed, then confirm a fresh source selection. A source
   change requires rereading and reconfirmation before any output. A day with no
   notes can continue through an explicit "continue without notes"
   confirmation, which leaves only the manual editor.
3. **Compose:** Write the entry in a multiline editor. Optional assistance is
   selected from the server's available backend list. The first backends are
   the chosen hosted default and Apple on-device, and the architecture accepts
   more without changing the workflow. A draft action sends only the confirmed
   source snapshots to exactly one selected backend, which extracts
   source-linked memories for review and then drafts from confirmed memories.
   When the draft job completes, the draft becomes the active resource and its
   full text opens in the Main Pane's editor, where the person can edit it. The
   manual path stays available after any assistant failure. The confirmed
   sources remain in the middle sidebar for read-only inspection. Choose US
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
7. **Publish:** Write the local file first. Publish the confirmed body to the
   configured Notion root's year and month hierarchy only after the read-only
   preflight and conflict checks. Show success, pending, or blocked state
   separately for each destination.

## Boundaries people can rely on

- Readable notes remain selectable when front matter is missing or malformed;
  invalid UTF-8 receives a warning and has no readable body.
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
and the later decisions recorded in [AGENTS.md](../AGENTS.md).
