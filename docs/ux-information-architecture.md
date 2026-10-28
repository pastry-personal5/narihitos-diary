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
