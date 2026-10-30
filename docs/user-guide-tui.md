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
   with a completed entry offers to amend it: open the Publish stage and choose
   **Amend this entry**. The editor opens with the saved text, then you confirm,
   preview current against replacement content, approve, and publish. If the file
   was edited by hand since publication, a banner says so.

## Moving around

- Tab / Shift-Tab and the arrow keys move focus between the Workflow
  Sidebar, Resources Sidebar, Main Pane, and Utility Sidebar (left to right),
  and between controls inside the Main Pane. On terminals narrower than 151
  columns the Utility Sidebar is hidden until focused, then opens over the
  right edge. Enter activates the focused control. There are no bare-letter
  hotkeys (Korean IME composes bare letters), so every action is reached by
  focus + Enter or by a Ctrl chord.
- Ctrl-G opens the Command Line (`help`, `retry`, `day YYYY-MM-DD`,
  `stage <stage>`, `save`, `quit`). Ctrl-S saves/flushes the editor. Ctrl-C
  quits — press it twice to confirm, which also doubles as the "draft isn't
  safely saved" warning on the first press. Ctrl-P / Ctrl-N move to
  the previous/next workflow stage.
- The Status Line (bottom) always shows diary day, workflow stage,
  connection, IME (`IME: EN` / `IME: KO` / `IME: ?`), progress, and errors.
  Work Status (top) explains current/pending work in more detail.

## Sources stage

The Resources Sidebar lists discovered Ayumi notes for the diary day and
adjacent dates, each marked `included`/`excluded`, its availability, and any
date-disagreement or warning marker.

- **View a source's content:** focus the candidate and press Enter (or
  activate "Inspect active prepared source"). Its sanitized text appears in
  the Main Pane under "Source text". This works for any readable candidate,
  included or not — opening a source never changes whether it's included.
- **Include/exclude a source as draft input:** focus the candidate and
  activate "Include/exclude active candidate". This toggles prospective
  membership in the set sent to the assistant for extraction/drafting. Any
  change to the included set invalidates prior bulk preparation and review,
  so re-run "Prepare included sources" and re-inspect each included source
  before confirming.
- **Confirm the selection:** once every included source has been reviewed,
  activate "Confirm reviewed source set". A day with no notes can instead use
  "Continue without notes", which leaves only manual editing later.
- **Refine a source with the assistant:** with a source inspected, activate
  "Refine inspected source with assistant". This sends only that source's
  text to the currently selected backend (hosted Codex or Apple on-device)
  for a bounded rewrite suggestion. The suggestion appears under "Suggested
  refinement" with two controls:
  - **Approve refinement** — writes the suggested text back as the real
    Ayumi note. Fails safely if the note changed since the suggestion was
    made.
  - **Reject refinement** — discards the suggestion; the note is unchanged.
- **Edit a source directly:** activate "Edit inspected source" to open it in
  the shared multiline editor (Ctrl-S saves).
- **Delete a source:** activate "Delete inspected source and linked audio",
  then confirm with Enter (Esc cancels). This hard-deletes the note and its
  safely resolved linked audio; audio bytes are never read or sent.

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
  assistant.
- "Extract memories" sends only the confirmed included source bodies to the
  selected backend; review the resulting memories, then "Confirm reviewed
  memories" and "Draft with selected assistant" to send those confirmed
  memories (not the raw sources) for drafting. A late result never overwrites
  newer unsent text.
- Choose US English or Korean, and concise/standard/reflective
  style (concise by default). A Korean entry from English notes is a
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

## Preview and Publish

After confirming the entry, open Preview and generate the local output plan.
It shows the target path, exact Markdown with `date` and `language` front
matter, SHA-256 digest, and initially blocked Notion status. Review the file before
approving the plan. Publish then starts a local publication job; an existing
different file is never replaced. The job can be cancelled before it commits,
and an uncertain write is inspected on restart before that diary day can act
again. A conflict shows the existing and planned text. Reread with Preview,
discard the workflow from Start, or choose another day in Preview; carried
text must be confirmed again.

If `paths.icloud_entries_root` is set, the same approval also saves an
identical copy to that iCloud Drive folder, which must already exist under
`~/Library/Mobile Documents/`. Preview shows an `iCloud mirror` line, and
Publish shows a mirror row only if the copy is pending, blocked (for example
the folder is missing or the file is still an iCloud placeholder), or in
conflict. Local and Notion are unaffected either way.
An amendment replaces that copy too, but only while it still holds the text the
app saved there; a file you edited in iCloud stays untouched and shows a conflict.

After the local receipt and read-back are verified, return to Preview and
generate the Notion-only plan. It shows the human-readable
`Notion root → YYYY → YYYY-MM` path, whether the year and month already exist,
the renderer/API versions, digest, and every ordered native block beginning
with `heading_2: YYYY-MM-DD (DDD)`. Inspect that projection and approve it;
Publish then creates only missing canonical pages and appends the section at
the month page's bottom. A duplicate or wrong-weekday same-day heading stops
Notion without changing the verified local file. A blocked retry needs a new
preview and approval, while an uncertain append is inspected before any new
write. The token, page IDs, and unrelated Notion prose are never shown.
The `paths.entries_root` setting may be absolute, relative to the directory
where `nd` starts, or begin with `~/` to resolve from the current user's home.
