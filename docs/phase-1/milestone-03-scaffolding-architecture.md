# P1-M3: Scaffolding and tools architecture plan

Status: Done

This plan turns the proposed module boundaries into buildable shells while
leaving behavior to later milestones. Dependency and build-tool changes remain
owner approval points under [AGENTS.md](../../AGENTS.md).

P1-M3 does not start until P1-M2 is `Done`. The implementation must first turn
this milestone's dependency and tooling candidates into one owner-reviewed
approval packet; research examples are evidence, not pre-approval.

## Affected modules and interfaces

- Workspace members for `diary-core`, connectors, the control contract and
  server, the TUI, and the `apps/nd` composition root.
- `helpers/apple-bridge` as the single Swift package and executable boundary.
- Repository configuration templates, ignore rules, and contributor tooling.
- Minimal command parsing for `nd` and `nd serve`.

## Planned repository shape

Use the architecture's boundaries as the default. Combining outer crates is
permitted only when the approval record maps every responsibility to its new
home and confirms that the dependency rule is unchanged.

```text
Cargo.toml
apps/nd/                       one `nd` binary and composition root
crates/diary-core/             domain, application shells, and inward ports
crates/diary-connectors/       outbound adapter shells
crates/diary-control-contract/ versioned wire DTO shells
crates/diary-control/          Control API Server shell
crates/diary-tui/              terminal-client shell
helpers/apple-bridge/          one Swift executable plus fake-based tests
config/global_config.example.yaml
```

The initial dependency direction is:

| Package | May depend on in the scaffold | Must not depend on |
|---|---|---|
| `diary-core` | Standard library and separately approved inward-only utilities | Every other workspace package and all framework/adaptor crates |
| `diary-control-contract` | Standard library and approved wire-format utilities | `diary-core`, connectors, server, TUI, and Apple code |
| `diary-connectors` | `diary-core` | Control server, TUI, and composition root |
| `diary-control` | `diary-core`, `diary-control-contract`, approved server primitives | TUI and composition root |
| `diary-tui` | `diary-control-contract` and approved presentation/client primitives | Connectors, server internals, SQLite, Notion, and Apple code |
| `nd` | All packages needed to select and compose concrete adapters | Product policy that belongs in `diary-core` |

Empty packages should expose only enough code to compile and prove dependency
direction. Do not manufacture future traits, DTO fields, state transitions, or
connector methods merely to populate the scaffold. Apple-framework imports stay
inside the Swift helper; a future Rust macOS shim, if justified, requires its
own approval and platform gate.

## Decision and approval packet

Before any manifest or build-tool edit, prepare one compact table with:

- each direct Rust dependency, exact feature set, owning package, purpose,
  version requirement, supported Rust version, and relevant transitive license
  evidence;
- the standard-library or already-approved alternative considered;
- every additional formatter, linter, license checker, code generator, task
  runner, or CI/build tool, including whether it changes generated files; and
- the Swift toolchain and package platform version, checked against macOS 26.0
  and the locally available Xcode command-line tools.

The packet must not select the hosted provider, hosted credential location, or
hosted HTTP client. Record the owner's approval in the Phase 1 changelog. A
later dependency or feature change needs a new approval rather than inheriting
blanket approval from the initial packet.

## Implementation sequence

### 1. Establish workspace metadata and boundaries

Create the virtual Cargo workspace and package manifests with the single binary
under `apps/nd`. Set the Rust edition, Phase 1 minimum Rust version, and
Apache-2.0 metadata consistently. Keep default features minimal and do not use
feature flags to conceal an invalid dependency direction. Inspect `cargo
metadata` and package-specific builds to prove the member list, target list, and
independent `diary-core` boundary.

### 2. Add observable command shells

Expose the default command and `serve` through the one `nd` target. Help output
may describe their intended roles, but executing either unfinished mode must
return a documented, unmistakable not-yet-implemented outcome. Tests or an
equivalent harness must show that these smoke runs do not bind a port, load the
real configuration, create Application Support state, read diary sources, or
contact Notion or a model. Do not add an HTTP server or terminal session in this
milestone.

### 3. Establish the Swift helper seam

Create one executable Swift package and its test target. Before wiring a Rust
client, record these scaffold choices:

- process lifetime: one process per request or one supervised process;
- deterministic lookup in a development checkout and the future installed
  layout, including the missing-helper outcome;
- request framing and maximum frame behavior on standard input/output; and
- how timeout, cancellation, process exit, malformed input, and standard-error
  diagnostics become content-free failures.

Choose between per-request and supervised lifetime using crash isolation,
cancellation, startup cost, future Foundation Models state, and protocol
complexity—not hypothetical throughput. The protocol shell needs only a fake
command. Its envelope carries protocol version, request ID, command, payload,
and either a success value or a stable error code. Standard output is protocol
only; test diagnostics go to standard error and contain no diary content,
credentials, or private paths. Bound correlation fields as well as whole
frames so a valid request cannot force an oversized response. Tests reject an
unsupported version, unknown command, malformed frame, and extra protocol
output. Real iCloud and Foundation Models commands remain in P1-M5 and P1-M8.

### 4. Make configuration safe by default

Track `config/global_config.example.yaml`; keep
`config/global_config.yaml` ignored. The example documents keys and product
defaults with obvious placeholders, never a working token, real root page ID,
username, or diary path. Decide and document the explicit override mechanism,
its precedence over the repository-root default, failure when the real file is
not owner-only, and behavior when it is absent. The command shells need not
parse private configuration merely to prove this layout.

Review ignore rules for Cargo and Swift products, the real configuration,
runtime files, coverage output, and scratch fixtures. A rule must not hide the
example configuration, source files, or reproducible test fixtures.

### 5. Document and verify the baseline

Update contributor documentation only with commands that work from a clean
checkout after the approved prerequisites are installed. Add targeted tests for
workspace boundaries, command-shell behavior, and the fake protocol before the
full gate. Record concise acceptance evidence in the overview: approvals and
decision links, tool versions, inspection results, smoke-test outcomes, and the
dated Rust/Swift gate result.

The required final commands are the repository gate, not substitutes:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
swift build --package-path helpers/apple-bridge
swift test --package-path helpers/apple-bridge
```

Later milestones may fill the shells and refine an approved outer-crate
grouping, but they must preserve the independent core, contract separation,
single `nd` binary, and one Swift-helper boundary. CI, packaging, signing, and
broad restructuring are not part of P1-M3.
