# Narihito's Diary

Make remembering a day feel effortless.

The active Phase 1 MVP is a macOS terminal client that reads Markdown source
notes from a mounted iCloud directory, lets the user choose which dated source
notes belong in an entry, and composes it with a selectable assistant backend,
paragraph suggestions, or manual writing in an always-available Draft editor.
It saves the confirmed entry as local Markdown and publishes it to a Notion
daily section. See the [product roadmap](docs/roadmap.md) and
[product behavior](docs/product-behavior.md) for the full workflow and its
boundaries.

## Status

P1-M7 (manual Start → Sources → Compose → Review) is done; P1-M8 (assistant
backends) is active, with no production assistant backend live yet. Preview
and Publish remain visibly locked until their later milestones.

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
