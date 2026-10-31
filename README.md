# Narihito's Diary

Make remembering a day feel effortless.

The active Phase 1 MVP is a macOS terminal client that reads Markdown source
notes from a mounted iCloud directory, lets the user inspect, refine, edit, or
explicitly delete an Ayumi source and its linked audio, choose which dated source
notes belong in an entry, and compose it with a selectable assistant backend,
paragraph suggestions, or manual writing in an always-available Draft editor.
It saves the confirmed entry as local Markdown and publishes it to a Notion
daily section. See the [product roadmap](docs/roadmap.md) and
[product behavior](docs/product-behavior.md) for the full workflow and its
boundaries.

The Sources stage discovers nearby notes when opened. Its flat menu lets you
include or exclude notes, optionally refine one, and confirm the list without
opening every note. Source-list letter shortcuts are available while the
Resources Sidebar is focused and the input indicator reads `IME: EN`; `?`
shows the shortcut help.

The TUI carries language and writing style to new diary days during the
session. It saves the assistant choice, including manual mode, and restores it
for new days after a restart; reopening an existing day keeps that day's own
settings.

## Status

P1-M8 is done and adds optional Codex hosted and Apple on-device assistants for
memory extraction, drafting, paragraph review, and source refinement. Codex profiles are
configured explicitly and each call
uses a fresh confined `codex app-server` process; Apple assistance is available
only when Foundation Models reports ready. Manual editing remains available.
P1-M9 (local publication and recovery) is done. P1-M10 is in progress: after
the local file is verified, Publish Remote can inspect the configured Notion
`Root → YYYY → YYYY-MM` path and show the exact native-block projection; Push
creates only missing canonical pages, adds the
daily section in bounded chunks, and reads it back before completing. The live
configured-root compatibility check and disposable-hierarchy acceptance remain
owner-authorized gates; synthetic tests never access a contributor's Notion.

The workflow ends with **Preview** (the local file only), **Publish Local**
(saving it completes the day), and **Publish Remote**, where iCloud and Notion
are listed and each is pushed on request. Nothing is sent to a remote
automatically, and an entry already there is shown and resolved with Append or
Overwrite.

A completed day can be amended from its Publish Local stage: the editor opens
with the saved text, and approving the replacement plan replaces only the local
file and Notion section the app itself wrote (a remote is never changed by an
amendment; push again to update iCloud).

Set the optional `paths.icloud_entries_root` (under
`~/Library/Mobile Documents/`) to make iCloud a remote you can push an identical
Markdown file to; see `config/global_config.example.yaml`.

## Quick start

```sh
cargo run -p nd -- serve   # start the server (its own terminal)
cargo run -p nd            # open the terminal client
```

The terminal needs at least 80 columns by 30 rows; see
[docs/ux-tui.md](docs/ux-tui.md) for terminal and IME details. Setup and the
exact validation commands are in the [contribution
guide](docs/contribution-guide.md).

## License

Licensed under the Apache-2.0 license. See [LICENSE](LICENSE).

## Contributing

Contributors and AI agents: see [AGENTS.md](AGENTS.md) and [docs/contribution-guide.md](docs/contribution-guide.md).
