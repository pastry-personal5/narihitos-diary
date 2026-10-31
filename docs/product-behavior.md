# Product behavior

Status: Active

This document summarizes the intended user-visible diary workflow. The
[architecture](architecture.md) describes where the behavior lives;
the [terminal UX](ux-tui.md) describes how it appears to a person.

P1-M7 implements the manual journey through complete-entry Review. P1-M8 adds
the optional Codex hosted and Apple Foundation Models assistant paths inside
Compose. P1-M9 adds local Preview and Publish; P1-M10 adds the local-first
Notion-only retry flow while keeping live hierarchy qualification gated;
P1-M11 adds amendments of a completed day. P1-M22 supersedes the combined
publication tail with local-only Preview, Publish Local, and per-remote push.

## Core journey

The left Workflow Sidebar always shows the seven stages `Start`, `Sources`,
`Compose`, `Review`, `Preview`, `Publish Local`, and `Publish Remote`. The
middle Resources Sidebar shows the sources, draft, assistant suggestions, or
destinations for the active stage, and activating one displays its content in
the right Main Pane.

1. **Start:** Start the server by hand with `nd serve` in its own terminal,
   then open the TUI with `nd`. The TUI never starts the server and says so when
   it cannot attach. Choose a diary day; a new workflow defaults to today in the
   Mac's local time zone, and unfinished drafts are listed first. Choosing a day
   that already has a completed entry offers to amend it.
2. **Sources:** Discover Ayumi Markdown notes only in the diary day's directory
   and the adjacent date directories; an explicitly chosen extra date is a
   Mac-local calendar day (Ayumi names are UTC, so notes whose local creation
   date is that day are loaded, even from the neighbouring UTC directory). Review proposed notes. Both the directory date and a
   valid offset-aware `created_at` date can suggest a note for the chosen day.
   Show disagreements and warnings; the person decides which readable notes to
   include. Activating a source note in the Resources Sidebar shows its readable text in
   the Main Pane without changing whether it is included. From that inspection,
   the person can request a digest-bound assistant refinement, approve or reject
   it, edit the body directly, or explicitly confirm hard deletion of the note
   and its safely resolved linked audio. Approve/edit writes the real Ayumi note.
   Reading a note is optional: one Confirm Source List action acquires included
   cloud-only notes when needed, reads them, and confirms a fresh source
   selection. A source
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
   English or Korean as the entry language and a concise, standard,
   or reflective style; remember the last language choice and start with concise
   style. Source notes are expected to be en-US, so a Korean entry
   is a translation.
4. Still in **Compose**, focus a paragraph and ask an available backend to
   review it. The Main Pane shows the current paragraph and suggestion. Accept
   applies a still-current suggestion; reject leaves the entry unchanged.
5. **Review:** Review inferred reflections and confirm the complete edited
   entry.
6. **Preview:** Preview the local Markdown file only: its destination path and
   exact text. Preview says nothing about any remote. Approve the exact output
   plan before writing.
7. **Publish Local:** Write the local file, at the configurable default path
   `~/Documents/narihitos-diary/entries/YYYY/MM/YYYY-MM-DD.md`. Keep the
   confirmed diary entry visible in a distinct foreground color while showing
   local status and controls. If another local entry exists, show it in a
   second distinct foreground color. Indent both prose blocks beneath their
   headings. A verified local write completes the day.
8. **Publish Remote:** The remotes (iCloud, Notion) are listed; the person
   picks one, sees the confirmed diary entry in the same distinct foreground
   color and what the remote already holds in a second distinct color, checks
   readiness on request, and pushes. A push is its own approval of exactly the plan shown, and nothing is
   sent to a remote without it, now or later. Push the
   confirmed Markdown body to Notion, with YAML front matter stripped, under a native
   `heading_2: YYYY-MM-DD (DDD)` section, in ascending date order, in the configured Notion
   root's `YYYY` year and `YYYY-MM` month subpages. `DDD` is the fixed uppercase
   English weekday. Convert the body losslessly to native Notion blocks,
   preserving unsupported constructs literally. Allow unrelated hierarchy
   content, create only missing unique canonical subpages, and write only after
   the read-only preflight and changed-content checks. Show each remote's status
   separately: not pushed, exists, pushed, or blocked.

### iCloud remote

When `paths.icloud_entries_root` is configured, iCloud is a remote. It is never
written automatically: the person previews the copy (the saved local file's
exact bytes) in Publish Remote and pushes it. A missing folder or an evicted
placeholder shows the remote as blocked; it never affects the local file or
Notion. Content that is already there is shown, and Append or Overwrite
resolves it (an Overwrite backs the file up first); the copy this app saved
earlier is updated without asking. See the
[remote publication design](superpowers/specs/2026-10-31-remote-publication-stages-design.md)
(P1-M22); the [original mirror design](superpowers/specs/2026-10-30-icloud-output-design.md)
is superseded in part.

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
- Linked audio is deleted by path only when the person explicitly confirms
  deletion of its Ayumi source; it is never opened, decoded, transcribed, or sent.
- The complete edited entry requires confirmation, and a blank entry cannot be
  confirmed. Assistant drafts and paragraph reviews are never accepted
  automatically.
- Drafting is hosted by default when at least one configured Codex profile is
  ready. Extraction may send only confirmed readable note bodies; drafting may
  send only reviewed memories and an explicitly selected prior draft; paragraph
  review may send only the displayed paragraph. Writing settings accompany only
  drafting and review. Each hosted job starts a fresh confined `codex
  app-server` process for the selected profile with no readable filesystem
  roots, network, MCP servers, web search, approval prompts, or persisted
  history. Note metadata (location, weather), persistent source IDs, attachment
  paths, audio, credentials, and Notion details never leave the Mac. Apple
  Foundation Models keeps the same action contract on device and appears
  unavailable when the model or locale is not ready. The screen labels the
  selected backend and whether its composition path is hosted or Apple
  on-device; manual editing always remains available.
- A different local file at the target path is never overwritten without an
  explicit, confirmed choice, and nothing ever ends in a dead-end conflict. When
  a diary entry already exists at the local file, Preview defaults to
  **Append**, which keeps what is there, and offers **Overwrite** (P1-M21,
  P1-M22); Notion's remote menu offers the same two. iCloud shows existing
  content and offers only Overwrite, keeping its copy byte-identical to the
  verified local file (owner decision, 2026-10-31). A file that changes after
  it was previewed or approved is previewed again, never stuck. Append keeps the existing content and adds a
  `---` separator, an `*Appended YYYY-MM-DD HH:MM*` line (Mac-local time fixed
  when the plan is built), and the new text. Overwrite copies a file to
  `<name>.md.bak-<digest>` beside it first, or replaces a Notion section's body
  (Notion's trash is its safety net), and asks for a second Enter. Each is a
  per-publication choice that is previewed exactly and applies only while the
  content is still what was previewed; a duplicate or non-canonical Notion
  heading cannot be changed safely: the remote shows a warning with a link to
  fix the page by hand in Notion. An iCloud push receives the final local bytes.
- A completed entry can be amended: the editor opens with the saved text
  (including your own hand edits to the file, flagged). The amendment preview
  shows the file as it is now against its replacement, and approving binds that
  exact content; the file is replaced atomically, and only while it still holds
  what was shown. If it changed after approval, previewing again shows it as it
  is, or the amendment can be abandoned, which leaves files and receipts
  untouched. An amendment never writes a remote. Pushing again updates iCloud's
  copy the app itself saved (its own receipt digest), and updates a Notion
  section the app wrote in place (new body inserted and read back before the old
  blocks are trashed); anything else already there is shown for Append or
  Overwrite.
- If Notion is unconfigured, unreachable, or fails its preflight, the local
  entry can still be approved and written. Notion shows Blocked in Publish
  Remote; a malformed month page includes a repair URL. After repair, checking
  readiness refreshes its menu. Each push approves only the plan shown, and an
  uncertain Notion write is inspected before any retry.
- Draft and source text is purged 7 days after every output verifies and
  abandoned drafts after 30 days. Receipts and digests stay, and a draft can be
  discarded at any time.

These outcomes follow the owner decisions in the [research summary](phase-1/milestone-01-research-summary.md)
and the later decisions recorded in the [Phase 1 changelog](phase-1/changelog.md).
