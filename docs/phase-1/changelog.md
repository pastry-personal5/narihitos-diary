# Phase 1 changelog

Status: Active

This changelog records owner decisions and material plan changes for Phase 1.

## Entries

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
