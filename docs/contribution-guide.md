# Contribution Guide

Status: Active

How to set up `narihitos-diary`, validate a change, write commit messages, and open a pull request.

## Setup and development

Prerequisites:

- Rust 1.89 or newer with Cargo, rustfmt, and Clippy. The workspace uses Rust
  edition 2024.
- Xcode command-line tools with Swift 6.2 or newer on macOS 26.0 or newer.
- The P1-M4 SQLite adapter uses `rusqlite` 0.40.2 with its default system
  linkage. Cargo downloads the Rust crates; macOS supplies SQLite, so no
  separately installed database service or bundled C build is required.
- The Swift helper still has no third-party package dependency.

The real configuration is repository-local, ignored by Git, and must have no
group or world permissions. Mode `600` is the recommended setup:

```sh
cp config/global_config.example.yaml config/global_config.yaml
chmod 600 config/global_config.yaml
```

Replace every placeholder before using a later server implementation. The
Ayumi value is one directory name beneath iCloud Drive, not an absolute path.
By default the server will resolve `config/global_config.yaml` from its working
directory. `ND_CONFIG_PATH` selects a different file and takes precedence.

Build both language workspaces from the repository root:

```sh
cargo build --workspace
swift build --package-path helpers/apple-bridge
```

Inspect and smoke-test the command shells. The two mode commands currently
exit with status 1 and an explicit not-implemented diagnostic. P1-M4 adds the
inward workflow and test-only SQLite assembly; P1-M5 adds bounded Ayumi source
adapters and the iCloud helper commands. The production server and TUI remain
unassembled until P1-M6 and P1-M7.

```sh
cargo run -p nd -- --help
cargo run -p nd
cargo run -p nd -- serve
cargo test -p nd --test command_shells
printf '%s\n' '{"protocol_version":1,"request_id":"smoke-1","command":"ping","payload":{}}' | swift run --package-path helpers/apple-bridge apple-bridge
```

The helper must emit exactly one JSON response line on standard output. The
Rust helper client uses `ND_APPLE_BRIDGE_PATH` when set; otherwise it checks the
installed `../libexec/apple-bridge` path relative to `nd`, then
`helpers/apple-bridge/.build/debug/apple-bridge` in a development checkout.
Source tests use synthetic trees and fake helper services or executables; they
must not read a contributor's live iCloud tree.

## Required validation

Before opening a pull request, run the full checks from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
swift build --package-path helpers/apple-bridge
swift test --package-path helpers/apple-bridge
```

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org):
`<type>[(scope)]: <imperative summary>`. Keep the lowercase title at most 50
characters with no period. Use a short body only to explain why.

Keep each commit focused. Example:

    feat(parser): add support for nested lists

    Nested list items were flattened into a single level. Keep the nesting
    so the parsed document matches the source.

## Pull requests

TBD
