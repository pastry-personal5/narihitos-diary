# P1-M3: Scaffolding and tools overview

Status: Done

This milestone creates the buildable project skeleton and verification baseline
used by every later Phase 1 milestone. It introduces structure and test seams,
not diary behavior.

## Goal

Establish the approved Rust workspace, `nd` binary, Swift helper package, and
development checks without prematurely implementing product workflows.

## Entry criteria

- P1-M2 was `Done`, and P1-M3 remained `Planned` until the approval record
  below was owner-reviewed.
- The owner reviewed one approval packet covering every proposed Rust
  dependency, Cargo feature, additional build or validation tool, and the Swift
  toolchain version. Approval is recorded before a manifest or tool
  configuration changes.
- No unresolved item in [AGENTS.md](../../AGENTS.md) is required to create the
  scaffold. In particular, this milestone does not choose a hosted provider,
  hosted credential location, or hosted HTTP dependency.

## Approval packet

Owner review was completed before the manifests or tool configuration described
here were added. The approved scaffold deliberately uses no third-party package:
all Rust dependencies are workspace paths, and the Swift package uses only the
Apple SDK's Foundation module.

### Rust packages and direct dependencies

All packages are version `0.1.0`, use Rust edition 2024, declare
`rust-version = "1.89"` and `license = "Apache-2.0"`, and define no Cargo
features. There are no crates.io dependencies or third-party transitive
licenses in this milestone.

| Owning package | Direct dependency | Requirement and features | Purpose | Alternative considered |
|---|---|---|---|---|
| `diary-core` | None | Standard library only | Keep domain and application shells independently buildable. | No dependency is needed for an empty boundary shell. |
| `diary-control-contract` | None | Standard library only | Keep versioned transport shells separate from core and adapters. | No dependency is needed until wire DTOs are introduced. |
| `diary-connectors` | `diary-core` | Local path `0.1.0`; default features disabled; no features | Prove that outbound adapters point inward. | Combining it with core would erase the adapter boundary. |
| `diary-control` | `diary-core`, `diary-control-contract` | Local paths `0.1.0`; default features disabled; no features | Prove that the server shell depends on policy and its wire contract. | A standard-library-only server shell would not make the intended dependency direction inspectable. |
| `diary-tui` | `diary-control-contract` | Local path `0.1.0`; default features disabled; no features | Prove that the client shares only the wire contract with the server. | Depending on server internals would violate the client boundary. |
| `nd` | All five workspace libraries | Local paths `0.1.0`; default features disabled; no features | Establish the one composition root without product behavior. | Combining outer packages is unnecessary at this size and would make the approved architecture less visible. |

`nd` command parsing uses `std::env`; smoke tests use only the Rust standard
library. Ratatui, Crossterm, Tokio, HTTP, SQLite, serialization, configuration,
Notion, Apple-framework Rust bindings, and hosted-provider dependencies remain
deferred to their owning milestones. Each later dependency, feature, or build
tool remains subject to the repository's owner-approval rule.

### Toolchain and tools

| Item | Approved selection | Compatibility and license effect | Alternative considered |
|---|---|---|---|
| Rust package baseline | Rust 1.89 minimum; edition 2024 | Matches the Phase 1 minimum. The local validation toolchain is Rust/Cargo 1.98.0. | Raising the minimum to the installed compiler would unnecessarily narrow supported systems. |
| Rust validation | Cargo, rustfmt, and Clippy from the existing Rust toolchain | No repository dependency or generated source. Local versions are Cargo 1.98.0, rustfmt 1.9.0, and Clippy 0.1.98. | No additional task runner, formatter, linter, license checker, generator, or CI tool is needed. |
| Swift package baseline | `swift-tools-version: 6.2`; macOS 26.0 minimum | Compatible with the Phase 1 platform. Validation uses Apple Swift 6.4 from Xcode 27.1; no external Swift package or transitive license is added. | Requiring the locally installed Swift 6.4 would raise the baseline without scaffold benefit. |
| Swift implementation and tests | Swift Package Manager, XCTest, and the Apple SDK Foundation module | Uses only tools and frameworks shipped with Xcode; no generated source. | A third-party JSON or test package is unnecessary for the bounded fake protocol. |

### Scaffold decisions approved with this packet

- Keep all six Rust packages shown in the architecture; do not combine outer
  crates in P1-M3.
- Spawn one Swift helper process per request. This gives simple cancellation
  and crash isolation now; a supervised long-lived process would add lifecycle
  and recovery complexity before startup cost or model-state evidence exists.
- Exchange exactly one newline-delimited UTF-8 JSON request and response per
  process, capped at 64 KiB including the newline. Standard output is protocol
  only; content-free diagnostics use standard error. Request IDs are nonempty
  and capped at 128 UTF-8 bytes so every correlated response remains bounded.
  Oversized, unterminated, malformed, unsupported-version, and unknown-command
  requests fail closed.
- Resolve the helper from `ND_APPLE_BRIDGE_PATH` when explicitly set, then from
  `../libexec/apple-bridge` relative to the installed `nd` executable, then
  from `helpers/apple-bridge/.build/debug/apple-bridge` in a development
  checkout. A missing helper marks Apple-backed capabilities unavailable and
  does not disable hosted drafting or the manual editor.
- Resolve configuration from `ND_CONFIG_PATH` when set, otherwise from
  `config/global_config.yaml` relative to the server working directory. The
  override has precedence. Once configuration loading is implemented, a
  missing file or any group/world permissions fail server
  startup without exposing credentials or private paths.

## In scope

- Confirm the initial dependency and tool choices with the owner before changing
  manifests or build tooling.
- Create the Rust workspace boundaries from the architecture, preserving an
  independently testable inward-facing core and one canonical `nd` binary.
- Provide `nd` and `nd serve` command shells with no claimed workflow behavior.
- Create `helpers/apple-bridge` with a versioned protocol shell and fake-based
  test target; choose and document its Swift toolchain version.
- Settle and record the scaffold-level choices left by the architecture: Swift
  helper process lifetime and lookup, protocol framing, configuration-path
  override and precedence, and the outer-crate mapping.
- Add repository-local configuration examples and ignore rules without adding
  real credentials or private diary data.
- Establish unit-test, formatting, lint, and build commands for both languages.

## Out of scope

- Diary workflow, HTTP endpoints, terminal screens, connector behavior, and
  external writes.
- Selecting the hosted provider, its credential storage, or its HTTP client.
- Reading the configured Notion root or private Ayumi notes.
- CI or release packaging, signing, installation, database migrations, and a
  production Apple command.
- Adding placeholder abstractions for behavior owned by P1-M4 or later.

## Completion checklist

### Readiness and decisions

- [x] P1-M2 is `Done`, and the phase plan marks P1-M3 `Active` before
  implementation begins.
- [x] The approval record lists each proposed direct dependency and Cargo
  feature, its purpose and owning package, version/MSRV constraints, relevant
  transitive licenses, and why a standard-library or existing-tool alternative
  is insufficient; it separately lists any new build or validation tool.
- [x] The owner approved that record and the Swift toolchain version before the
  corresponding manifest or tool configuration was added.
- [x] The changelog records the selected helper lifetime and lookup rules,
  protocol framing, configuration-path precedence, and any combination of the
  proposed outer crates, with rationale.

### Rust scaffold and command shells

- [x] Every boundary in the product architecture maps to a workspace package or
  to a documented, owner-approved combination of outer packages; package
  metadata declares Apache-2.0 and a Rust version compatible with the Phase 1
  minimum.
- [x] `diary-core` builds and tests independently, and workspace metadata shows
  that it does not depend on terminal, HTTP, SQLite, Notion, configuration,
  async-runtime, or Apple-framework code.
- [x] Exactly one binary target is named `nd`; its help identifies the default
  mode and `serve` without advertising unfinished diary behavior.
- [x] Smoke tests prove that the default mode and `serve` return their documented
  not-yet-implemented outcome without opening a listener, reading private data,
  creating persistent state, or performing an external write.

### Swift helper and safe configuration

- [x] `helpers/apple-bridge` declares the selected Swift tools and macOS
  platform versions, builds as the one helper executable, and has no real
  iCloud or Foundation Models behavior.
- [x] The helper protocol shell defines a version, request correlation, command,
  payload, success response, and redacted error shape; fake tests cover a valid
  exchange plus unsupported-version, unknown-command, and malformed-input
  failures without placing diagnostics on standard output.
- [x] A tracked `config/global_config.example.yaml`, the ignored
  `config/global_config.yaml`, and the documented path-resolution rule agree;
  the example contains only placeholders and product defaults, and setup
  requires owner-only permissions for the real file.
- [x] Ignore rules cover the real configuration, Rust and Swift build output,
  local runtime artifacts, and test scratch data without hiding source or an
  example configuration.

### Evidence and gate

- [x] Contributor documentation names the actual prerequisites, setup, build,
  smoke-test, format, lint, and test commands from the repository root.
- [x] Acceptance evidence records the approval reference, selected tool
  versions, boundary/target inspection, smoke-test outcomes, and dated results
  for every required gate command; no checklist item is checked from intent
  alone.
- [x] The standard milestone gate passes: `cargo fmt --all -- --check`, `cargo
  clippy --all-targets --all-features -- -D warnings`, `cargo test
  --all-features`, `swift build --package-path helpers/apple-bridge`, and `swift
  test --package-path helpers/apple-bridge`.

### Post-implementation review

- [x] The command shells and helper reject boundary inputs deterministically,
  clean up test scratch state after failures, and have regression coverage for
  every corrected edge case.
- [x] Manifests, configuration examples, ignore rules, and contributor guidance
  agree with the implemented scaffold and do not overstate unfinished behavior.
- [x] The milestone diff is internally consistent, passes targeted inspection,
  and passes the complete Rust and Swift gate after review fixes.

## Acceptance evidence

The following concise results support the checked items above; raw build logs
are intentionally not stored in the repository.

| Evidence | Result |
|---|---|
| Owner approval and decision record | Owner approved the P1-M3 packet in the 2026-10-28 implementation interview before manifest changes. The approval covers this dependency-free scaffold, not later dependency, feature, or build-tool changes. Decisions are recorded in the [Phase 1 changelog](changelog.md). |
| Workspace boundaries and binary-target inspection | On 2026-10-28, `cargo metadata --no-deps --format-version 1` reported the six approved workspace members, their inward-only local dependencies, and exactly one binary target, `nd`. `cargo test -p diary-core` passed independently; metadata reports zero `diary-core` dependencies. All packages declare the workspace's Apache-2.0 license and Rust 1.89 minimum. |
| `nd` / `nd serve` smoke tests | Four Rust integration tests passed. Help names only the unfinished default and `serve` modes; default and `serve` return 1, while unsupported arguments return 2 without starting either mode. Runs use an empty temporary home plus deliberately missing config/helper overrides, create no state, and remove their unique scratch directory even after an assertion failure; source and metadata inspection show no listener, private-data, persistence, or external-write dependency or code path. |
| Swift fake protocol tests and stream-discipline check | Eight XCTest cases passed for success, unsupported and invalid version types, unknown command, malformed and invalid framing, bounded request correlation, and exact/oversized 64 KiB frames. Every tested response stays within one capped JSON line. Process smokes produced exactly one JSON standard-output line: valid `ping` exited 0 with empty standard error; an unknown command exited 1 with only the stable content-free diagnostic on standard error. `swift package dump-package` confirmed tools 6.2, macOS 26.0, no package dependency, and one executable product. |
| Configuration and ignore-rule inspection | The example contains only explicit placeholders and the entries product default; its Ayumi value is one iCloud directory name rather than an absolute path. Contributor setup recommends mode `600` and documents `ND_CONFIG_PATH` precedence. `git check-ignore` confirmed the real config, Rust/Swift products, local runtime state, scratch directories, and coverage files are ignored while the example, Rust source, and Swift manifest remain visible. |
| Full Rust and Swift gate, with tool versions and date | Passed on 2026-10-28: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features`, `swift build --package-path helpers/apple-bridge`, and `swift test --package-path helpers/apple-bridge`; `git diff --check` also passed. Validation used Rust/Cargo 1.98.0, rustfmt 1.9.0, Clippy 0.1.98, Apple Swift 6.4, and Xcode 27.1. |
