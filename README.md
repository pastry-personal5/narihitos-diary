# Narihito's Diary

Make remembering a day feel effortless.

The active Phase 1 MVP is a macOS TUI that reads Markdown files from a mounted
iCloud directory, lets the user choose which dated source notes belong in an
entry even when their directory and header dates disagree, and supports hosted
drafting, optional on-device drafting, or manual writing. It saves the confirmed entry as local
Markdown and publishes its body to a Notion daily section under root, year,
and month pages. See the
[product roadmap](docs/roadmap.md).

The repository contains the independently testable workflow core, private
SQLite persistence, bounded Ayumi source ingestion and Swift iCloud bridge,
and P1-M6's secure loopback Control API Server. Start the foreground server
with `cargo run -p nd -- serve`; the default `nd` TUI mode remains unfinished
until P1-M7.

## License

Licensed under the Apache-2.0 license. See [LICENSE](LICENSE).

## Contributing

Contributors and AI agents: see [AGENTS.md](AGENTS.md) and [docs/contribution-guide.md](docs/contribution-guide.md).
