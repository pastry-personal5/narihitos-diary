# TUI user guide

Status: Active

How to use the `nd` terminal client day to day: starting it, moving through
the workflow stages, and working with sources and the assistant. For the
behavior this guide describes, see [product-behavior.md](product-behavior.md)
and [ux-tui.md](ux-tui.md); this doc does not duplicate their detail.

## Starting

1. Run `nd serve` in its own terminal. The client never starts the server.
2. Run `nd` to open the TUI. If it can't attach, it shows a "server not
   running" state with the `nd serve` command.
3. On **Start**, choose a diary day. A new workflow defaults to today in the
   Mac's local time zone; unfinished drafts are listed first. Choosing a day
   with a completed entry offers to amend it: open the Publish Local stage and choose
   **Amend this entry**. The editor opens with the saved text, then you confirm,
   preview the current file against its replacement, approve, and publish. If the
   file was edited by hand since publication, a banner says so. An amendment
   changes the local file only; push the remotes again afterward.

## Moving around

- Tab / Shift-Tab and the arrow keys move focus between the Workflow
  Sidebar, Resources Sidebar, Main Pane, and Utility Sidebar (left to right),
  and between controls inside the Main Pane. On terminals narrower than 161
  columns the Utility Sidebar is hidden until focused, then opens over the
  right edge. Enter activates the focused control. There are no bare-letter
  hotkeys (Korean IME composes bare letters) except the Sources note-list
  shortcuts below, which work only under `IME: EN`; every action is reached by
  focus + Enter or by a Ctrl chord.
- Ctrl-G opens the Command Line (`help`, `retry`, `day YYYY-MM-DD`,
  `stage <stage>`, `save`, `quit`). Ctrl-S saves/flushes the editor. Ctrl-C
  quits — press it twice to confirm, which also doubles as the "draft isn't
  safely saved" warning on the first press. Ctrl-P / Ctrl-N move to
  the previous/next workflow stage.
- The Status Line (bottom) always shows diary day, workflow stage,
  connection, IME (`IME: EN` / `IME: KO` / `IME: ?`), progress, and errors.
  Work Status (top) explains current/pending work in more detail; its second
  row is a `Tip:` saying what to do to reach the next stage.

## Sources stage

The Resources Sidebar lists discovered Ayumi notes for the diary day and
adjacent dates, each marked `included`/`excluded`, its availability, and any
date-disagreement or warning marker.

The Sources stage is a short menu. Discovery of the last three days starts when
you first enter it. The Main Pane lists only the actions that apply (for
example Accept and Reject appear only while a refinement is shown).

- **Include or exclude a note:** focus the note in the Resources Sidebar and
  choose "Include this note" / "Exclude this note" in the Main Pane menu.
- **View a note's content:** press Enter on the note (or "View content"). Its
  sanitized text appears in the Main Pane under "Source text". Viewing never
  changes inclusion and is not required before confirming.
- **Confirm the list:** choose "Confirm Source List". It downloads any
  cloud-only included note, reads the included notes, and confirms them, with
  progress in Work Status. If an included note cannot be read, nothing is
  confirmed and the reason is shown. With nothing included the same item reads
  "Continue without notes" and asks for Enter to confirm. Changing the
  included set later needs a new confirmation. The control shows why it is
  disabled if more than 64 notes are included; exclude notes to continue.
- **Refine a note with the assistant:** with the note's text shown and an
  assistant selected, choose "Refine with assistant". Only that note's text
  goes to the selected backend. The suggestion appears under "Suggested
  refinement" with two controls:
  - **Accept refinement** — writes the suggested text back as the real Ayumi
    note. Fails safely if the note changed since the suggestion was made.
  - **Reject refinement** — discards the suggestion; the note is unchanged.
- **Edit a note directly:** choose "Edit note" to open it in the shared
  multiline editor (Ctrl-S saves).
- **Delete a note:** choose "Delete note and linked audio", then confirm with
  Enter (Esc cancels). This hard-deletes the note and its safely resolved
  linked audio; audio bytes are never read or sent.
- **Hide a note:** "Hide note" removes it from the list for this session and
  excludes it; "Show hidden notes" brings them back (excluded). Nothing is
  written, and hiding resets when you restart or open another day.
- **Find more notes:** "Refresh notes" searches again, "Add a date" opens a
  field for one extra date (Enter searches it), and "Load more" pages further.
- **Letter shortcuts:** with the note list focused and `IME: EN`, `i`
  include, `x` exclude, `v` view, `h` hide, `u` show hidden, `r` refine, `n`
  reject, `e` edit, `d` delete (asks to confirm), `f` refresh, `a` add a date,
  `m` load more, `?` help; either case. Accept refinement and Confirm Source
  List have no letter. Under `IME: KO` or `IME: ?` letters do nothing. `?` (or
  "Shortcuts help") opens a Help Overlay listing them; Esc, Enter, or `?`
  closes it.

A source change always requires re-reading and re-confirming before any
later output.

## Compose stage

- Pick a backend in the Utility Sidebar's Assistant Widget, which lists the
  server's capability list (hosted default or Apple on-device) and a
  "None (manual only)" row; focus a row and press Enter. The widget is
  read-only with no open workflow, while an assistant job runs, and while
  actions are disabled. The label shows which composition path is active. The
  Diary Day Widget above it always shows the diary day you are working on.
  Manual editing in the Draft editor is always available, with or without an
  assistant. The choice, including "None (manual only)", is saved and restored
  for newly created workflows after restarting the TUI. An existing workflow
  keeps its own assistant choice.
- "Extract memories" sends only the confirmed included source bodies to the
  selected backend; review the resulting memories, then "Confirm reviewed
  memories" and "Draft with selected assistant" to send those confirmed
  memories (not the raw sources) for drafting. A late result never overwrites
  newer unsent text.
- Choose US English or Korean, and concise/standard/reflective
  style (concise by default). The current language and style carry to newly
  created workflows while this TUI session is open; an existing workflow keeps
  its own saved settings. A Korean entry from English notes is a
  translation and is labeled as such.
- Focus a paragraph and activate "Review focused paragraph" to get a
  suggested rewrite for just that paragraph; "Accept current suggestion" or
  "Reject current suggestion" — accept fails safely if the paragraph changed
  since the suggestion was made.
- "Cancel assistant action" stops a running job; "Refresh assistant
  resources" re-lists available backends in the Assistant Widget.

## Review stage

Review any inferred-reflection spans (marked inline between reserved
delimiters — accept by removing the delimiters, edit the text inside, or
remove the whole span) and then "Confirm complete entry". A blank entry
can't be confirmed, and confirmation is blocked while any reserved delimiter
remains.

## Preview, Publish Local, and Publish Remote

After confirming the entry, open Preview and generate the local output plan.
Preview is only about the file on this Mac: it shows the target path, exact
Markdown with `date` and `language` front matter, and SHA-256 digest, and says
nothing about iCloud or Notion. Review the file before approving the plan.
Publish Local then starts a local publication job; an existing different file
is never replaced without your explicit choice. The job can be cancelled before it commits,
and an uncertain write is read back in the same job. If that read-back is
unavailable, restart inspects it again before that diary day can act. A
file that changes after you approved it is shown again in Preview, with the
existing and planned text.
Publish Local also keeps the confirmed diary entry visible under **Diary entry
to publish**. Its different foreground color separates the prose from status
and controls, and the prose is indented beneath the heading; the heading keeps
the distinction when `NO_COLOR` is set.
When another entry already exists, **Existing local entry** shows that content
in a second foreground color, separate from the entry you are publishing.

When a file already exists for the day, Preview never stops at a conflict: the
plan defaults to Append, so it can be approved as it stands, and the menu also
offers **Overwrite the existing entry**. Append keeps what is there and adds a `---` line, `*Appended 2026-10-30 21:14*`, and your new text
below it. Overwrite replaces the file after saving a copy beside it as
`<date>.md.bak-<digest>` and asks you to press Enter a second time. Either way
Preview shows the exact result, and nothing is written until you approve; if the
file changes in between, the approval is refused and Preview shows it again. The
same two choices appear in Notion's menu when it already holds an entry. iCloud
offers only Overwrite so its copy matches the local file byte for byte.

Saving the local file completes the day. Nothing is sent to iCloud or Notion
automatically.

### Publish Remote

Once the local file is saved, open **Publish Remote** (any time, including after
reopening the day later). The Resources Sidebar lists the remotes, iCloud and
Notion, each with a status: `Not pushed`, `Exists` (it already holds content that
is not this app's), `Pushed`, or `Blocked`. Pick one to open its menu in the Main
Pane:

The Main Pane keeps the confirmed **Diary entry to publish** visible in a
different foreground color while you inspect each remote.
If the remote already contains an entry, **Already there** shows it in a second
foreground color so the existing and proposed prose remain easy to compare.
Both prose blocks are indented beneath their headings.

- **Show what will be pushed** refreshes the read-only plan.
- **Check readiness** asks why the remote is or is not ready. Nothing is checked
  until you choose it, and the check never writes anything. It shows `READY`,
  `NOT READY`, or `PUBLISHED`, a plain reason, and the next step.
- **Push** sends exactly the plan shown. Pushing is the approval; there is no
  separate approve step.

The remote is looked at as soon as you pick it. If it already holds an entry for
the day, that entry is shown. Notion offers **Append to the existing entry** and
**Overwrite the existing entry**; iCloud offers only **Overwrite the existing entry**.
Choose a mode to see the exact result, then Push. Append keeps what is there;
Overwrite saves a copy of an iCloud file first
(a Notion section's old blocks go to Notion's trash) and asks you to press Enter a
second time. If the remote changes while you decide, the push is refused and the
screen shows what is there now instead of leaving you stuck.

iCloud needs `paths.icloud_entries_root` under `~/Library/Mobile Documents/`; the
folder must already exist. Its copy is the saved local file's exact text. An
amendment never changes a remote on its own: push again, and iCloud updates the
copy this app saved earlier without asking (your own edits there are shown as
`Exists`).

For Notion, the menu shows the human-readable
`Notion root → YYYY → YYYY-MM` path, whether the year and month already exist,
the renderer/API versions, digest, and every ordered native block beginning
with `heading_2: YYYY-MM-DD (DDD)`. Inspect that projection, then Push;
the push creates only missing canonical pages and adds the section at
its place in ascending date order on the month page (the `Position`
line says where; sections already out of order stay as they are). If the month
page has two sections for the same day or a wrong-weekday heading, this app
cannot change it safely: the screen shows a warning and a link to that Notion
page so you can fix it by hand in the Notion app or on Notion Web, then choose
Check readiness again. An uncertain append is inspected before any new write.
The token, page IDs, and unrelated Notion prose are never shown, except that
one page link. A readiness answer applies to the workflow as it was when you
asked, so it returns to `Not checked` after anything changes.
The `paths.entries_root` setting may be absolute, relative to the directory
where `nd` starts, or begin with `~/` to resolve from the current user's home.
