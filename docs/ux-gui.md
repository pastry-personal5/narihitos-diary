# Terminal UX

Status: Proposal

This document separates approved terminal behavior from proposed presentation
details. Product scope is in [product-behavior.md](product-behavior.md), and
component vocabulary is in [ux-terms.md](ux-terms.md).

## Current interface

There is no production TUI yet. This document describes the intended client
experience; it does not claim an implemented screen or keybinding.

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
- Offer US English and South Korean Korean per entry, remember the last choice,
  and offer concise, standard, and reflective styles with concise as default.
  The choice names the language of the finished entry; source notes are
  expected to be en-US. Mark assistant-inferred reflections for human review.
- Require confirmation of the complete edited entry and then show the exact
  local and Notion outputs before the first write. Display the root/year/month
  path and any subpages that would be created. Show conflicts and partial
  success plainly.
- When the local file differs, stop the whole plan; when Notion holds
  conflicting same-day content, stop Notion only. Show both versions and offer
  only reread, discard the entry, or save under a different diary day, as
  described in [Publication and recovery](planned-mvp-architecture.md#publication-and-recovery).
  Never overwrite.
- When Notion is unconfigured, unreachable, or fails its preflight, show it as
  blocked in the preview and still allow the local entry to be approved and
  written. Notion can be retried later: a blocked destination needs a new
  Notion-only preview and approval, and a pending one reuses the approval only
  while its plan is unchanged.
- Offer to amend a completed day. The editor opens with the saved text and, if
  the file was hand-edited, a banner saying it changed since publication. The
  output preview shows current and replacement content per destination, and a
  destination that no longer matches our receipt is a Conflict.
- Display the macOS input language through Text Input Source Services
  notifications, use a 500 ms polling fallback when notification registration
  fails, show `IME: EN` or `IME: KO` for English and Korean, and show `IME: ?`
  for any other or unknown language.
- Show a “server not running” state, with the command to start it, whenever the
  client cannot attach. The client never starts the server; the start command is
  `nd serve`.

## Input constraint

The person types Korean as well as English. While a Korean IME is active, bare
letters and `:` reach the application as composed text, and `Enter` or `Space`
may commit a composition instead of acting. The decided model (“Keybindings” in
[AGENTS.md](../AGENTS.md)) therefore uses focus navigation plus Ctrl chords and
no bare-letter or `:` hotkeys. Terminals also encode `Ctrl-I` as Tab, `Ctrl-M`
as Enter, and `Ctrl-[` as Esc; macOS reserves `Ctrl-Space` for input-source
switching by default; and Terminal.app sends Option as a character modifier
unless it is set to act as Meta. Avoid those collisions. On non-editor screens
Tab and the arrow keys move focus and Enter activates. The Command Line opens
with Ctrl-G; Ctrl-S saves or flushes the editor; Ctrl-D quits; and
Ctrl-P/Ctrl-N move to the previous/next workflow stage. There are no
function-key aliases: a real-terminal check found Apple Terminal does not
reliably deliver function keys as distinct chords (F10 never arrived at
all), so every action is reachable only by its Ctrl chord or by focus and
Enter. The Command Line accepts ASCII command tokens and maps Hangul typed on
the 2-Set Korean layout back to the key that produced it, so commands work
under either IME; every command is also reachable by focus and Enter.
Real-terminal acceptance checks this with the Korean IME active.

## Proposed presentation details

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
reachable. Opening a source is read-only; returning to the draft preserves the
unsent buffer and cursor. When an assistant draft completes, the draft becomes
active and its full text opens in the editable Main Pane. When a `ko-KR` entry is
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
followed by the Notion destination, heading, and exact body. It separates
read-only preflight results from actions that create pages or append text, and
marks a blocked destination with its reason. Approval is a deliberate action on
this screen, tied to the displayed plan, and no typed command bypasses it. The
publication screen should distinguish local success, Notion pending, Notion
blocked, verified success, and a conflict. The Conflict view lays the existing
content beside the planned payload and offers the three resolutions above.
Connection loss disables publication until the client reloads the server's
current workflow.

Colors may reinforce but never carry meaning alone; the client honors
`NO_COLOR` and stays legible on a 16-color terminal. Messages about notes name a
relative path and the problem, never note content.

Ratatui views render the ViewModel; the client sends actions to the Control
API Server and receives state changes through REST and events. The screen
chrome, stage states, terminal-size rules, and connection and error states are
in the [information architecture](ux-information-architecture.md).
