# UX information architecture

Status: Active

The Standard Layout is the canonical containment model for the terminal client.
It names screens and their content; interaction behavior is in [ux-gui.md](ux-gui.md)
and server ownership is in the [architecture](planned-mvp-architecture.md).

## Standard Layout

```text
Terminal client
├── Work Status (full width above the three columns; four content rows)
├── Three-column region
│   ├── Workflow Sidebar (left)
│   │   ├── Start
│   │   ├── Sources
│   │   ├── Compose
│   │   ├── Review
│   │   ├── Preview
│   │   └── Publish
│   ├── Resources Sidebar (middle)
│   │   └── Contextual source notes, draft, assistant suggestions, or destinations
│   └── Main Pane (right)
│       └── Active stage and active resource: source text, editor, suggestion,
│           complete-entry review, output preview, or publication result
├── Command Line
└── Status Line: diary day, workflow state, connection, IME, progress, errors
```

At wide terminal sizes, the Sources stage reads spatially like this. This is a
containment wireframe, not a final pixel specification:

```text
┌─ Work Status ───────────────────────────────────────────────────────────────┐
│ Preparing 2 selected source notes                                           │
│ Downloading note-b · waiting                                                │
│ connected · draft saved                                                     │
│ Next: review prepared text, then confirm the source set                      │
└─────────────────────────────────────────────────────────────────────────────┘
┌───────────────────┬──────────────────────┬──────────────────────────────────┐
│ Workflow          │ Resources            │ Main Pane                        │
├───────────────────┼──────────────────────┼──────────────────────────────────┤
│ done Start        │ [x] note-a           │ Source: note-b                   │
│ > Sources         │ > [x] note-b         │                                  │
│ available Compose │ [ ] note-c  warning  │ Read-only sanitized source text  │
│ locked Review     │                      │                                  │
│ locked Preview    │ Include  Exclude     │                                  │
│ locked Publish    │ Prepare  Confirm     │                                  │
├───────────────────┴──────────────────────┴──────────────────────────────────┤
│ Command Line                                                                │
├─────────────────────────────────────────────────────────────────────────────┤
│ Status Line: 2026-10-29 · Sources · connected · IME: EN                     │
└─────────────────────────────────────────────────────────────────────────────┘
```

Work Status spans the full width above the three columns. Its bordered body has
exactly four content rows, excluding its title and borders, and does not take
focus. It tells the person what work is current or just completed, what is
pending or blocked, whether the connection and draft are safe, and the newest
useful message or next action. Each priority uses one visual row, truncated by
display width, and empty priorities stay blank. Its messages are safe
presentation text, never diary content or arbitrary server errors. The bottom
Status Line remains a compact ambient summary, and important state may appear
in both places intentionally.

`>` is the active resource shown in the Main Pane; `[x]` is inclusion. They are
independent. In Compose, the Resources Sidebar lists `Draft`, the confirmed
sources, and any assistant suggestions, while the right Main Pane becomes the
editable draft.

The stage labels and order are canonical in [UX terms](ux-terms.md#canonical-workflow-stages).
The Sources stage may return to day choice or add one explicitly requested
date. After a source set is confirmed, the person can use an available assistant
or go straight to the editor in Compose. Every route to output passes through
Review and Preview. Source change detection returns the workflow to Sources and
invalidates later confirmations and previews; recovered editor text remains
visible but unconfirmed. An amendment skips Sources: it enters Compose with the
saved text, and the Workflow Sidebar marks Sources done from the receipt's
digests.

Activating and including a source are separate actions. Activating any readable
source in the Resources Sidebar shows its text in the Main Pane without changing
the included set. An explicit include/exclude control changes the prospective
set. After confirmation, only those confirmed source snapshots are eligible for
assistant work. Choosing **Create draft** sends the allowed text envelope from
the Control API Server to exactly one selected assistant backend; merely
highlighting, opening, or including a source does not call an assistant.

In Compose, the Resources Sidebar keeps the draft and confirmed sources
reachable. Activating a source temporarily replaces the editor with a read-only
source view in the Main Pane; returning to the draft restores the editor and its
cursor without losing unsaved text. When a draft job finishes, the draft becomes
active and its text opens in the editable Main Pane. Assistant suggestions,
including paragraph reviews, appear as resources with explicit accept and
reject actions. Accepting
a suggestion edits the draft only if it still targets the current buffer;
rejecting it never changes the entry.

Dates use `YYYY-MM-DD`. Where space and context permit, add the abbreviated
weekday as `YYYY-MM-DD (Mon)`; the ISO date remains present so date evidence
and diary-day identity are never ambiguous.

Stages represent one server-owned workflow revision. ViewModels hold only
presentation state and unsent edits; the Control API Server owns confirmed
selections, drafts, plans, and receipts. A reconnect fetches the latest server
state before allowing another mutation. The [UX terms](ux-terms.md) name the
components used above. The server's voiding rules are in the
[workflow state table](planned-mvp-architecture.md#workflow-states-and-invalidation).

## Navigation and stage states

The Workflow Sidebar lists the six canonical stages. Each has one progress
state, and exactly one accessible stage also has the `current` marker because it
is open in the Main Pane:

| Progress state | Meaning | Can the person open it? |
|---|---|---|
| done | Confirmed and still valid. | Yes. |
| available | Its prerequisites are confirmed; not yet done. | Yes. |
| locked | A prerequisite is unconfirmed, so it cannot be reached. | No; the Status Line names the missing step. |
| stale | Was confirmed, then voided by a later change. The reason is shown. | Yes, to reconfirm. |

`current` is navigation state, not a fifth progress state. It may appear beside
`done`, `available`, or `stale`; moving it changes only the active Main Pane.

Optional assistant actions do not add or remove sidebar stages. Opening an
earlier stage is navigation and changes nothing. The first edit there that
would void later confirmations
shows an impact notice listing what becomes unconfirmed, and applies only after
the person continues. Text already typed is kept as unconfirmed.

## Start stage and connection states

The server is started by hand, so the client may open before it exists.

| Connection state | What the screen shows |
|---|---|
| not running | The start command, `nd serve`, and retry while waiting. Nothing else is enabled. |
| connecting | A progress indicator; no workflow actions. |
| connected | The Start stage with Setup check and unfinished drafts. |
| reconnecting | Existing screens stay readable; every mutation is disabled until the latest workflow state is fetched. |
| version mismatch | Which side is older and the command to restart the server. |

## Empty and error states

These conditions are distinct and are never collapsed into “no entries”.

| Condition | Message direction |
|---|---|
| iCloud Drive root absent | iCloud Drive is unavailable or not configured. |
| Configured Ayumi directory absent | The directory name is wrong or not synchronized. |
| Permission denied | The terminal that started the server needs macOS privacy access. |
| Note not downloaded | Download needed; offered for selected notes only. |
| Invalid metadata or front matter | The relative path and the problem, never the content. |
| Invalid UTF-8 | Warning; the note has no readable body. |
| No notes in the three-day window | Nothing found for the window, with the explicit extra-date choice and “continue without notes”. |
| Assistant backend unavailable (hosted provider unconfigured or unreachable, Apple model unavailable, unsupported locale, or another adapter-specific safe reason) | The Draft editor and every other currently available backend are offered. |
| Notion unconfigured, unreachable, or failed preflight | Shown as blocked in the preview; the local entry can still be approved. |

## Chrome layout

The Standard Layout above is content, not pixels. The proposed screen chrome
maps it onto six zones, named in [UX terms](ux-terms.md#canonical-chrome-terms):
a full-width Work Status pane above a left Workflow Sidebar, middle Resources
Sidebar, and right Main Pane, followed by a bottom Command Line and bottom-most
Status Line.

- **Work Status** is the full-width, bordered top pane. Its body is exactly four
  rows excluding title and borders. It presents current/recent work, pending or
  blocked work, connection and save safety, and one useful notice or next
  action. It is read-only and non-focusable. A condition requiring a decision
  still uses a Banner and focused controls in the Main Pane.
- **Workflow Sidebar** is the left sidebar zone and always represents the canonical
  `Start`, `Sources`, `Compose`, `Review`, `Preview`, and `Publish` sequence. It
  marks each stage `done`, `available`, `locked`, or `stale`, plus exactly one
  accessible stage as `current`, and supports jump-to-stage navigation where
  the state permits it. It does not
  become a menu of screens or assistant-specific steps.
- **Resources Sidebar** is the middle sidebar zone and presents compact, stateful
  items relevant to the active
  stage. During Sources it is the source list: active focus and inclusion are
  distinct, and activating a readable note shows its text in the Main Pane.
  During Compose it lists the draft, confirmed source notes, memories, and
  assistant suggestions; the draft remains the editable resource and source
  text is read-only. Later stages use the same region for entry review state and
  output resources such as the local file and Notion section.
  A resource can be `ready`, `active`, `included`, `preparing`, `unavailable`,
  `warning`, `blocked`, `conflict`, or `stale`; words or symbols convey state
  without relying on color. A hierarchy such as local output
  `year / month / entry` or
  the Notion `root / year / month / section` is rendered as an indented tree.
  The Main Pane may show a full candidate table when no source body is active so
  both dates, availability, and warnings have stable columns.
- **Main Pane** is the right content zone and renders the active stage and active
  resource: source text, assistant choice or progress, the draft editor, an
  assistant suggestion, complete-entry review, output preview, or publication
  status. A Banner at its top holds any condition that needs a decision, with
  the explanation.
- **Command Line** accepts only the bounded application grammar `help`, `retry`,
  `day YYYY-MM-DD`, `stage <stage>`, `save`, and `quit`; it is not a shell.
  Selection confirmation, complete-entry confirmation, approval, and
  publication are focused on-screen actions, so a typed command cannot bypass
  their review screens.
- **Status Line** is the compact realization of the "App status" and
  "Action/status area" content named in the Standard Layout tree above: diary
  day, workflow state, connection, a short error summary, progress, IME
  indicator, and action hints, in that priority order. When it is too narrow,
  items drop from the end of that list. Work Status may repeat important items;
  details and controls for a decision belong in the Banner, not either status
  region.

### Terminal size and text width

The layout needs at least 80 columns by 30 rows. The six added rows preserve
four Work Status content rows plus its top/title and bottom borders without
taking rows from the three-column region. At 120 columns or wider, both
sidebars are visible. From 100 through 119 columns, the Resources Sidebar
collapses and opens over the Main Pane on request while the Workflow Sidebar
remains visible. Below 100 columns, either sidebar opens over the Main Pane on
request; focus controls expose both sidebars' state and actions. Below the
minimum, the client shows only a “terminal too small” notice with the required
size.
Korean text is double-width: measure display width rather than character
count, wrap at grapheme boundaries, and keep the editor cursor aligned.

Keys follow “Keybindings” in [AGENTS.md](../AGENTS.md): focus navigation plus
Ctrl chords, with details in [ux-gui.md](ux-gui.md#input-constraint).
