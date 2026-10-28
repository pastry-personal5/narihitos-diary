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
│   ├── Source selection
│   │   ├── Day choice and explicit extra-date choice
│   │   ├── Candidate list: inclusion, date evidence, availability, warnings
│   │   └── Source detail and selected-only download state
│   ├── Memory review (optional)
│   ├── Draft editor: language, style, source context, inferred reflections
│   ├── Entry review: complete edited text and confirmation
│   ├── Output preview: local file and Notion section, paths, planned changes
│   └── Publication status: per-destination result and retry state
└── Action/status area: available actions, progress, errors
```

The source selection screen may return to day choice or add one explicitly
requested date. After a source snapshot is confirmed, the person can proceed
through memory review or go straight to the draft editor. Every route to
output passes through entry review and output preview. Source change detection
returns the workflow to source review and invalidates later previews; recovered
editor text remains visible but unconfirmed.

Screens represent one server-owned workflow revision. ViewModels hold only
presentation state and unsent edits; the Control API Server owns confirmed
selections, drafts, plans, and receipts. A reconnect fetches the latest server
state before allowing another mutation. The [UX terms](ux-terms.md) name the
components used above.

## Chrome layout

The Standard Layout above is content, not pixels. The proposed screen chrome
maps it onto four zones, named in [UX terms](ux-terms.md#proposed-chrome-terms):
a left Sidebar, a right Main Pane, a bottom Command Line, and a bottom-most
Status Line.

- **Main Pane** renders the active screen's content: source detail, memory
  review, draft editor, entry review, output preview, or publication status.
- **Sidebar** supplements the Main Pane in one of two modes: Navigator mode
  lists the Standard Layout's screens for jump-to-step navigation; Source List
  mode lists the active day's source-note candidates (inclusion, date
  evidence, availability, warnings) for inclusion and detail selection.
- **Command Line** accepts typed commands (for example: jump to a diary day,
  confirm, publish, switch Sidebar mode).
- **Status Line** is the single realization of both the "App status" and
  "Action/status area" content named in the Standard Layout tree above:
  diary day, workflow state, connection, IME indicator, available actions,
  progress, and errors all appear there rather than in separate bars.

Sidebar-mode and Command Line keybindings remain open implementation
details.
