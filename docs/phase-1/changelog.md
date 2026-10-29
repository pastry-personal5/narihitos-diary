# Phase 1 changelog

Status: Active

This changelog records owner decisions and material plan changes for Phase 1.

## Entries

- 2026-10-29 — P1-M6 completion — Marked P1-M6 `Done`. Implemented the
  owner-approved Tokio/Axum loopback server, strict v1 DTO and ETag contract,
  purpose-named command/CAS layer, per-day reconciliation gates, server-owned
  expiring source preparation, bounded jobs and SSE, private lock/runtime/token
  lifecycle, and typed rotating diagnostics. Synthetic core/SQLite, real-socket,
  isolated-process restart, permission, retention, capacity, and privacy tests
  pass. The full Rust and Swift gate passed with Rust/Cargo 1.98.0 and Apple
  Swift 6.4; no private diary tree, live Notion root, or model/provider was
  accessed.
- 2026-10-29 — P1-M6 owner approval and start — Marked P1-M6 `Active` and
  approved the dependency and exact-limit packet in the milestone overview.
  Approved exact direct dependencies are Tokio 1.53.1, Axum 0.8.9, Serde
  1.0.229, Serde JSON 1.0.151, futures-core 0.3.34, getrandom 0.3.4, subtle
  2.6.1, and fs2 0.4.3 with only the recorded features. The packet adds no
  build tool, generator, logging/config framework, HTTP client, hosted-provider
  choice, or Swift package. It also freezes the v1 transport, source, job, SSE,
  identifier, port-attempt, diagnostic, and shutdown bounds and the optional
  `server.port_fallback_count` configuration key.
- 2026-10-29 — P1-M6 planning review — Reconciled the server plan with the
  implemented M4/M5 contracts. Added the missing workflow-list, writing-setting,
  manual-composition, and explicit empty-selection command boundaries; made
  prepared source bodies/stamps server-owned and expiring with a freshness
  recheck; defined buffer-only autosave CAS recovery; and tightened startup,
  degraded optional capabilities, runtime/database permissions, log crash
  recovery, numeric transport limits, job expiry, and SSE gap behavior. Fixed
  the diagnostic `rg` example to match uppercase log levels. P1-M6 remains
  `Planned`; no dependency, listener, production state, or private data was
  added or accessed.
- 2026-10-29 — P1-M6 logging requirement — Planned an internal owner-only
  Control API Server log under `.local/logs/control-api/`, with UTC-day and
  8 MiB rotation, a strict rolling 72-hour retention limit, a 24 MiB directory
  cap, and content-free typed records. Codex and Claude Code may read that
  Git-ignored directory directly for diagnosis without a control token; logs
  remain absent from REST, SSE, and MCP, and agents remain barred from the
  separate runtime-token directory. P1-M6 remains `Planned`; no implementation
  or dependency change was made.
- 2026-10-29 — P1-M5 post-completion review — Bounded discovery and selected
  reads at the opened-file boundary; rejected lone-CR, nested/duplicate date,
  and additional ambiguous audio-link forms; isolated helper handoffs per
  concurrent request; capped download waits to the Swift protocol limit;
  tightened response JSON, source-date, containment, and file-identity checks;
  and made helper access failure invalidate stale workflow state before its
  typed error returns. The complete gate passed with 46 Rust tests, all
  doctests, and 10 Swift tests. Added synthetic regressions only; no private
  source tree or live service was accessed.
- 2026-10-29 — Completion — Marked P1-M5 `Done`. Added connector-neutral
  source ports and SQLite v3 availability persistence; bounded Ayumi discovery,
  Markdown/audio privacy filtering, selected snapshots, and freshness checks;
  a dependency-free Rust helper client with deterministic lookup, process
  bounds, cancellation, typed failures, and private handoff cleanup; and Swift
  iCloud availability, download, and coordinated-read commands behind fakes.
  The full gate passed with 42 Rust tests, all doctests, and 10 Swift tests. No
  private iCloud tree, live model, Notion service, HTTP listener, or TUI was
  opened.
- 2026-10-29 — M4/M5 implementation review — Hardened M4 publication retries
  so reused approval is durably destination-scoped, verified destinations
  cannot regress, blocked destinations cannot change outside an approved retry,
  and restored approval scopes must name a remote target. Extended retention
  evidence through discard, both concurrency-version increments, restart, and
  repurge. Hardened the implemented M5 Ayumi reader against date-directory
  symlink escapes, malformed timestamp acceptance, unsafe attachment paths,
  fabricated file identities, metadata races, disappearance, and forged nested
  source references. The full Rust and Swift gate passed. M5 remains `Active`:
  the Swift iCloud commands, byte handoff, Rust helper client, and checkpoint
  integration are still pending.
- 2026-10-28 — Owner decision — P1-M7 uses Ctrl-G (F2) for Command Line,
  Ctrl-Enter (F3) to save/flush, Ctrl-D (F10) to quit, and
  Ctrl-Left/Ctrl-Right (F6/F7) for previous/next workflow step. The existing
  real-terminal Korean-IME acceptance check remains required.
- 2026-10-28 — Owner decision — TUI dates use `YYYY-MM-DD`, adding an
  abbreviated weekday as `YYYY-MM-DD (Mon)` where space and context permit.
  The ISO date remains visible in every form.
- 2026-10-28 — Planning review — Strengthened the P1-M7 manual-TUI plan with
  P1-M6 and dependency entry gates, validated runtime-file/health attachment,
  explicit server-authority and conditional-save recovery rules, selected-only
  source snapshot review, real-terminal IME evidence, and an acceptance-evidence
  table. No implementation, dependency, or product decision was added; P1-M7
  remains planned behind P1-M6.
- 2026-10-28 — Planning review — Expanded P1-M6 with P1-M4/P1-M5 entry gates,
  an explicit dependency and build-tool approval step, the P1-M7 manual-flow
  route boundary, separate workflow and entry-buffer preconditions, isolated
  lifecycle and loopback security tests, bounded jobs and SSE recovery, and an
  acceptance-evidence table. P1-M6 remains planned; no server implementation,
  dependency, provider, or live-data access was added.
- 2026-10-28 — Completion — Marked P1-M4 `Done`. Added the dependency-free
  `diary-core` workflow, revision, approval, publication, and retention
  contracts; a `rusqlite` system-SQLite adapter with forward v1/v2 migrations,
  transactional compare-and-swap, durable intents, and versioned receipts;
  and test-only `nd` assembly. Twenty-four Rust tests and eight Swift tests
  passed with formatting, warnings-denied Clippy, both builds, privacy/schema
  inspection, and a P1-M4-scoped `git diff --check`. No production database, live source,
  model, file destination, Notion call, HTTP listener, or TUI was added.
- 2026-10-28 — Owner approval and start — Marked P1-M4 `Active` after P1-M3
  completion. The owner approved `rusqlite` 0.40.2 for `diary-connectors`
  with system-SQLite linkage and no optional Cargo features. The packet adds
  no build tool, bundled C compilation, hosted provider, credential decision,
  HTTP dependency, or Swift package. Direct unsafe SQLite FFI was rejected in
  favor of maintained MIT-licensed bindings compatible with Apache-2.0.
- 2026-10-28 — Completion review — Rechecked P1-M3 code, tests, configuration,
  and its P1-M4/P1-M5 follow-on plans. Corrected the Ayumi config example to
  use one iCloud directory name, removed a contradictory blanket dependency
  approval claim and an undecided initial-language default, and aligned the M4
  autosave and pending/blocked retry rules with the product architecture. The
  command-shell tests now cover unsupported arguments and reliably clean
  scratch state. The helper bounds request IDs, distinguishes JSON booleans
  from integer protocol versions, and tests exact and oversized frames so
  responses remain capped. All four Rust command-shell tests, eight Swift
  protocol tests, the complete Rust and Swift gate, and `git diff --check`
  passed.
- 2026-10-28 — Completion — Marked P1-M3 `Done`. Added the six-package Rust
  workspace with one `nd` binary, explicit non-functional default and `serve`
  shells, a Swift 6.2/macOS 26 helper package with a bounded versioned fake
  protocol, safe configuration example and ignore rules, and reproducible
  contributor commands. Workspace, target, command, helper-stream, and ignore
  inspections passed. The complete Rust and Swift gate passed with Rust/Cargo
  1.98.0 and Apple Swift 6.4 from Xcode 27.1; no third-party dependency or
  later-milestone product behavior was added.
- 2026-10-28 — Planning review — Expanded the P1-M5 plan with milestone entry
  gates, bounded source identity and date evidence, malformed-header and audio
  privacy behavior, a private byte handoff that fits the approved 64 KiB helper
  envelope, explicit failure mapping, checkpoint freshness tests, and an
  acceptance-evidence table. P1-M5 remains planned behind P1-M3 and P1-M4;
  no source implementation or dependency was added.
- 2026-10-28 — Owner approval — Approved the P1-M3 dependency and toolchain
  packet before manifest changes. The approval covers this scaffold only;
  later dependency, feature, and build-tool changes still require approval
  under `AGENTS.md`. P1-M3 remains third-party-dependency free: six separate
  Rust packages use only standard-library or local path
  dependencies, and the Swift package uses SwiftPM, XCTest, and Foundation.
  Selected Rust 1.89 with edition 2024 and Swift tools 6.2 with macOS 26.0.
  The helper runs once per request and exchanges one newline-delimited UTF-8
  JSON frame in each direction, capped at 64 KiB; standard output is protocol
  only. Lookup prefers `ND_APPLE_BRIDGE_PATH`, then the installed `libexec`
  path, then the development build. Configuration prefers `ND_CONFIG_PATH`
  over repo-local `config/global_config.yaml` and later server loading must
  reject an absent or non-owner-only file. Keeping all outer crates preserves
  the architecture visibly without premature combination.
- 2026-10-28 — Planning review — Expanded the P1-M4 plan with milestone entry
  gates, an explicit boundary between core policy and later adapters,
  conditional workflow and buffer revisions, SQLite migration and privacy
  checks, fake crash reconciliation, retention boundaries, and an acceptance
  evidence table. No implementation, dependency, or product decision was added;
  P1-M4 remains planned behind P1-M3.
- 2026-10-28 — Completion — Marked P1-M2 `Done` after its architecture,
  UX reconciliation, owner-decision records, and completion checklist were
  complete and the milestone's documented alternative gate passed. P1-M3
  remains `Planned` until its dependency, Cargo-feature, build-tool, and Swift
  toolchain approval packet is owner-reviewed; no manifest, dependency, or
  production code was added by this transition.
- 2026-10-28 — Review — Strengthened the P1-M3 scaffold plan with an explicit
  P1-M2 entry gate, a dependency and toolchain approval record, a concrete
  workspace and dependency map, observable non-functional `nd` command shells,
  Swift helper protocol failure cases, safe configuration requirements, and a
  structured acceptance-evidence baseline. The review selected no dependency,
  build tool, hosted provider, credential location, or HTTP client; those
  approval and decision points remain open before implementation.
- 2026-10-28 — Planning — Expanded Phase 1 from the P1-M3 scaffold through
  eight additional implementation milestones, P1-M4 through P1-M11. The order
  follows the architecture's dependency direction: core/persistence, sources
  and Apple bridge, control server, TUI/manual workflow, draft assistants,
  local publication, Notion publication, then amendments and release
  qualification. P1-M8 retains the hosted-provider decision gate, P1-M10
  requires an owner-authorized read-only root compatibility check and disposable
  live-write fixture, and every milestone has a verifiable checklist and the
  standard Rust/Swift gate. No production code or dependency was added.
- 2026-10-28 — Research — Compared how Codex and Claude Code attach to the
  server for context and token cost. Claude Code defers MCP tool schemas by
  default and caps results at 25,000 tokens; Codex compacts each tool schema to
  5,000 bytes and offers tool allowlists. The architecture gained a small,
  ranged agent surface, a fixed port and a separate agent token when `/mcp` is
  enabled, and a note that a shell-capable agent can read owner-only files.
  Codex's app server was assessed as a drafting backend: its thread, compaction,
  and token-usage features mostly go unused for a short two-pass draft, while
  its agent baseline, hardening needs, and experimental protocol add cost. See
  the [composition research](../research/agent-assisted-diary-composition.md#codex-app-server-as-the-drafting-backend).
- 2026-10-28 — Review — Fixed design gaps found in the second review: startup
  reconciliation no longer blocks the listener and gates per diary day; editor
  autosave has its own entry-buffer version instead of bumping the workflow
  revision; REST gained workflow list and discard, setup check, preferences,
  amendments, and conflict resolutions; "abandoned" now means no workflow change
  for 30 days; a save-under-a-different-day target with an unfinished workflow
  receives no carried text; the header date is the calendar date in the
  timestamp's own offset; and the Phase 1 plan now names the server, the Swift
  helper, drafting, and amendment. The P1-M1 documents moved from Archived to
  Active and their stale rows were corrected.
- 2026-10-28 — Owner interview — Reversed the on-device-only rule. Hosted
  drafting is now the default behind the `DraftAssistant` port, with Apple
  on-device optional and a hosted adapter in Phase 1. No extra consent step;
  readable note bodies, memories, drafts, and the edited entry may leave the
  Mac, while front matter, attachment paths, audio, and credentials never do.
  The UI labels the active path. A later agent client may read those classes
  and write draft proposals; confirming, approving, and publishing stay
  human-only. The provider, credential storage, and HTTP dependency stay
  Undecided. This supersedes "composition stays on the Mac" and the research
  statements that Phase 1 sends no composition text to a hosted model.
- 2026-10-28 — Owner interview — Decided: keybindings are focus navigation plus
  Ctrl chords (no bare-letter or `:` hotkeys), and the Command Line maps Hangul
  typed on the 2-Set Korean layout back to ASCII command tokens. A day with no
  notes may produce a manual entry through "continue without notes". A
  Completed entry can be amended (replace the app's own receipted output) with
  the editor seeded from the saved, possibly hand-edited, local file; Notion is
  replaced by recorded block IDs and a mismatch is a Conflict. A blocked Notion
  retry needs a new Notion-only approval; a pending one reuses its approval
  while the plan digest is unchanged. The Swift helper joins the gate with
  `swift build` and `swift test`, and the stack line reads Rust plus one Swift
  helper. This narrows the earlier "never overwrite" rule to content the app
  did not write.
- 2026-10-28 — Owner interview — Decided: the canonical project acronym is `nd`,
  always lowercase, and the application is the single binary `nd` (`nd` opens
  the TUI; `nd serve` runs the server by hand in its own terminal). The Control
  API Server's HTTP and event layer is async on Tokio, superseding the research
  note that Tokio is not required; the TUI still does not need it. The same
  Swift helper that reaches the Apple model also performs iCloud availability,
  download, and coordinated reads. The research notes under `docs/research/` are
  marked Active, and their stale statements were corrected: config location,
  Tokio, IME languages, and the ignored `tmp/` fixture. Specific crates still
  need approval during scaffolding.
- 2026-10-28 — Owner — Noted an MCP server (not an MCP CLI) as a Phase 2
  feature in the [roadmap](../roadmap.md). Phase 2 stays unplanned.
- 2026-10-28 — Review — Revised the [architecture](../planned-mvp-architecture.md)
  and the UX documents ([terms](../ux-terms.md),
  [information architecture](../ux-information-architecture.md),
  [behavior](../ux-gui.md)). The architecture gained workflow states with
  voiding rules, a process lifecycle and startup order, an authorization trust
  boundary, write-ahead operation intents, a Swift helper contract, a files
  table, and a split of the catch-all `DraftStore` port into `WorkflowStore`
  and `OperationLog`. The UX documents gained step states, connection and error
  states, a Banner, terminal-size rules, inline inferred-reflection delimiters,
  and a confirm/approve/publish vocabulary.
- 2026-10-28 — Owner interview — Decided: Ayumi sources are en-US only and
  output is en-US or ko-KR; Japanese is out of scope for sources and for the
  IME indicator, which shows English and Korean only. The app does not detect
  language from a note body or block on it, and no known metadata field
  identifies language (recorded as Undecided). This replaces the research
  summary's pending `en ↔ ja` check; a live `en ↔ ko` switch was already
  observed in the probe. A human starts the server manually; the TUI never
  spawns it. A Swift helper process bridges Apple Foundation Models. The
  keybinding model stays open, with the Korean-IME constraint recorded. A new
  workflow defaults to today in the Mac's local time zone, unfinished drafts
  first. The Notion token moves to the repo-local, git-ignored
  `config/global_config.yaml` (owner-only), superseding the per-user global
  config file. Draft and source text is purged 7 days after verified output and
  30 days after abandonment. A conflict shows both versions and offers reread,
  discard, or a different diary day, never an overwrite. A blocked or failed
  Notion preflight still lets the local entry be approved and written.
- 2026-10-28 — Owner — Exempted P1-M2 from the standard milestone gate because
  it is documentation-only and the Cargo manifest arrives in P1-M3. See the
  [P1-M2 overview](./milestone-02-software-architecture-overview.md). The
  exemption does not extend to any later milestone.
- 2026-10-28 — Owner direction and architecture proposal — Planned P1-M2
  around a local Control API Server with the TUI as an HTTP client, extensible
  source and output connectors, REST and streaming interfaces, MVVM presentation,
  an explicit workflow pipeline, and a future MCP route. The
  [architecture proposal](../planned-mvp-architecture.md) preserves the
  read-only runtime Notion preflight and introduces no production dependency.
- 2026-10-28 — Owner — Restored the completed P1-M1 milestone documents and
  their supporting research notes to `docs/` for P1-M2/P1-M3 preparation.
  Added only milestone titles and goals; no milestone plans or code were created.
- 2026-10-28 — Research — Closed P1-M1 after all checklist items gained
  evidence. Its research-only exit-gate exemption applies; no production Rust
  code, dependency, or later milestone was added. Archived the concluded
  research and milestone documents. See [P1-M1 overview](./milestone-01-research-overview.md).
- 2026-10-28 — Owner and research — The owner reaffirmed that the configured
  Notion root stays uninspected during P1-M1 and must pass the runtime read-only
  preflight. A twelve-scenario synthetic hierarchy fixture covered missing,
  existing, duplicate, incompatible, and ambiguous year/month paths. This
  closes the remaining P1-M1 evidence item while retaining live connector
  validation as later acceptance work. See [validation evidence](../research/p1-m1-validation-evidence.md).
- 2026-10-28 — Owner correction — The configured Notion page is the root of a
  page hierarchy: `Root → YYYY → YYYY-MM`. Create missing year and month
  subpages under their respective parents, then append each `YYYY-MM-DD` daily
  section at the bottom of its month page. This supersedes the earlier
  direct-to-root publication destination; the confirmed body, duplicate
  checks, and read-before-retry rule remain. See [Notion
  research](../research/notion-diary-section.md).
- 2026-10-28 — Owner correction — Place each new Notion daily section at the
  bottom of its destination page (later clarified above as the month page).
  This supersedes the page-top placement and
  deferred top-insertion design recorded in the interview below. The
  researched append operation and duplicate-safe read-back remain the
  connector direction. See [Notion research](../research/notion-diary-section.md).
- 2026-10-28 — Owner interview — Confirmed that an unverifiable source
  freshness check stops output until reread and reconfirmation, and that the
  app requests download of selected cloud-only notes through a coordinated
  macOS read. See [input research](../research/markdown-input-format.md) and
  [operational constraints](../research/mvp-operational-constraints.md).
- 2026-10-28 — Owner interview — Chose reviewed two-pass Apple on-device
  drafting and an always-available multiline manual path, including after
  model failure. Composition remains on device. Each entry offers US English
  or South Korean Korean and concise, standard, or reflective style; the last
  language is remembered and concise is the style default. Model-inferred
  reflection is allowed by default but visibly marked for review. The complete
  edited entry requires confirmation, and unfinished drafts persist locally
  for recovery. See [composition research](../research/agent-assisted-diary-composition.md).
- 2026-10-28 — Owner interview — Chose a configurable local Documents entry
  root with `YYYY/MM/YYYY-MM-DD.md` files, YAML front matter containing day
  and language, and no automatic overwrite of different content. A private
  per-user SQLite database in Application Support holds draft recovery and
  publication receipts, excluding the Notion token. See [operational
  constraints](../research/mvp-operational-constraints.md).
- 2026-10-28 — Owner interview — Chose the confirmed Markdown body without
  YAML front matter for Notion, under an ISO-date `heading_2`, with newest
  daily sections at the top of the configured page (superseded by the correction
  above). Conflicting same-day content stops
  publication; an ambiguous write is read back before retry. The configured
  page is not inspected in P1-M1; a fail-closed read-only runtime preflight is
  required. The exact safe top-insertion method is deferred to connector
  design. See [Notion research](../research/notion-diary-section.md).
- 2026-10-28 — Owner interview — Confirmed Apple Silicon, macOS 26.0+, Rust
  1.89+, bounded workers, Ratatui/Crossterm multiline editing, and TIS
  notifications with a 500 ms polling fallback and unknown-language label.
  Chose a plaintext Notion token in a per-user global config file requiring
  owner-only permissions instead of Keychain. See [TUI research](../research/rust-tui-stack.md)
  and [IME research](../research/macos-ime-language-status.md).
- 2026-10-28 — Owner — Confirmed best-effort source freshness checks rather
  than continuous monitoring. When metadata/digest checks detect a change, the
  app invalidates the draft and previews and requires reread/reconfirmation.
- 2026-10-28 — Owner — Confirmed that supported UTF-8 files may include an
  optional BOM and may use LF or CRLF line endings.
- 2026-10-28 — Owner — Confirmed that Phase 1 supports UTF-8 source text only.
  A non-UTF-8 file is warned about and is not treated as having a readable body.
- 2026-10-28 — Owner — Confirmed that unknown front-matter fields are ignored
  and missing non-date fields are tolerated. Only a valid offset-aware
  `created_at` contributes the header date; otherwise the TUI warns and keeps
  the directory date.
- 2026-10-28 — Owner — Confirmed that a note with missing or malformed front
  matter remains selectable when its Markdown body is readable. The TUI shows
  a warning and uses only valid date metadata for proposals.
- 2026-10-28 — Owner — Confirmed that automatic discovery for diary day `D`
  reads only `D-1`, `D`, and `D+1`. Notes farther away require explicit date
  selection; the TUI does not automatically scan a wider archive.
- 2026-10-28 — Owner — Reopened P1-M1 and requested an interview for every
  unresolved or previously inferred choice. The autonomous completion pass is
  retained as research evidence, but its recommendations are not owner
  decisions until individually confirmed here.
- 2026-10-28 — Owner — Completed P1-M1 under its research-only exit-gate
  exemption. The final decision/evidence map and bounded runtime uncertainties
  are in [the research summary](./milestone-01-research-summary.md); this status
  was superseded by the reopening entry above.
- 2026-10-28 — Research — Recommended Apple on-device guided two-pass drafting with
  source-linked review and a full multiline manual fallback. Phase 1 sends no
  composition text to a hosted model, uses source-language concise first-person
  defaults, and prohibits model-inferred reflections. A one-pass Japanese probe
  was rejected after inventing unsupported details; the two-pass synthetic
  probe was grounded.
- 2026-10-28 — Research — Recommended a Phase 1 baseline of Apple Silicon on macOS 26.0+
  and Rust 1.89+. Selected dedicated bounded worker threads, a Foundation
  iCloud availability/coordinated-read adapter, and direct
  `security-framework` generic-password storage; disposable interoperability
  and Keychain probes passed.
- 2026-10-28 — Research — Recommended process-global Text Input Source Services for
  the language-only IME label, with notifications first, a 500 ms polling
  fallback, and `IME: ?` for missing language. A terminal-hosted AppKit context
  was absent; the Rust probe observed live language changes without privacy
  permissions.
- 2026-10-28 — Research — Recommended source discovery at `D-1`, `D`, and `D+1`
  plus one explicitly selected day, accepted readable UTF-8 bodies with
  optional front matter and per-file diagnostics, and made source ID/content-
  digest snapshots invalidate drafts and previews when files change. The owner
  initially confirmed only the discovery bound; the later interview entries
  above resolve the remaining recommendations.
- 2026-10-28 — Research — Recommended local output at configurable
  `~/Documents/narihitos-diary/entries/YYYY/MM/YYYY-MM-DD.md` with atomic create,
  same-digest resume, and no automatic overwrite. Versioned private per-day
  Notion receipts live under Application Support and contain digests/status/
  block IDs but no credential or duplicate diary prose.
- 2026-10-28 — Research — Recommended that Notion receive the complete confirmed
  entry as a `heading_2` ISO diary day followed by paragraph blocks at page end.
  Same-day ambiguity stops publication; ambiguous writes are read before retry,
  and `Retry-After` is honored. The uninspected owner page requires a fail-closed
  first-run read-only preflight.
- 2026-10-28 — Owner — Added P1-M1 research to determine how the Rust TUI can
  show English/Japanese input-language status in Apple Terminal. Japanese
  conversion sub-modes, input-source switching, other terminals, and production
  implementation remain outside this research.
- 2026-10-28 — Owner — Decided that Phase 1 ignores Ayumi audio attachments.
  Audio is not played, copied, uploaded, decoded, transcribed, or used for
  composition; attachment paths are excluded and attachment presence does not
  affect source-note eligibility.
- 2026-10-28 — Owner — Decided that Ayumi diary-day source proposals use both
  the directory date and Markdown-header `created_at` date. The MVP TUI must
  show date disagreements and let the user include or exclude discovered notes
  regardless of metadata date; the confirmed selection controls composition.
- 2026-10-28 — Owner — Planned P1-M1 as a research-and-decisions milestone,
  left later Phase 1 milestones unplanned, and exempted P1-M1 alone from the
  standard milestone exit gate because it produces research documentation
  rather than Rust implementation.
