# Narihito's Diary

Make remembering a day feel effortless.

The active Phase 1 MVP is a macOS terminal client that reads Markdown source
notes from a mounted iCloud directory, lets the user choose which dated source notes belong in an
entry even when their directory and header dates disagree, and supports a
selectable set of assistant backends (hosted by default, optional Apple
on-device, and future approved adapters), paragraph suggestions that the user
accepts or rejects, or manual writing in the always-available Draft editor. It saves the confirmed entry as local
Markdown and publishes its body to a Notion daily section under root, year, and
month pages. See the
[product roadmap](docs/roadmap.md).

The repository contains the independently testable workflow core, private
SQLite persistence, bounded Ayumi source ingestion and Swift iCloud bridge,
and P1-M6's secure loopback Control API Server. Start the foreground server
with `cargo run -p nd -- serve`; P1-M12 completed the canonical terminology,
assistant-neutral contract, and M7 lower-layer readiness pass. P1-M7 remains
planned behind its dependency and macOS FFI approval gate.

## License

Licensed under the Apache-2.0 license. See [LICENSE](LICENSE).

## Contributing

Contributors and AI agents: see [AGENTS.md](AGENTS.md) and [docs/contribution-guide.md](docs/contribution-guide.md).
