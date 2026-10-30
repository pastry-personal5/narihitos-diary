# AGENTS.md

Instructions for contributors and AI coding agents. This is the single source of agent instructions; `CLAUDE.md` imports it with `@AGENTS.md`.

## Project

`narihitos-diary`: Make remembering a day feel effortless.

Stack: Rust, plus one Swift helper process for Apple APIs.

See [README.md](README.md) for the overview. Setup, build, and validation commands are in [docs/contribution-guide.md](docs/contribution-guide.md); other docs are indexed at [docs/README.md](docs/README.md).

## Agent workflow

- Work only on a clear task in the active milestone. Read its plans, relevant design docs, and applicable "Undecided" items first.
- Before implementation, add a verifiable completion checklist to the milestone overview. Check items only when evidence exists.
- Keep implementation, tests, and affected docs together. A milestone is done only when its checklist is complete and the gate passes: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-features`, plus `swift build` and `swift test` for the helper once `helpers/apple-bridge` exists, unless its overview records an explicit owner-approved exemption. If a required command cannot run, leave its item unchecked and report the blocker.
- Ask before building on an undecided item, changing dependencies or build tools, deleting user data, or broadly restructuring the repository. Other work within the approved task and milestone may proceed.
- Follow the active phase and milestone order in `docs/roadmap.md` unless the phase plan says otherwise. Create the roadmap when planning the first phase.

## Decided — ask before changing

Each decision is recorded in full in the doc that owns its topic; this is the index. Ask before changing any of them, and record a newly resolved decision in its owning doc plus a one-line update here.

- **Stack and server architecture:** Rust + one Swift helper process; a local Control API Server owns workflows and connectors; Tokio async server with a synchronous TUI; the single binary is `nd` (`nd serve` starts the server, `nd` opens the TUI). See [docs/architecture.md](docs/architecture.md).
- **Hosted assistant provider:** the Phase 1 hosted default is OpenAI's Codex, reached only through the `codex app-server` JSON-RPC surface over stdio (never the TypeScript SDK, never bare `codex exec` flags). One registry member per configured `codex_home` profile; the first-listed profile is the default when more than one is configured. Each job spawns and kills its own `codex app-server` process, with a per-turn restricted sandbox (no readable roots, no network, no MCP servers, no web search, approval never, no history/rollout persistence). The account credential lives inside each profile's own `codex_home`; Rust never reads it. See [docs/architecture.md](docs/architecture.md) "Composition and egress" and [docs/phase-1/milestone-08-draft-assistants-overview.md](docs/phase-1/milestone-08-draft-assistants-overview.md).
- **Apple on-device adapter:** targets the latest stable macOS/Foundation Models SDK only (no older-OS deployment floor). Supports all four assistant actions (extraction, drafting, paragraph review, and Ayumi source refinement) for both en-US and ko-KR where the action produces entry prose; source refinement operates on en-US Ayumi text. This matches the Codex hosted default. Request and response payloads, including a drafted entry approaching the 512 KiB entry bound, travel over the Swift bridge's in-band frame with its cap raised to cover that bound, not the P1-M5 private handoff-file transport. See [docs/phase-1/milestone-08-draft-assistants-overview.md](docs/phase-1/milestone-08-draft-assistants-overview.md).
- **Product behavior:** source discovery and selection, composition and hosted data egress, draft/source retention, local and Notion publication, conflicts, and amendments. P1-M21 (owner-approved 2026-10-30) narrows "never overwrite" to "never overwrite without an explicit, confirmed choice": Preview offers **Append** (existing content kept, then a dated `---` separator and the new text) and **Overwrite** (file backed up first, then replaced) at every connector when an entry already exists. See [docs/product-behavior.md](docs/product-behavior.md) and [P1-M21](docs/phase-1/milestone-21-append-overwrite-existing-entry-overview.md).
- **iCloud mirror output:** optional `paths.icloud_entries_root` (under
  `~/Library/Mobile Documents/`) mirrors each local entry as byte-identical
  Markdown after the verified local write, under the same approval and isolated
  from local and Notion. With Append or Overwrite (P1-M21) it still ends
  byte-identical to the final local file, backing up differing content first. See
  [the design](docs/superpowers/specs/2026-10-30-icloud-output-design.md) and
  [P1-M14](docs/phase-1/milestone-14-icloud-output-overview.md).
- **Local output front matter:**- **Local output front matter:** P1-M9 uses YAML keys `date` and `language`. See [docs/phase-1/milestone-09-local-publication-recovery-overview.md](docs/phase-1/milestone-09-local-publication-recovery-overview.md).
- **Notion publication format:** P1-M10 adds a native `heading_2` formatted
  `YYYY-MM-DD (DDD)` with fixed uppercase English weekdays, followed by a
  lossless native-block projection of the confirmed Markdown body. P1-M20
  (owner-approved 2026-10-30) inserts it in ascending date order: after the
  greatest earlier dated section, or before the smallest later one; existing
  out-of-order sections are never rewritten. Unrelated hierarchy content is
  allowed; canonical year/month matches must be unique.
  See [docs/phase-1/milestone-10-notion-publication-overview.md](docs/phase-1/milestone-10-notion-publication-overview.md)
  and [P1-M20](docs/phase-1/milestone-20-notion-ascending-order-overview.md).
- **Ayumi source maintenance:** Sources-stage approval/edit writes the real
  note; explicitly confirmed deletion hard-deletes that note and only its
  safely resolved linked audio. Audio bytes are never read or sent. See
  [docs/product-behavior.md](docs/product-behavior.md).
- **Terminal UX:** canonical workflow stages and layout, keybindings, the Korean-IME input constraint, date display, and the target terminal. P1-M15 adds a right Utility Sidebar (Diary Day and Assistant Widgets); P1-M16 adds source-list letter shortcuts only while `IME: EN`, the one exception to no bare-letter hotkeys. P1-M17 keeps language and style across new workflows in one TUI session and restores the persisted assistant choice after restart. P1-M19 adds a manual, read-only readiness check for Notion and iCloud on the Publish stage. P1-M21 adds Append and Overwrite controls to Preview when an entry already exists, with a second Enter for Overwrite. See [docs/ux-tui.md](docs/ux-tui.md).
- **License:** Apache-2.0 for all project code.
- **Process:** numbered phases contain numbered milestones; milestone numbering restarts per phase. IDs use `P1-M2`. P1-M18 may break compatibility and change APIs during its refactor, with no numeric line or complexity limit; avoid very large files by responsibility. See [docs/development-process.md](docs/development-process.md) and [P1-M18](docs/phase-1/milestone-18-clean-architecture-refactor-overview.md).

## Undecided — ask before inventing

- **Notion connector:** the configured root page's structure remains
  uninspected. Validate the root and any existing year/month path before
  creating pages or appending a daily section.

The interview decisions and remaining page-compatibility check are mapped in the
[research summary](docs/phase-1/milestone-01-research-summary.md).

## Control API diagnostics

P1-M6 writes versioned, content-free server records beneath the Git-ignored
`.local/logs/control-api/` directory. Codex and Claude Code may inspect only
that log directory for diagnosis; do not change its permissions, do not run
commands with it as the working directory (an instrumented binary then drops
a stray `default.profraw` there, and startup refuses the unrecognized entry
with `diagnostic-unrecognized-entry`), and do not
read the separate Application Support runtime directory or its control token.
Keep inspection bounded, for example:

```sh
tail -n 200 .local/logs/control-api/control-api.log
rg 'level=(WARN|ERROR)' .local/logs/control-api/
```

Agents may run `nd serve` themselves to reproduce or verify a startup or server
problem (owner-approved 2026-10-30). Start it in the background from the
repository root, read only the bounded log output above, and stop the process
when done. This approval does not cover reading the Application Support
directory, the control token, or `diagnostic-tag-key`, and deleting persistent
state still needs explicit confirmation.

Failed-job `reason` tokens name the exact failed check, and Debug-level
`component=assistant event=snapshot` records describe workflow state with flags,
counts, and keyed correlation tags (details in the architecture doc). The tag
key file `diagnostic-tag-key` is in the Application Support directory; do not
read it.

Records use `ndlog-v1` with typed ASCII fields. Treat diary text, source
metadata or paths, credentials, request data, and arbitrary errors in a record
as a privacy defect.

## Code conventions

- Format Rust code with rustfmt; do not hand-format around it.
- Keep Clippy clean with all warnings denied.
- Prefer explicit error handling and propagate errors with context.
- Keep terminal interaction separate from diary domain logic.
- Never hard-code AI prompts; follow "AI prompts" in [docs/contribution-guide.md](docs/contribution-guide.md).

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
- Start at `docs/README.md` and read only relevant docs or sections. For a long doc such as the architecture doc, list headings with `rg -n '^#' <file>` and read one section.
- Keep logs short with quiet flags or filtering. Run affected tests while iterating and the full gate once at the end.
- Consult only the needed dependency source/module for API details.
- Make targeted edits; inspect `git diff --stat` before a full diff.
- Reply with concise summaries, not raw logs.
