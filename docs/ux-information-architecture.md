# UX information architecture

Status: Proposal

The Standard Layout is the proposed containment model for the terminal client.
It names screens and their content; interaction behavior is in [ux-gui.md](ux-gui.md)
and server ownership is in the [architecture](planned-mvp-architecture.md).

## Standard Layout

```text
Terminal client
├── App status: diary day, workflow state, connection, IME indicator
├── Main screen
│   ├── Start or resume
│   │   ├── Server connection and Setup check
│   │   ├── Unfinished drafts (listed first), with discard; completed days offer Amend
│   │   └── Diary day choice (default: today, Mac local time)
│   ├── Source selection
│   │   ├── Day choice, explicit extra-date choice, and “continue without notes”
│   │   ├── Candidate table: inclusion, date evidence, availability, warnings
│   │   └── Source detail and selected-only download state
│   ├── Memory review (optional)
│   ├── Draft editor: language, style, source context, inferred reflections
│   ├── Entry review: complete edited text and confirmation
│   ├── Output preview: local file and Notion section, paths, planned changes,
│   │   blocked destinations, and approval
│   └── Publication status: per-destination result, retry state, Conflict view
└── Action/status area: available actions, progress, errors
```

The source selection screen may return to day choice or add one explicitly
requested date. After a source snapshot is confirmed, the person can proceed
through memory review or go straight to the draft editor. Every route to
output passes through entry review and output preview. Source change detection
returns the workflow to source review and invalidates later previews; recovered
editor text remains visible but unconfirmed. An amendment skips source review:
it enters at the Draft editor with the saved text, and the Navigator marks the
source steps done from the receipt's digests.

Dates use `YYYY-MM-DD`. Where space and context permit, add the abbreviated
weekday as `YYYY-MM-DD (Mon)`; the ISO date remains present so date evidence
and diary-day identity are never ambiguous.

Screens represent one server-owned workflow revision. ViewModels hold only
presentation state and unsent edits; the Control API Server owns confirmed
selections, drafts, plans, and receipts. A reconnect fetches the latest server
state before allowing another mutation. The [UX terms](ux-terms.md) name the
components used above. The server's voiding rules are in the
[workflow state table](planned-mvp-architecture.md#workflow-states-and-invalidation).

## Navigation and step states

The Navigator lists the screens above and marks each with a step state:

| Step state | Meaning | Can the person open it? |
|---|---|---|
| done | Confirmed and still valid. | Yes. |
| current | Where the workflow is now. | Yes. |
| available | Its prerequisites are confirmed; not yet done. | Yes. |
| locked | A prerequisite is unconfirmed, so it cannot be reached. | No; the Status Line names the missing step. |
| stale | Was confirmed, then voided by a later change. The reason is shown. | Yes, to reconfirm. |

Memory review is marked optional. Opening an earlier screen is navigation and
changes nothing. The first edit there that would void later confirmations
shows an impact notice listing what becomes unconfirmed, and applies only after
the person continues. Text already typed is kept as unconfirmed.

## Start screen and connection states

The server is started by hand, so the client may open before it exists.

| Connection state | What the screen shows |
|---|---|
| not running | The start command, `nd serve`, and retry while waiting. Nothing else is enabled. |
| connecting | A progress indicator; no workflow actions. |
| connected | The Start screen with Setup check and unfinished drafts. |
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
| Composition path unavailable (hosted provider unconfigured or unreachable, Apple model unavailable, or locale unsupported) | The manual editor is offered, and the other path if it is available. |
| Notion unconfigured, unreachable, or failed preflight | Shown as blocked in the preview; the local entry can still be approved. |

## Chrome layout

The Standard Layout above is content, not pixels. The proposed screen chrome
maps it onto five zones, named in [UX terms](ux-terms.md#proposed-chrome-terms):
a left Workflow Sidebar, a central Main Pane, a right Resources Sidebar, a
bottom Command Line, and a bottom-most Status Line.

- **Main Pane** renders the active screen's content: the candidate table and
  source detail, memory review, draft editor, entry review, output preview, or
  publication status. A Banner at its top holds any condition that needs a
  decision, with the explanation.
- **Workflow Sidebar** is always the Navigator. It lists the Standard Layout's
  stages with their `done`, `current`, `available`, `locked`, or `stale` state
  and supports jump-to-step navigation where the state permits it. It does not
  change mode, so a person can always see where the active screen sits in the
  workflow.
- **Resources Sidebar** presents compact, stateful items relevant to the active
  screen. It is a source list during selection and editing (inclusion mark,
  short identity, date-evidence disagreement, availability, and warning or
  download state); a source detail can expose the selected item's metadata.
  Later screens use the same region for reviewed memories, the entry buffer and
  review state, and output resources such as the local file and Notion section.
  A resource can be `ready`, `selected`, `preparing`, `unavailable`, `warning`,
  `blocked`, `conflict`, or `stale`; words or symbols convey state without
  relying on color. A hierarchy such as local output `year / month / entry` or
  the Notion `root / year / month / section` is rendered as an indented tree.
  The full candidate table remains in the Main Pane so both dates, availability,
  and warnings have stable columns.
- **Command Line** accepts typed commands (for example: jump to a diary day or
  confirm). Approval and publication are on-screen actions
  at the output preview; a typed command cannot bypass that screen.
- **Status Line** is the single realization of both the "App status" and
  "Action/status area" content named in the Standard Layout tree above: diary
  day, workflow state, connection, a short error summary, progress, IME
  indicator, and action hints, in that priority order. When it is too narrow,
  items drop from the end of that list. Details of an error belong in the
  Banner, not the Status Line.

### Terminal size and text width

The layout needs at least 80 columns by 24 rows. At 120 columns or wider, both
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
