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

Inspect the command boundary and start the server from the repository root.
`nd serve` validates the owner-only configuration, private state and diagnostic
directories, binds only numeric IPv4 loopback using the documented bounded
port sequence, and publishes an ephemeral attachment file only after its
health check succeeds. The default `nd` TUI mode still exits with the explicit
not-implemented diagnostic until P1-M7.

```sh
cargo run -p nd -- --help
cargo run -p nd
cargo run -p nd -- serve
cargo test -p nd --test command_shells
cargo test -p nd --test p1_m6_process
printf '%s\n' '{"protocol_version":1,"request_id":"smoke-1","command":"ping","payload":{}}' | swift run --package-path helpers/apple-bridge apple-bridge
```

The helper must emit exactly one JSON response line on standard output. The
Rust helper client uses `ND_APPLE_BRIDGE_PATH` when set; otherwise it checks the
installed `../libexec/apple-bridge` path relative to `nd`, then
`helpers/apple-bridge/.build/debug/apple-bridge` in a development checkout.
Source tests use synthetic trees and fake helper services or executables; they
must not read a contributor's live iCloud tree.

The optional `server.port_fallback_count` configuration value defaults to `8`
and may be `0` through `16`. The server always tries `8771` and `8772` first,
then that many additional consecutive ports. An unavailable Apple helper or
unconfigured Notion destination is reported as a disabled capability and does
not prevent the explicit continue-without-notes action and manual editing in
the Draft editor.

Content-free `ndlog-v1` diagnostics are stored in the Git-ignored
`.local/logs/control-api/` directory with owner-only permissions. Contributors,
Codex, and Claude Code may use bounded read-only inspection without a Control
API token:

```sh
tail -n 200 .local/logs/control-api/control-api.log
rg 'level=(WARN|ERROR)' .local/logs/control-api/
```

Do not change log permissions or inspect the separate Application Support
runtime directory, which contains the ephemeral control token. Logs must never
contain diary text, source metadata or paths, credentials, raw request data, or
arbitrary error strings.

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
