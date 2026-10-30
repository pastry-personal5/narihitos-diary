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

## Status

P1-M8 is done and adds optional Codex hosted and Apple on-device assistants for
memory extraction, drafting, paragraph review, and source refinement. Codex profiles are
configured explicitly and each call
uses a fresh confined `codex app-server` process; Apple assistance is available
only when Foundation Models reports ready. Manual editing remains available.
P1-M9 (local publication and recovery) is done. Preview displays the
exact local Markdown file for approval; Publish writes the local file without
replacing different content. Notion publication remains blocked until P1-M10.

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
