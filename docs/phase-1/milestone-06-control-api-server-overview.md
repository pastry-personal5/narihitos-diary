# P1-M6: Control API Server overview

Status: Done

This milestone exposes the completed core and source adapters through a
loopback Control API Server and makes `nd serve` a recoverable foreground
process. It supplies the transport needed by P1-M7 without moving diary policy
into HTTP handlers.

## Goal

Provide the versioned REST, SSE, job, lifecycle, and local authorization
contract through which the Phase 1 TUI uses diary workflows.

## Entry criteria

- P1-M4 and P1-M5 are `Done`, and the phase plan marks P1-M6 `Active` before
  implementation. Inspect their implemented ports, source outcomes, SQLite
  schema, and Swift-helper behavior rather than assuming planned signatures.
- Before changing a manifest, Cargo feature, or build tool, prepare an
  owner-reviewed dependency packet: each direct dependency and feature set,
  owning package, purpose, Rust 1.89 and license compatibility, transitive
  concerns, and the standard-library or already-approved alternative. Include
  any schema generator or test tool. Tokio is the decided server runtime, but
  its crate version and features and all HTTP/JSON/locking dependencies still
  need approval. Record the approval in the Phase 1 changelog.
- Use synthetic workflows, temporary configuration and state, fake source
  adapters, and fake destinations in tests. Do not inspect the private Ayumi
  tree or configured Notion root, or select a hosted provider in this milestone.

## In scope

- Foreground startup and bounded shutdown, single-instance lock, owner-only
  configuration and runtime files, production database path, migrations, purge,
  and background intent reconciliation.
- A thin application-command service around the implemented workflow, store,
  source, and operation-log ports. It owns load/validate/side-effect/CAS
  orchestration so handlers remain transport-only and every accepted mutation
  has one named atomic commit path.
- An owner-only internal rotating diagnostic log with a rolling 72-hour
  retention limit, bounded disk use, content-free records, and a stable
  repo-local location readable by Codex and Claude Code without a control token.
- Loopback-only binding with bounded port fallback, ephemeral control token,
  capability checks, `Host`/`Origin` validation, bounded requests and work,
  safe problem details, and content-free diagnostics.
- Versioned DTOs and routes for the completed manual workflow: readiness,
  preferences, start/resume/list/discard, discovery, source preparation and
  confirmation, explicit no-notes continuation, per-entry writing settings,
  manual-composition start, entry autosave and confirmation, jobs, and SSE
  notifications.
- Typed contract fixtures and in-process plus real-loopback transport tests for
  the later TUI client.

## Out of scope

- `/mcp`, remote access, browser clients, agent tokens, and a claim that a
  same-user process holding the control token is a human.
- A log-download API, log streaming over HTTP/SSE/MCP, arbitrary application
  messages, request or response bodies, and granting coding agents access to
  the runtime-token directory.
- Terminal screens and model extraction/drafting (P1-M7/P1-M8); real local or
  Notion output routes and connectors (P1-M9/P1-M10); amendments (P1-M11).
  Routes for those features remain unavailable until their owning milestones.
- Broadening the listener beyond `127.0.0.1`, selecting the hosted provider,
  or inspecting private source or Notion data during tests.

## Approved dependency and exact-limit packet

The owner approved the following packet on 2026-10-29 before any manifest edit.
Version requirements are exact for this milestone; default features are off
unless the table says otherwise. All selected crates support Rust 1.89 and have
licenses compatible with Apache-2.0.

| Owning package | Direct dependency and features | Purpose and compatibility | Alternative and transitive concern |
|---|---|---|---|
| `diary-control` and `nd` | `tokio = 1.53.1`, features `macros`, `net`, `rt-multi-thread`, `signal`, `sync`, `time` | Decided async server runtime; MIT; MSRV 1.71. | A standard-library listener would violate the decided Tokio runtime. Features for filesystem, process, I/O utilities, and the blanket `full` set stay disabled. Platform reactor/signal crates are the material transitive set. |
| `diary-control` | `axum = 0.8.9`, features `http1`, `json`, `query`, `tokio` | HTTP/1 routing, bounded JSON extraction, and response bodies; MIT; MSRV 1.80. | Hand-written HTTP parsing is a security and maintenance risk. Defaults stay disabled, excluding forms, macros, multipart, WebSockets, HTTP/2, tower logging, and tracing. It brings the Tokio/Hyper/Tower/http/bytes/futures stack, whose crates use MIT or MIT/Apache-2.0 terms. |
| `diary-control-contract` and `diary-control` | `serde = 1.0.229`, feature `derive`; `serde_json = 1.0.151`, no optional features | Strict versioned DTOs and JSON fixtures; both MIT/Apache-2.0; MSRVs 1.56 and 1.71. | Manual JSON already exists only for the fixed Swift envelope and would duplicate a much larger public contract. `preserve_order`, `raw_value`, arbitrary precision, and unbounded depth stay disabled. |
| `diary-control` | `futures-core = 0.3.34`, no optional features | The minimal `Stream` trait needed for bounded SSE bodies over Tokio channels; MIT/Apache-2.0. | Avoids the broader `futures-util`, `tokio-stream`, or an async-stream macro dependency. |
| `diary-control` | `getrandom = 0.3.4`, feature `std` | OS CSPRNG for 256-bit tokens and opaque instance/job/preparation IDs; MIT/Apache-2.0; MSRV 1.63. | Time/PID/counter IDs are guessable. No general RNG API is needed; platform entropy bindings are the only material transitives. |
| `diary-control` | `subtle = 2.6.1`, features `std`, with defaults off | Fixed-length constant-time token comparison; BSD-3-Clause. | An ordinary byte comparison may exit early. A local cryptographic primitive would be inappropriate. |
| `diary-control` | `fs2 = 0.4.3`, no features | Nonblocking advisory single-instance file lock (`flock` on macOS); MIT/Apache-2.0; documented Rust 1.8+ support. | `create_new` alone cannot distinguish a crashed owner from a live process; direct `libc` would require forbidden local unsafe code. The only macOS transitive is `libc`. |

No build tool, code/schema generator, formatter, linter, validation tool,
hosted-provider dependency, HTTP client, configuration parser, logging
framework, or Swift package is proposed. Configuration and the typed log
format remain small purpose-built parsers/encoders; tests use the standard
library, Axum's in-process service boundary where appropriate, and real
loopback sockets.

The proposed frozen v1 bounds are:

| Boundary | Limit |
|---|---|
| Listener and port fallback | Numeric `127.0.0.1` only; try `8771`, `8772`, then `server.port_fallback_count` consecutive ports; default `8`, valid `0..=16`, for at most 18 total attempts. |
| HTTP and JSON | 64 concurrent connections; 1 MiB request body; JSON depth 16; 512 KiB entry text; other strings 4 KiB unless a narrower field rule applies; 256 members per generic collection; unknown fields rejected. |
| Workflows and sources | At most 256 retained workflow summaries, paged 50 at a time; 512 candidates per workflow; 64 selected sources; 8 MiB per source and 16 MiB total prepared sanitized text; ordinary responses 2 MiB and selected-source detail responses 8 MiB plus a 64 KiB envelope. Candidate lists are paged, never truncated silently. |
| Jobs and blocking work | Queue 32; 4 concurrent workers; 128 live/retained job records; 120-second execution timeout; results and authenticated preparation handles expire after 10 minutes; cancellation and expiry remove prepared private text. |
| SSE | 16 subscribers; 256-event replay ring; 64 queued events per subscriber; events at most 1 KiB; heartbeat every 15 seconds; stream lifetime 30 minutes, followed by reconnect/refetch. |
| Identifiers and versions | Workflow IDs are the stable `wf_YYYYMMDD` derivation of `DiaryDay` but are opaque to clients. Instance, token, job, preparation, correlation, and event-instance values use 256 random bits encoded as 64 lowercase hex digits with a type prefix. Strong workflow ETags are `\"wf-<u64>\"`; buffer ETags are `\"buf-<u64>\"`. Timestamps are signed Unix milliseconds; diary days remain `YYYY-MM-DD`. |
| Diagnostics and lifecycle | Tokens/IDs are capped at 64 ASCII bytes after their fixed prefix; log records are 1 KiB; active rotation is 8 MiB; total log directory cap is 24 MiB; retention is 72 hours; maintenance runs at startup, rotation, and hourly. Graceful shutdown is 10 seconds. |

The config key is optional so existing valid files continue to load; omission
uses eight fallback ports. Invalid, duplicate, unknown, or out-of-range server
keys fail closed. All other startup and capability rules remain as specified in
the architecture plan.

## Completion checklist

This checklist is the implementation completion contract. Items stay unchecked
until the corresponding synthetic test or full-gate evidence is recorded below.

### Process and security

- [x] An isolated `nd serve` process test proves lock, configuration checks,
  diagnostic initialization, migration and purge ordering, listener readiness
  before runtime-file publication, duplicate-process refusal, stale-file
  recovery, background reconciliation, and bounded signal shutdown. An
  unresolved intent gates only its diary day and survives restart when its
  destination is unavailable.
- [x] Listener tests prove `127.0.0.1` binding, the configured bounded sequence
  beginning at `8771`, fallback and exhausted-range behavior. Runtime directory
  and file ownership/modes, database and sidecar modes, symlink/link rejection,
  atomic replacement, token rotation, and instance-owned cleanup before lock
  release are checked; a crashed server's leftover file is never trusted
  without the lock and matching health instance.
- [x] Tests reject absent or unsafe configuration before opening the database
  or listener, but do not make an unconfigured Notion destination or unavailable
  Apple helper fatal to the manual/no-notes server. They prove the Notion
  credential never enters runtime data, health, DTOs, errors, process
  arguments, or logs.
- [x] Real HTTP tests cover version/build discovery, bearer authorization and
  capability denial, constant-time comparison of a fresh OS-random token,
  loopback `Host` and `Origin` checks, explicit request/response/concurrency
  bounds, no-store responses, safe RFC 9457 errors, and content-free logs.

### Workflow contract

- [x] A reviewed `/v1` route and DTO table covers every P1-M7 manual-flow
  action and maps each to an implemented use case. Unsupported model, output,
  amendment, and MCP actions are not advertised or accidentally callable;
  route handlers contain no source, confirmation, approval, or publication
  policy.
- [x] The implemented-contract audit is closed with focused core/SQLite tests:
  bounded workflow listing, stable wire-ID-to-`DiaryDay` resolution, trusted
  prepared-selection confirmation with updated date evidence, entry writing
  settings/manual start, and buffer-only autosave all have explicit command and
  store semantics. No second workflow authority or durable client-supplied
  snapshot is introduced.
- [x] Multi-client tests prove atomic workflow `If-Match` checks, separate
  entry-buffer `If-Match` checks, confirmation against both current versions,
  conditional discard, stable status/problem mappings, and no mutation
  after a stale request. An internal CAS race during buffer-only autosave is
  retried only against an unchanged buffer version and a still-editable
  canonical workflow; otherwise the request fails without overwriting state.
- [x] Job and SSE tests prove accepted work, bounded execution, progress,
  timeout, cancellation requests, event ordering, reconnect and missed-event
  recovery by canonical state refetch. Prepared source bodies remain in a
  bounded, expiring, in-memory authenticated job result; confirmation names its
  server-issued handle and revalidates freshness rather than accepting stamps
  or bodies from the client. Restart expires the handle and requires prepare
  again. Entry confirmation runs the implemented pre-confirmation freshness
  checkpoint and durably commits any invalidation before returning its typed
  failure. SSE contains no diary text or credential.
- [x] Contract fixtures/schema and startup/client-attachment documentation
  match the implementation, including runtime-file and health negotiation,
  exact JSON/ETag/date/ID rules, pagination or collection bounds, body and
  response limits, job-result expiry, disabled later routes, and `/mcp` being
  outside Phase 1.

### Diagnostics and coding-agent access

- [x] `nd serve` creates `.local/logs/control-api/` relative to its working
  directory with owner-only directory and file modes, rejects symlinks and
  unsafe pre-existing entries, and writes only the versioned content-free
  line format defined by the architecture plan. Logs are not served by any
  Control API, SSE, or MCP route.
- [x] Fake-clock tests prove rotation at each UTC-day boundary and before a
  segment exceeds 8 MiB, deterministic unique segment names, startup and
  periodic cleanup, a strict rolling 72-hour cutoff, and a 24 MiB directory
  cap that may evict the oldest still-retained segments. Boundary, restart,
  clock-rollback, torn-active-record, malformed-segment, unrecognized-entry,
  deletion-failure, and disk-write-failure behavior is covered.
- [x] Privacy tests attempt to pass diary text, source references, private
  paths, request/header/body values, tokens, credentials, and arbitrary error
  strings through every logging entry point and prove the API accepts only
  the event schema's bounded typed fields and ASCII event codes. Every encoded
  record is at most 1 KiB, and log lines cannot be forged with newline or
  delimiter injection.
- [x] P1-M6 completion updates `AGENTS.md` (which `CLAUDE.md` imports) and the
  contribution guide with the stable directory, format version, safe `tail`
  and `rg` examples, and the rule that Codex and Claude Code may read only the
  log directory—not the separate Application Support runtime-token directory.
  A repository-root agent smoke test reads recent synthetic records without
  changing permissions or using the Control API token.

### Evidence and gate

- [x] Acceptance evidence below records focused tests and a dated full gate:
  `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`, `swift build --package-path helpers/apple-bridge`,
  and `swift test --package-path helpers/apple-bridge`.

### Post-completion implementation review

- [x] Audit the shipped M6 lifecycle, HTTP/security boundary, commands, jobs,
  SSE, and diagnostics against the frozen architecture and v1 contract; record
  each material finding as a focused regression test.
- [x] Fix every confirmed correctness, privacy, resource-bound, or recovery
  defect without broadening the approved dependency or product scope.
- [x] Re-run focused checks plus the complete Rust and Swift gate, record the
  dated review evidence below, and return P1-M6 to `Done` only when it passes.

## Acceptance evidence

All fixtures and server state below are synthetic. No test reads the private
Ayumi tree, configured Notion root, real Application Support state, or a live
model/provider.

| Evidence | Result |
|---|---|
| P1-M4/P1-M5 entry gate and owner-approved dependency packet | M4 and M5 were `Done`. On 2026-10-29 the owner approved the exact Tokio, Axum, Serde, futures-core, getrandom, subtle, and fs2 versions/features plus the frozen numeric limits before manifest changes; the changelog records the packet. Cargo metadata and the lockfile contain only that direct set and its compatible transitive graph. |
| Implemented-contract gap audit and focused core/SQLite changes | Synthetic command/SQLite tests pass for unfinished-first bounded summaries, stable day-derived wire IDs, revision-checked discard, per-day reconciliation gates, atomic workflow settings plus remembered preferences, trusted `SelectedSource` confirmation with updated date evidence, explicit empty selection, manual start, buffer-only autosave, stale clients, and entry confirmation. No durable UUID or client-authored snapshot/stamp was added. |
| Process lifecycle, filesystem permissions, and isolated restart tests | The isolated `nd serve` process test uses a temporary working directory, config, home, and state root. It proves safe stale-runtime replacement, duplicate-lock refusal, health-before-publication, owner-only runtime/database/log files, OS-random token rotation across restart, SIGTERM shutdown, instance-owned cleanup, and nonfatal missing Apple/Notion capabilities. Lifecycle unit tests cover strict config bounds, symlink/mode rejection, lock release, and atomic advertisement cleanup. |
| Loopback, authentication, origin, bounds, and redaction tests | Real-socket tests pass for numeric `127.0.0.1`, fixed fallback/exhaustion bounds, versioned health, exact/deduplicated Host, optional exact Origin, forbidden proxy headers, bearer success/failure, capability denial, no-store, JSON depth and string limits, correlated RFC 9457 problems, strong ETags, `428`, stale `412`, and conditional discard. Request concurrency/body bounds, bounded blocking diagnostics, and constant-time fixed-token comparison are enforced in the shared middleware. |
| DTO, multi-client revision, job, and SSE tests | Strict unknown-field contract tests and real-loopback flows pass for async discovery acceptance/polling, progress/completion, post-commit SSE ordering and privacy, stale workflow clients, independent buffer preconditions, and canonical refetch semantics. Focused regressions cover expiring prepared-body review with updated date evidence, duplicate source rejection, terminal-state job expiry, cancellation/timeout commit revocation, subscriber cleanup, oversized-replay reset, and typed SSE shape; preparation handles are random, process-local, revision/source-bound, consumed once, and freshness-checked before commit. |
| Rotation, 72-hour retention, bounded disk use, privacy, and coding-agent read tests | Focused control tests cover UTC-day and 8 MiB pre-write rotation, clock rollback including retention by newest record, collision-safe names, torn-tail repair, malformed/reordered/unrecognized record rejection, exact 72-hour deletion, oldest-first directory-cap eviction with active-file preservation, injected write/deletion failures, strict modes, and registered typed event fields. The process smoke reads only synthetic `ndlog-v1` records from the repository-relative log path and finds no token, credential, or config placeholder. |
| Documentation and contract inspection | `control-api-v1.md`, the checked route/DTO table, README, configuration example, contribution guide, `AGENTS.md`, phase plan, and architecture agree on attachment, authorization, capabilities, IDs/ETags, limits, disabled later routes, diagnostics, and `/mcp` exclusion. Codex/Claude guidance permits bounded read-only log access and forbids runtime-token-directory access. |
| Full Rust and Swift gate, with date and tool versions | Passed on 2026-10-29: formatting, warnings-denied workspace Clippy, all Rust tests and doctests, Swift build, Swift test (10 cases), and `git diff --check`. The completion run used Rust/Cargo 1.98.0 and Apple Swift 6.4 targeting arm64 macOS 26.0. |
| Post-completion implementation review and requalification | Completed on 2026-10-29. The review repaired prepared-source review access, capability and proxy enforcement, request and response bounds, conditional late-job commits, job expiry/capacity accounting, SSE replay/subscriber lifetime, typed request diagnostics and controlled failure shutdown, path/runtime cleanup races, and rollback-safe retention. The delegated full gate passed again: formatting, warnings-denied Clippy, all Rust tests and doctests including escalated real-loopback/process coverage, Swift build, Swift test (10 cases), and `git diff --check`. |
