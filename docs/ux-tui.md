# Terminal UX

Status: Active

This document separates approved terminal behavior from proposed presentation
details. Product scope is in [product-behavior.md](product-behavior.md), and
component vocabulary is in [ux-terms.md](ux-terms.md).

## Current interface

The `nd` terminal client implements Start → Sources → Compose → Review,
including M8 backend selection, two-pass memory review and drafting, paragraph
suggestions, assistant job progress/cancellation, and manual fallback. P1-M9
adds the local Preview and Publish controls. P1-M10 now adds the separate
Notion-only preview, approval, and publication controls after local read-back
is verified; live hierarchy qualification remains pending owner authorization.
The adaptive shell keeps both sidebars inline at 120 columns or wider, overlays
Resources from 100 through 119, and opens either focused sidebar over the Main
Pane from 80 through 99. Work Status keeps its four content rows, while Command
Line and Status Line have separate rows beneath the body. Wave Terminal
qualification and the Apple Terminal secondary regression run passed on
2026-10-29.
The Status Line now consumes notification-based TIS language updates and
permanently selects the bounded 500 ms query fallback if notification
registration or delivery is unavailable. The Command Line accepts only
`help`, `retry`, `day YYYY-MM-DD`, `stage <stage>`, `save`, and `quit`;
selection and entry confirmation remain focused controls and cannot be typed to
bypass review.

**P1-M15:** a fourth column, the Utility Sidebar, sits to the right of the Main
Pane (Workflow, Resources, Main Pane, Utility). It stacks two Widgets: the
read-only Diary Day Widget above the Assistant Widget. The Assistant Widget
replaces the backend rows in the Resources Sidebar and includes a "None (manual
only)" row. All four columns are inline from 161 columns (the Main Pane stays at
least 50 wide); from 120 through 160 the Utility Sidebar opens as a focused
overlay, and below 120 it overlays alongside Resources. Tab order follows screen
order. See [the P1-M15 plan](phase-1/milestone-15-utility-sidebar-architecture.md).

**P1-M16:** a narrow exception to the no-bare-letter rule below: the
Sources-stage source list accepts single-letter shortcuts only while the Resources
Sidebar is focused and the IME indicator reads `IME: EN` (never Accept
refinement or Confirm Source List). `?` opens a large modal Help Overlay of those
shortcuts, and `h`/`u` hide and show notes for the session. The Sources stage is
a flat menu: include or exclude a note, optionally refine it with the assistant
(accept or reject), edit or delete it, and Confirm Source List. Reading each note
is optional, and discovery starts when the stage is entered. Work Status's
second row carries a `Tip:` naming the next step to the following stage. See
[the P1-M16 plan](phase-1/milestone-16-simple-sources-architecture.md).

## Approved behavior to represent

- Show directory and header dates together when choosing source notes, visibly
  mark disagreements and malformed metadata, and allow inclusion or exclusion
  regardless of either date.
- Keep the two-sidebar layout stable beneath a full-width Work Status pane: the
  left Workflow Sidebar shows `Start`, `Sources`, `Compose`, `Review`,
  `Preview`, and `Publish`; the middle Resources Sidebar shows context
  resources; and the right Main Pane shows the active resource. Work Status has
  four content rows excluding title and borders and explains current work and
  useful messages. Activating a readable source is distinct from including it
  in the source set.
- Request downloads only for selected cloud-only notes. Keep unavailable and
  unreadable states distinct from an empty note.
- List each Ayumi source by its Mac-local creation time as
  `YYYY-MM-DD HH:MM:SS` without the `.md` extension. Ayumi file and directory
  names are UTC, so the TUI converts them; the real file name appears in the
  source detail. The Resources Sidebar is 16 columns wider than a quarter of
  the body to fit that label.
- Start a new workflow on today's date in the Mac's local time zone, and list
  unfinished drafts first. The person can discard a draft at any time.
- Offer the multiline Draft editor for manual editing at all times after source confirmation,
  including when assistant support is unavailable or fails. Optional
  assistance uses a source-linked memory review before drafting.
- Let a day with no source notes continue through an explicit “continue without
  notes” confirmation. Only manual editing in the Draft editor is offered, and a blank entry
  cannot be confirmed.
- List assistant backends from server capabilities instead of assuming a fixed
  two-item choice. Label the selected backend and its composition path (hosted
  by default, or Apple on-device) in the Setup check, Compose stage, and Status
  Line. The label is information, not a consent step; manual editing is not an
  assistant backend.
- Send only the confirmed included source snapshots to exactly one backend when
  the person invokes an assistant action. Opening, highlighting, or including a
  source does not itself call a backend.
- Let a person focus a paragraph for assistant review and explicitly accept or
  reject the suggestion. The target is the Markdown paragraph containing the
  editor cursor, bounded by blank lines and shown before the request. Accept
  must fail safely if the paragraph changed; reject does not alter the entry.
- Offer US English and Korean per entry, remember the last choice,
  and offer concise, standard, and reflective styles with concise as default.
  Carry the current language and style to new workflows during the TUI session.
  Persist the assistant choice, including manual mode, for new workflows after
  restart; opening an existing workflow keeps its saved settings.
  The choice names the language of the finished entry; source notes are
  expected to be en-US. Mark assistant-inferred reflections for human review.
- Require confirmation of the complete edited entry and then show the exact
  local and Notion outputs before the first write. Display the root/year/month
  path and any subpages that would be created. The Notion projection begins
  with `YYYY-MM-DD (DDD)`, using a fixed uppercase English weekday, and shows
  the ordered native blocks produced by lossless Markdown conversion. Show
  conflicts and partial success plainly.
- Preview shows only the local file and never mentions a remote. Publish Local
  shows the local save and, once the day is completed, points to Publish Remote.
  Publish Remote lists the remotes (iCloud, Notion) in the Resources Sidebar with
  a status (`Not pushed`, `Exists`, `Pushed`, `Blocked`); the Main Pane shows the
  selected remote's menu: Show what will be pushed, Check readiness, Append and
  Overwrite (only while it holds content, which is shown automatically), and Push.
  Push is the approval; Overwrite asks for a second Enter and Esc cancels. A
  blocked Notion month page shows a warning and a URL to fix it by hand. A remote
  failure never raises a modal and never
  fails the local publication.
- Each remote's menu offers a manual `Check readiness` control. Nothing runs
  automatically. It shows `Not checked` before a check and `Checking...` during
  one, then `READY`, `NOT READY`, `CONFLICT`, or `PUBLISHED` with a
  plain-language reason and next step. The check is read-only, needs no mutation
  authority, and its answer clears when the workflow revision, day, or remote
  changes.
- The Notion plan shows a `Position` line (`end`, `after YYYY-MM-DD`, or
  `before YYYY-MM-DD`) saying where the day's section goes on the month page
  (P1-M20). A page whose order changes before the write needs a new preview.
- No screen ends in a dead-end "conflict". Preview never stops at one: when the
  local file already holds something else, the plan defaults to **Append**, which
  keeps what is there and adds the entry below it, so it can be approved as it
  stands. **Overwrite** stays an explicit choice and never happens on its own.
  Content already at a remote is shown in that remote's menu with the same two
  choices. If a file changes after it was previewed or approved, the screen says
  so in plain words and shows it again (an approval that is refused previews the
  file again automatically), as described in
  [Publication and recovery](architecture.md#publication-and-recovery).
- When an entry already exists, Preview lists **Append to the existing entry**
  and **Overwrite the existing entry** after its two standing controls (P1-M21).
  They are reached by focus and Enter like every control, and appear only while
  the preview shows an existing entry. Choosing one re-previews the exact plan: a
  `Mode:` line, the existing content, and the resulting content (for a file, the
  planned backup name). Overwrite asks for a second Enter and Esc cancels;
  Append needs none because the existing text is kept. Remotes offer the same
  two in their own menu.
- When Notion is unconfigured, unreachable, or fails its preflight, show it as
  blocked in the preview and still allow the local entry to be approved and
  written. Notion can be retried later: a blocked destination needs a new
  Notion-only preview and approval, and a pending one reuses the approval only
  while its plan is unchanged. The Preview and Publish controls are shared:
  once the local file is verified, the same preview, approve, and publish
  actions target Notion only and never rewrite the local file.
- Offer to amend a completed day. The editor opens with the saved text and, if
  the file was hand-edited, a banner saying it changed since publication. The
  amendment preview shows the current file against its replacement, nothing
  about any remote. The offer appears on the Publish Local stage of a completed
  day ("Amend this entry"); the Preview stage then lists preview and approve,
  plus abandon the amendment once the file changed after approval. Publish Local
  replaces the file. Remotes are pushed again afterward: iCloud updates the copy
  this app saved, and a Notion section this app wrote is updated in place.
- Display the macOS input language through Text Input Source Services
  notifications, use a 500 ms polling fallback when notification registration
  fails, show `IME: EN` or `IME: KO` for English and Korean, and show `IME: ?`
  for any other or unknown language.
- Show a “server not running” state, with the command to start it, whenever the
  client cannot attach. The client never starts the server; the start command is
  `nd serve`.
- Display dates as `YYYY-MM-DD`, adding the abbreviated weekday as
  `YYYY-MM-DD (Mon)` where the available space and context permit; the ISO date
  stays visible in every form.
- Treat Wave Terminal (waveterm.dev) as the primary real-terminal acceptance
  target and Apple Terminal as the secondary regression target (owner decision,
  2026-10-29). Both passed P1-M7 qualification on 2026-10-29; see the
  [changelog](phase-1/changelog.md) for the findings that drove this choice.

## Input constraint

The person types Korean as well as English. While a Korean IME is active, bare
letters and `:` reach the application as composed text, and `Enter` or `Space`
may commit a composition instead of acting. The decided model therefore uses
focus navigation plus Ctrl chords and no bare-letter or `:` hotkeys, except the
Sources source-list shortcuts above (owner decision, 2026-10-30: only while
`IME: EN`). Terminals
also encode `Ctrl-I` as Tab, `Ctrl-M`
as Enter, and `Ctrl-[` as Esc; macOS reserves `Ctrl-Space` for input-source
switching by default; and Terminal.app sends Option as a character modifier
unless it is set to act as Meta. Avoid those collisions. On non-editor screens
Tab and the arrow keys move focus and Enter activates. The Command Line opens
with Ctrl-G; Ctrl-S saves or flushes the editor; Ctrl-C, pressed twice, quits
(owner decision, 2026-10-29: replaced the single Ctrl-D chord to guard
against an accidental single keystroke ending the session); and Ctrl-P/Ctrl-N
move to the previous/next workflow stage. There are no
function-key aliases: a real-terminal check found Apple Terminal does not
reliably deliver function keys as distinct chords (F10 never arrived at
all), so every action is reachable only by its Ctrl chord or by focus and
Enter. The Command Line accepts ASCII command tokens and maps Hangul typed on
the 2-Set Korean layout back to the key that produced it, so commands work
under either IME; every command is also reachable by focus and Enter.
Real-terminal acceptance checks this with the Korean IME active.

## Current and planned presentation details

Work Status is a non-focusable pane above all three columns. Its four fixed
content rows prioritize current or recent work, pending or blocked work,
connection and draft-save safety, and a useful safe notice or next action. The
bottom Status Line remains the compact diary-day, workflow, connection, IME,
progress, error, and action-hint summary; repeating an important item in both is
intentional. A decision and its controls still belong in the Main Pane Banner.

The Resources Sidebar lists source notes with separate active and included
states. Activating a readable note opens its sanitized text in the Main Pane;
when no body is active, the Main Pane can show a candidate table with inclusion,
note identity, directory date, header date, availability, and warning status in
stable columns. Markers are words or symbols, never color alone. The chosen
diary day stays visible while inspecting another date. An included cloud-only
row offers a download action and progress state; the person confirms the
selection only after its readable snapshot has been reviewed.

The editor should keep backend/path, language, and style choices visible and
allow paste, multiline editing, and review of source-linked memories. The
Resources Sidebar keeps the draft, confirmed sources, and assistant suggestions
reachable. Opening a source exposes Refine, Approve, Reject, Edit, and Delete
controls. Edit reuses the multiline editor; Delete requires an Enter/Esc
confirmation naming the source and linked-audio consequence. Returning to the draft preserves the
unsent buffer and cursor. When an assistant draft completes against the current
buffer version, it becomes active and its full text opens editable in the Main
Pane. A late result remains inspectable but cannot overwrite unsent or newer
text. When a `ko-KR` entry is
drafted from en-US notes, the editor says it is a translation. An inferred
reflection appears inline between reserved delimiters, so it survives any
edit; the person accepts it by removing the delimiters, edits the text, or
removes the span. Entry review refuses confirmation while any reserved delimiter
character remains and points to the first one. A paragraph-review suggestion
shows the current paragraph, proposed replacement, backend label, and explicit
Accept and Reject controls; a stale suggestion is read-only and cannot be
applied. Entry review shows the entire edited text, not a summary. Unsent text is flushed to the server after about a second without
typing, when focus leaves the editor, and before quitting; if the server is
unreachable Work Status and the compact Status Line say so and quitting asks
first. If freshness fails or another client changes the workflow, a Banner
explains what must be reread and recovered edits stay visible without being
treated as confirmed.

The output preview should show the local file path and rendered Markdown,
followed by the Notion destination, `YYYY-MM-DD (DDD)` heading, and exact
ordered native-block projection. It separates read-only preflight results from
actions that create pages or append blocks, allows unrelated hierarchy content,
and marks a blocked destination with its reason. Approval is a deliberate
action on this screen, tied to the displayed plan, and no typed command bypasses it. The
publication screens show the local save and each remote separately and
distinguish success, pending, blocked, and a file that changed. Cancelling a
publication that may already have written leaves the destination reconciling
rather than claiming rollback.
Connection loss disables publication until the client reloads the server's
current workflow.

Colors may reinforce but never carry meaning alone; the client honors
`NO_COLOR` and stays legible on a 16-color terminal. Messages about notes name a
relative path and the problem, never note content.

Ratatui views render the ViewModel; the client sends actions to the Control
API Server and receives state changes through REST and events. The screen
chrome, stage states, terminal-size rules, and connection and error states are
in the [information architecture](ux-information-architecture.md).
