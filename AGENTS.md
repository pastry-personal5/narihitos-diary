# AGENTS.md

Instructions for contributors and AI coding agents. This is the single source of agent instructions; `CLAUDE.md` imports it with `@AGENTS.md`.

## Project

`narihitos-diary`: Make remembering a day feel effortless.

Stack: Rust.

See [README.md](README.md) for the overview. Setup and contribution rules are in [docs/contribution-guide.md](docs/contribution-guide.md); other docs are indexed at [docs/README.md](docs/README.md).

## Agent workflow

- Work only on a clear task in the active milestone. Read its plans, relevant design docs, and applicable "Undecided" items first.
- Before implementation, add a verifiable completion checklist to the milestone overview. Check items only when evidence exists.
- Keep implementation, tests, and affected docs together. A milestone is done only when its checklist is complete and the gate passes: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-features`, unless its overview records an explicit owner-approved exemption. If a required command cannot run, leave its item unchecked and report the blocker.
- Ask before building on an undecided item, changing dependencies or build tools, deleting user data, or broadly restructuring the repository. Other work within the approved task and milestone may proceed.
- Follow the active phase and milestone order in `docs/roadmap.md` unless the phase plan says otherwise. Create the roadmap when planning the first phase.

## Decided — ask before changing

- **Technical stack:** Rust.
- **Control architecture:** a local Control API Server owns workflows and
  connectors. The TUI is an HTTP client of that server; the design allows
  additional input and output connectors and a later MCP interface. See the
  [architecture proposal](docs/planned-mvp-architecture.md).
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
- **Composition:** offer reviewed two-pass Apple on-device drafting and an
  always-available multiline manual editor, including after model failure.
  Composition stays on the Mac. Offer US English or South Korean Korean per
  entry, remembering the last choice, and concise, standard, or reflective
  style with concise as the default. Mark model-inferred reflections for user
  review. Confirm the complete edited entry before output. Recover unfinished
  drafts from local persistent state.
- **Local output:** default to configurable
  `~/Documents/narihitos-diary/entries/YYYY/MM/YYYY-MM-DD.md`, with YAML front
  matter containing diary day and writing language. Never automatically
  overwrite different content at the same path.
- **Private state and credential:** use a per-user SQLite database in
  Application Support for draft recovery and publication receipts. Keep the
  Notion token out of the database; store it in plaintext in a per-user global
  config file requiring owner-only permissions.
- **Notion publication:** treat the configured page as a root. Under it, use a
  `YYYY` year subpage and a `YYYY-MM` month subpage, creating missing subpages
  when publishing. Send the confirmed Markdown body after stripping YAML front
  matter, under a `heading_2: YYYY-MM-DD` section at the bottom of that month
  page. Stop on conflicting same-day content, and inspect the relevant parent
  and month pages before retrying an ambiguous write. Require a fail-closed
  read-only root-page preflight at runtime.
- **Phase 1 platform:** Apple Silicon, macOS 26.0+, Rust 1.89+; bounded
  background workers; Ratatui/Crossterm with multiline editing; TIS input-
  language notifications with a 500 ms polling fallback and `IME: ?` when
  unknown.

## Undecided — ask before inventing

- **Notion connector:** the configured root page's structure remains
  uninspected. Validate the root and any existing year/month path before
  creating pages or appending a daily section.

The interview decisions and remaining page-compatibility check are mapped in the
[research summary](docs/phase-1/milestone-01-research-summary.md).

## Environment

- The stable Rust toolchain with Cargo.
- The rustfmt and Clippy components.

## Commands

```sh
cargo build                                               # build the application
cargo run                                                 # run the TUI application
cargo test --all-features                                 # run the test suite
cargo fmt --all                                           # format Rust code
cargo clippy --all-targets --all-features -- -D warnings  # lint with warnings denied
```

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
- Start at `docs/README.md` and read only relevant docs or sections.
- Keep logs short with quiet flags or filtering. Run affected tests while iterating and the full gate once at the end.
- Consult only the needed dependency source/module for API details.
- Make targeted edits; inspect `git diff --stat` before a full diff.
- Reply with concise summaries, not raw logs.
