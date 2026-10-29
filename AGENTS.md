# AGENTS.md

Instructions for contributors and AI coding agents. This is the single source of agent instructions; `CLAUDE.md` imports it with `@AGENTS.md`.

## Project

`narihitos-diary`: Make remembering a day feel effortless.

Stack: Rust, plus one Swift helper process for Apple APIs.

See [README.md](README.md) for the overview. Setup and contribution rules are in [docs/contribution-guide.md](docs/contribution-guide.md); other docs are indexed at [docs/README.md](docs/README.md).

## Agent workflow

- Work only on a clear task in the active milestone. Read its plans, relevant design docs, and applicable "Undecided" items first.
- Before implementation, add a verifiable completion checklist to the milestone overview. Check items only when evidence exists.
- Keep implementation, tests, and affected docs together. A milestone is done only when its checklist is complete and the gate passes: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-features`, plus `swift build` and `swift test` for the helper once `helpers/apple-bridge` exists, unless its overview records an explicit owner-approved exemption. If a required command cannot run, leave its item unchecked and report the blocker.
- Ask before building on an undecided item, changing dependencies or build tools, deleting user data, or broadly restructuring the repository. Other work within the approved task and milestone may proceed.
- Follow the active phase and milestone order in `docs/roadmap.md` unless the phase plan says otherwise. Create the roadmap when planning the first phase.

## Decided — ask before changing

- **Technical stack:** Rust, plus one Swift helper process for Apple APIs (see
  Apple bridge). The gate covers both.
- **Control architecture:** a local Control API Server owns workflows and
  connectors. The TUI is an HTTP client of that server; the design allows
  additional input and output connectors and a later MCP interface. See the
  [architecture proposal](docs/planned-mvp-architecture.md).
- **Binary name:** the canonical project acronym is `nd`, always lowercase.
  The application is the single binary `nd`.
- **Server lifecycle:** a human starts the Control API Server manually in its
  own terminal with `nd serve`, a subcommand of the one binary; `nd` opens the
  TUI. The TUI never spawns the server; it shows a "server not
  running" state with that command until it can attach.
- **Server runtime:** the Control API Server's HTTP and event layer is async on
  Tokio. The TUI keeps bounded worker threads and does not need Tokio. This
  supersedes the earlier "Tokio not required" research note for the server;
  specific crates are approved during scaffolding.
- **Apple bridge:** the server reaches iCloud availability, download requests,
  and coordinated reads, and Apple's Swift-only Foundation Models API for the
  optional on-device model, through one Swift helper process. Its wire
  protocol, location, and build are scaffolding details. Its tests use fakes,
  never the real model or live iCloud.
- **License:** Apache-2.0 for all project code.
- **Process:** numbered phases contain numbered milestones; milestone numbering restarts per phase. IDs use `P1-M2`. See [docs/development-process.md](docs/development-process.md).
- **Diary-day source selection:** for Ayumi's Diary input, use both the
  `YYYY/MM/DD` directory date and the offset-aware `created_at` Markdown-header
  date to propose a day's source notes. Show disagreements in the TUI. The user
  can include or exclude discovered notes regardless of either metadata date;
  the confirmed selection is the source set for the final entry.
- **Ayumi audio in Phase 1:** ignore audio attachments. The MVP does not play,
  copy, upload, decode, transcribe, or use audio or its attachment path for
  composition. Attachment presence does not affect source-note eligibility.
- **Automatic source discovery:** for diary day `D`, inspect only the three date
  directories `D-1`, `D`, and `D+1`. Finding a note outside that range requires
  the user to select another date explicitly; do not scan a wider archive
  automatically.
- **Missing or malformed front matter:** if a Markdown body is readable, keep
  the note selectable and show a warning. Use only valid date metadata when
  proposing it; malformed metadata must not be silently accepted as valid.
- **Front-matter field tolerance:** ignore unknown fields and tolerate missing
  non-date fields. Only a valid offset-aware `created_at` contributes the
  header date; otherwise warn and retain the directory date.
- **Source text encoding:** support UTF-8 only. If a source file cannot be
  decoded as UTF-8, show a warning and do not treat its body as readable.
- **UTF-8 envelope:** accept UTF-8 with or without a BOM and accept both LF and
  CRLF line endings.
- **Source changes during a workflow:** make a best-effort freshness check using
  file metadata and a content digest at workflow checkpoints. If a change is
  detected, invalidate the draft and output previews and require rereading and
  reconfirmation; continuous filesystem monitoring is not required. If
  freshness cannot be verified, stop and require a successful reread.
- **Cloud-only sources:** request download only for selected cloud-only iCloud
  notes, then coordinate their reads through a macOS adapter.
- **Composition:** offer reviewed two-pass drafting behind one `DraftAssistant`
  port and an always-available multiline Draft editor for manual editing, including after
  assistant failure. Register assistant adapters by stable ID and allow two or
  more to be available without provider-specific workflow or TUI branches;
  each assistant action targets exactly one selected backend. The default
  adapter is a hosted model (provider undecided); Apple on-device drafting
  through the Swift helper is an optional adapter. A future Codex-backed
  adapter uses the same registry but requires its own approved provider/process,
  credential, dependency, and lifecycle decisions. Sending occurs only when
  the user invokes an assistant action and includes only confirmed source
  snapshots. Offer US English or South Korean Korean per entry, remembering the
  last choice, and concise, standard, or reflective style with concise as the
  default. Mark assistant-inferred reflections for user review. A user may ask a
  selected backend to review a paragraph; its version-bound suggestion must be
  explicitly accepted or rejected and never confirms the complete entry.
  Confirm the complete edited entry before output. Recover unfinished drafts
  from local persistent state. This supersedes the earlier on-device-only rule.
- **Hosted data egress:** with no extra consent step, readable source-note
  bodies, extracted memories, drafts, the edited entry, and a paragraph focused
  for review may be sent to one selected hosted provider. Front matter
  (location, weather, ids), attachment paths, audio, credentials, and the Notion
  root never leave. The UI labels the selected backend and its composition path;
  the label is not a consent. Logs and errors stay content-free. A later `/mcp`
  agent client (Codex, Claude Code) may read the same data classes and write
  draft proposals; confirming a selection or entry, approving a plan, and
  publishing stay human-only in the TUI.
- **Language scope:** Ayumi source notes are en-US only; composition output is
  en-US or ko-KR. Japanese is out of scope for sources and for the IME
  indicator, which covers English and Korean only. The app neither detects
  language from a note body nor blocks on it.
- **Default diary day:** a new workflow defaults to today in the Mac's local
  time zone, and the Start stage lists unfinished drafts first.
- **TUI workflow layout:** use the canonical user-facing stages `Start`,
  `Sources`, `Compose`, `Review`, `Preview`, and `Publish`, in that order. The
  standard layout has a full-width Work Status pane above a left Workflow
  Sidebar for those stages, a middle Resources Sidebar for sources, drafts,
  assistant suggestions, or destinations, and a right Main Pane for the active
  resource. Work Status has four content rows excluding its border and title;
  it explains current work and useful messages and may duplicate the compact
  bottom Status Line. Activating a source displays its readable text without
  changing whether it is included.
  Only the confirmed included snapshots can be sent, and only when the user
  explicitly invokes an assistant action. A completed assistant draft opens as
  editable text in the Main Pane; paragraph suggestions require explicit accept
  or reject.
- **TUI date display:** display dates as `YYYY-MM-DD`, adding the abbreviated
  weekday as `YYYY-MM-DD (Mon)` where the available space and context permit.
- **Draft retention:** keep receipts and digests. Purge draft and source text 7
  days after every output verifies, and purge abandoned drafts after 30 days.
  The user can discard a draft at any time.
- **Local output:** default to configurable
  `~/Documents/narihitos-diary/entries/YYYY/MM/YYYY-MM-DD.md`, with YAML front
  matter containing diary day and writing language. Never automatically
  overwrite different content at the same path; the one exception is an
  approved amendment of the app's own receipted output.
- **Private state and credential:** use a per-user SQLite database in
  Application Support for draft recovery and publication receipts. Keep the
  Notion token out of the database; store it in plaintext in the repo-local,
  git-ignored `config/global_config.yaml`, which must be owner-only. This
  supersedes the earlier per-user global config location.
- **Control-server diagnostics:** in P1-M6, keep content-free structured
  Control API Server logs under the repo-local, Git-ignored
  `.local/logs/control-api/` directory by default. Rotate by UTC day and bounded
  segment size, retain no log segment beyond a rolling 72 hours, and keep the
  directory owner-only. Codex and Claude Code may read this log directory
  directly for diagnosis, but logs are never exposed through HTTP or MCP and
  never contain diary text, source metadata, paths, credentials, tokens, raw
  request data, or arbitrary error strings.
- **Output conflicts and blocked Notion:** when the local file differs, the whole
  plan stops; when Notion holds conflicting same-day content, only Notion
  stops. The app shows both versions and offers only reread, discard the entry, or save under a
  different diary day; it never overwrites content it did not write. If Notion
  is unconfigured, unreachable, or fails preflight, the local entry can still
  be approved and written, and Notion shows blocked and retryable. Retrying a
  blocked Notion destination needs a new Notion-only preview and approval;
  retrying a pending one reuses the approval only while the re-checked plan
  digest is unchanged.
- **Amendments:** a Completed entry can be amended. Choosing its day offers
  amend instead of the existing-entry conflict. The amend workflow starts from
  the local file's text (hand-edited text included, with a "changed since
  publication" banner), does not re-select sources, and skips source freshness
  checks; the original selection survives as receipt digests. The local file is
  replaced atomically only if its digest still equals the one seen when the
  editor was seeded. A Notion section is replaced by its recorded block IDs
  only if it still matches the receipt; otherwise it is a Conflict with the
  usual three resolutions. Receipts are versioned and the 7-day purge restarts.
- **Notion publication:** treat the configured page as a root. Under it, use a
  `YYYY` year subpage and a `YYYY-MM` month subpage, creating missing subpages
  when publishing. Send the confirmed Markdown body after stripping YAML front
  matter, under a `heading_2: YYYY-MM-DD` section at the bottom of that month
  page. Stop on conflicting same-day content, and inspect the relevant parent
  and month pages before retrying an ambiguous write. Require a fail-closed
  read-only root-page preflight at runtime. An amendment may replace the app's
  own receipted section.
- **Keybindings:** focus plus Ctrl chords. Non-editor screens move focus
  between on-screen controls with Tab and the arrow keys and activate with
  Enter. The editor and Command Line use Ctrl chords, with function keys as
  aliases. No bare-letter or `:` hotkeys; avoid Ctrl-I, Ctrl-M, Ctrl-[, and
  Ctrl-Space (macOS's default input-source switch). The Command Line opens with
  a chord. Its commands are ASCII tokens, and the TUI maps Hangul typed on the
  2-Set Korean layout back to the key that produced it, so commands work under
  either IME. Every command is also reachable by focus and Enter. P1-M7 uses
  Ctrl-G (F2) for Command Line, Ctrl-Enter (F3) to save/flush the editor,
  Ctrl-D (F10) to quit, and Ctrl-Left/Ctrl-Right (F6/F7) for previous/next
  workflow stage. These chords must pass a real-terminal check with the Korean
  IME active.
- **Empty days:** a diary day with no source notes, or none included, may still
  produce an entry through an explicit "continue without notes" confirmation.
  Assistant extraction, drafting, and paragraph review are disabled; the manual
  editor works. A blank entry cannot be confirmed.
- **Phase 1 platform:** Apple Silicon, macOS 26.0+, Rust 1.89+; bounded
  background workers; Ratatui/Crossterm with multiline editing; TIS input-
  language notifications with a 500 ms polling fallback; `IME: EN`, `IME: KO`,
  and `IME: ?` when unknown.

## Undecided — ask before inventing

- **Notion connector:** the configured root page's structure remains
  uninspected. Validate the root and any existing year/month path before
  creating pages or appending a daily section.
- **Hosted provider:** the provider, its credential storage, and the HTTP
  dependency for the default hosted adapter are undecided. Settle them before
  building that adapter; until then the `DraftAssistant` port, the data classes
  in Hosted data egress, and the Draft editor are the contract.
- **Language metadata:** no known Ayumi front-matter field identifies a note's
  language, so nothing is blocked by language. Name a field before adding
  metadata-based rejection.

The interview decisions and remaining page-compatibility check are mapped in the
[research summary](docs/phase-1/milestone-01-research-summary.md).

## Environment

- The stable Rust toolchain with Cargo.
- The rustfmt and Clippy components.
- The Swift toolchain (Xcode command-line tools) for the Apple helper; its
  version is chosen during scaffolding.

## Commands

```sh
cargo build --workspace                                   # build the Rust workspace
cargo run -p nd                                           # run the TUI shell (not implemented in P1-M3)
cargo run -p nd -- serve                                  # run the server shell (not implemented in P1-M3)
cargo test --all-features                                 # run the test suite
cargo fmt --all                                           # format Rust code
cargo clippy --all-targets --all-features -- -D warnings  # lint with warnings denied
swift build --package-path helpers/apple-bridge           # build the Apple helper
swift test --package-path helpers/apple-bridge            # test the Apple helper with fakes
```

## Control API diagnostics

P1-M6 writes versioned, content-free server records beneath the Git-ignored
`.local/logs/control-api/` directory. Codex and Claude Code may inspect only
that log directory for diagnosis; do not change its permissions and do not
read the separate Application Support runtime directory or its control token.
Keep inspection bounded, for example:

```sh
tail -n 200 .local/logs/control-api/control-api.log
rg 'level=(WARN|ERROR)' .local/logs/control-api/
```

Records use `ndlog-v1` with typed ASCII fields. Treat diary text, source
metadata or paths, credentials, request data, and arbitrary errors in a record
as a privacy defect.

## Code conventions

- Format Rust code with rustfmt; do not hand-format around it.
- Keep Clippy clean with all warnings denied.
- Prefer explicit error handling and propagate errors with context.
- Keep terminal interaction separate from diary domain logic.

## Dependencies

Approved dependencies must have licenses compatible with Apache-2.0.

## Documentation

- `README.md` is user-facing; `AGENTS.md` holds contributor rules; `docs/` holds plans and design; `docs/archive/` holds inactive docs.
- Use lowercase kebab-case filenames. Start each doc with a title, `Status:` line, and one- to three-sentence summary.
- Add every doc to [docs/README.md](docs/README.md). Follow [docs/development-process.md](docs/development-process.md) for phase and milestone plans.
- Update affected docs with code. Update `README.md` for user-visible changes and this file for tooling, commands, or decisions.
- Link instead of copying facts.
- Archive completed or superseded docs: update status, `git mv` them under `docs/archive/` (completed phase directories go under `docs/archive/phases/`), and fix links. Archived docs are not maintained.

## Token efficiency

- Skip build/dependency output (`target/`), lockfiles, `LICENSE`, and `docs/archive/` unless needed. Search with `git grep` or `rg`.
- Start at `docs/README.md` and read only relevant docs or sections. For a long doc such as the architecture proposal, list headings with `rg -n '^#' <file>` and read one section.
- Keep logs short with quiet flags or filtering. Run affected tests while iterating and the full gate once at the end.
- Consult only the needed dependency source/module for API details.
- Make targeted edits; inspect `git diff --stat` before a full diff.
- Reply with concise summaries, not raw logs.
