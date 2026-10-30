# Contribution Guide

Status: Active

How to set up `narihitos-diary`, validate a change, write commit messages, and open a pull request.

## Setup and development

Prerequisites:

- Apple Silicon Mac running macOS 26.0 or newer.
- Rust 1.89 or newer with Cargo, rustfmt, and Clippy. The workspace uses Rust
  edition 2024.
- Xcode command-line tools with Swift 6.2 or newer.
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

Replace every placeholder before starting the server. The Ayumi value is one
directory name beneath iCloud Drive, not an absolute path. The optional
`assistants.codex_profiles` collection accepts zero or more records containing
a safe alias, an absolute owner-controlled `codex_home`, and an optional
absolute `codex_executable`. Each valid record becomes a separate hosted
backend; a runtime readiness failure disables that descriptor without stopping
manual editing or other backends.
The Notion root value accepts the page's 32 hexadecimal identifier with or
without UUID hyphens and is normalized internally. The token and root stay in
this owner-only file: they are never returned by the Control API, written to
SQLite, or included in diagnostics. A configured connector is not proof that
the hierarchy is compatible; the Notion-only Preview performs the paginated,
read-only check after local publication is verified.
By default the server will resolve `config/global_config.yaml` from its working
directory. `ND_CONFIG_PATH` selects a different file and takes precedence.

Build both language workspaces from the repository root:

```sh
cargo build --workspace
swift build --package-path helpers/apple-bridge
# Equivalent convenience wrapper:
make build
```

Inspect the command boundary and start the server from the repository root.
`nd serve` validates the owner-only configuration, private state and diagnostic
directories, binds only numeric IPv4 loopback using the documented bounded
port sequence, and publishes an ephemeral attachment file only after its
health check succeeds. Run it in one terminal, then open the TUI from a second
terminal. The implemented client supports Start, Sources, manual or assisted
Compose, complete-entry Review, local Preview/Publish, and the local-receipt-
gated Notion-only Preview/Publish flow. The TUI needs at least 80 columns by 30
rows and
never starts the server itself.

```sh
cargo run -p nd -- --help
cargo run -p nd
cargo run -p nd -- serve
cargo test -p nd --test command_shells --quiet
cargo test -p nd --test control_api_process --quiet
cargo test -p nd --test tui_attachment --test tui_worker --quiet
printf '%s\n' '{"protocol_version":1,"request_id":"smoke-1","command":"ping","payload":{}}' | swift run --package-path helpers/apple-bridge apple-bridge
```

P1-M7 uses Tab/arrows plus Enter and the Ctrl-G, Ctrl-S, Ctrl-C, Ctrl-P, and
Ctrl-N chords, with no function-key aliases. Its bounded Command Line grammar
is `help`, `retry`, `day YYYY-MM-DD`, `stage <stage>`, `save`, and `quit`.
Confirming a selection or entry is deliberately available only through focused
controls and Enter. Wave Terminal is the primary interactive target and Apple
Terminal is the secondary regression target; both completed P1-M7
qualification on 2026-10-29. The probe commands and acceptance evidence are
recorded in the [P1-M7
overview](phase-1/milestone-07-tui-manual-workflow-overview.md).

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

By default the log only records `INFO` and above (one line per HTTP request).
Set `ND_LOG_LEVEL=debug` before `nd serve` to also record job lifecycle
(`component=job`, fields `kind`, accepted/running/completed/failed/cancelled,
plus `reason` — a fixed error code such as `assistant-backend-unavailable` or
`assistant-timeout`, present only on `event=failed`) and preparation-handle
lifecycle (`component=preparation`,
issued/consumed/expired/binding-mismatch) — useful for diagnosing a rejected
`preparation-expired`/`preparation-binding-changed` request. Accepted values
are `trace`, `debug`, `info` (default), `warn`, `error`, case-insensitive; an
unrecognized value falls back to `info`.

```sh
ND_LOG_LEVEL=debug nd serve
tail -n 200 .local/logs/control-api/control-api.log
rg 'component=(job|preparation)' .local/logs/control-api/
```

## Required validation

Before opening a pull request, run the full checks from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --quiet
swift build --package-path helpers/apple-bridge
swift test --package-path helpers/apple-bridge
```

`make check` runs those five commands in the same order. Other phony aliases
include `make run`, `make serve`, `make fmt`, `make fmt-check`, `make clippy`,
`make test`, `make rust-test`, `make swift-build`, and `make swift-test`; run
plain `make` to list them.

For quick iteration, `make fast-test` runs the Rust tests with
[cargo-nextest](https://nexte.st) (`cargo install cargo-nextest --locked`) plus
the doc tests. `.config/nextest.toml` kills any test that runs past 120 s so a
hang cannot stall the run. It does not replace the gate above.

## AI prompts

- Never hard-code text sent to an AI in Rust or Swift. Every prompt lives in `assets/prompts/<provider>/<action>.md`, paired with its provider, model, and reasoning effort; the file format is in [assets/prompts/README.md](../assets/prompts/README.md).
- Add or change a prompt only by editing its asset file. A new provider or action gets a new file and an entry in `diary-core::prompt_asset`.
- Rust embeds assets with `include_str!`; the Swift helper receives raw templates over the bridge and must not keep its own copy.
- Send `model` and `reasoning_effort` to the provider whenever they are not `default`. Changing a pinned value of a decided provider needs owner approval (see [AGENTS.md](../AGENTS.md)).
- Substitute placeholders in a single pass so diary text is never re-expanded, and keep `cargo test` covering every shipped asset.

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
