# Contribution Guide

Status: Active

How to set up `narihitos-diary`, validate a change, write commit messages, and open a pull request.

## Setup and development

- The stable Rust toolchain with Cargo.
- The rustfmt and Clippy components.
- The Swift toolchain (Xcode command-line tools) for the Apple helper; its version is chosen during scaffolding.

```sh
cargo build                                               # build the application
cargo run                                                 # run the TUI application
cargo run -- serve                                        # run the Control API Server in the foreground
cargo test --all-features                                 # run the test suite
cargo fmt --all                                           # format Rust code
cargo clippy --all-targets --all-features -- -D warnings  # lint with warnings denied
swift build --package-path helpers/apple-bridge           # build the Apple helper (once it exists)
swift test --package-path helpers/apple-bridge            # test the Apple helper with fakes (once it exists)
```

## Required validation

Before opening a pull request, run the full checks from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
swift build --package-path helpers/apple-bridge   # once the package exists
swift test --package-path helpers/apple-bridge    # once the package exists; uses fakes
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
