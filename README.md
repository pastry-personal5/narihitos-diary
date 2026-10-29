# Narihito's Diary

Make remembering a day feel effortless.

The active Phase 1 MVP is a macOS terminal client that reads Markdown source
notes from a mounted iCloud directory, lets the user choose which dated source
notes belong in an entry even when their directory and header dates disagree,
and supports a
selectable set of assistant backends (hosted by default, optional Apple
on-device, and future approved adapters), paragraph suggestions that the user
accepts or rejects, or manual writing in the always-available Draft editor. It
saves the confirmed entry as local Markdown and publishes its body to a Notion
daily section under root, year, and month pages. See the [product
roadmap](docs/roadmap.md).

The repository contains the independently testable workflow core, private
SQLite persistence, bounded Ayumi source ingestion and Swift iCloud bridge,
and P1-M6's secure loopback Control API Server. Start the foreground server
with `cargo run -p nd -- serve`, then open the terminal client with
`cargo run -p nd`. P1-M7 is done: the client can attach, start or resume a
workflow, select and inspect sources, write in the recoverable Draft editor,
and confirm the complete entry. Server startup reports actionable,
content-free guidance for missing, unsafe, malformed, or invalid source
configuration. The terminal needs at least 80 columns by 30 rows. Its Status
Line follows TIS input-language notifications with a bounded 500 ms query
fallback, and the Workflow Sidebar stays at its minimum content-safe width so
the Main Pane retains the remaining space. Preview and Publish remain visibly
locked until their later milestones. Wave Terminal primary qualification and
the secondary Apple Terminal regression set passed on 2026-10-29. Setup and
the exact validation commands are in the [contribution
guide](docs/contribution-guide.md).

## License

Licensed under the Apache-2.0 license. See [LICENSE](LICENSE).

## Contributing

Contributors and AI agents: see [AGENTS.md](AGENTS.md) and [docs/contribution-guide.md](docs/contribution-guide.md).
