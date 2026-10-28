# Narihito's Diary

Make remembering a day feel effortless.

The active Phase 1 MVP is a macOS TUI that reads Markdown files from a mounted
iCloud directory, lets the user choose which dated source notes belong in an
entry even when their directory and header dates disagree, and supports hosted
drafting, optional on-device drafting, or manual writing. It saves the confirmed entry as local
Markdown and publishes its body to a Notion daily section under root, year,
and month pages. See the
[product roadmap](docs/roadmap.md).

The repository currently contains the P1-M3 project scaffold. The `nd` and
`nd serve` commands deliberately report that their modes are not implemented;
product workflow work begins in later milestones.

## License

Licensed under the Apache-2.0 license. See [LICENSE](LICENSE).

## Contributing

Contributors and AI agents: see [AGENTS.md](AGENTS.md) and [docs/contribution-guide.md](docs/contribution-guide.md).
