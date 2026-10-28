# P1-M1 research and owner decision summary

Status: Active

P1-M1 research produced the evidence below. The owner interview confirmed or
replaced its product recommendations. Later owner corrections set a Notion
root/year/month page hierarchy and bottom-of-month placement for daily entries.

Later decisions superseded some rows; the [changelog](changelog.md) and
[AGENTS.md](../../AGENTS.md) win. Superseded: composition is hosted by default
with Apple on-device optional; credentials live in the repo-local
`config/global_config.yaml`; the IME label covers English and Korean only; and
background work runs on the server's Tokio layer, with bounded worker threads in
the TUI.

## Decision map

| Area | Owner decision | Evidence |
|---|---|---|
| Source discovery (owner-confirmed) | Automatically inspect `D-1`, `D`, and `D+1`; load any other day only by explicit date jump. | [Input format](../research/markdown-input-format.md), [synthetic matrix](../research/p1-m1-validation-evidence.md) |
| Accepted input (owner-confirmed) | Support UTF-8 with optional BOM and LF or CRLF; keep readable bodies selectable with a warning when front matter is missing or malformed; ignore unknown fields and tolerate missing non-date fields; only valid offset-aware `created_at` supplies the header date. | [Input format](../research/markdown-input-format.md) |
| Selection stability | Make metadata/digest checks at workflow checkpoints; a detected change invalidates drafts and previews. An unverifiable check stops output until successful reread and reconfirmation. | [Operational constraints](../research/mvp-operational-constraints.md) |
| Audio | Drop supported audio-link nodes and targets; attachment count never changes eligibility. | [Input format](../research/markdown-input-format.md) |
| iCloud | Request download of selected cloud-only notes and coordinate their reads with a Foundation adapter. | [Input format](../research/markdown-input-format.md), [Apple evidence](../research/p1-m1-validation-evidence.md) |
| Composition | Use reviewed, guided two-pass drafting and a full multiline manual editor always, including after model failure. Recover unfinished drafts locally. (Originally Apple on-device only with no hosted provider; superseded: hosted is now the default and Apple on-device is optional.) | [Composition research](../research/agent-assisted-diary-composition.md), [prototype comparison](../research/p1-m1-validation-evidence.md) |
| Writing choices | Choose US English or South Korean Korean per entry, remembering the last choice. Offer concise, standard, or reflective style, defaulting to concise. Allow inferred reflections by default but mark them for review; confirm the complete edited text. | [Composition research](../research/agent-assisted-diary-composition.md) |
| Local output | Default to configurable `~/Documents/narihitos-diary/entries/YYYY/MM/YYYY-MM-DD.md` with YAML day/language front matter; atomically create, resume identical content, and never overwrite different content automatically. | [Operational constraints](../research/mvp-operational-constraints.md) |
| Persistent state | Store draft recovery and versioned publication receipts in a per-user SQLite database under Application Support. Keep credentials out of it. | [Operational constraints](../research/mvp-operational-constraints.md) |
| Notion hierarchy and payload | Use the configured root page, a `YYYY` child page, and a `YYYY-MM` child page, creating missing year/month pages when publishing. Append the confirmed Markdown body without YAML front matter under `heading_2: YYYY-MM-DD` at the bottom of the month page. | [Notion research](../research/notion-diary-section.md), [hierarchy fixture](../research/p1-m1-validation-evidence.md) |
| Notion duplicate/retry | Require a receipt plus read-back match in the target month page; stop on conflicting same-day content or ambiguous year/month paths; inspect relevant pages before retrying an ambiguous create or append, and honor `Retry-After`. | [Notion research](../research/notion-diary-section.md), [fixture matrix](../research/p1-m1-validation-evidence.md) |
| Environment | Apple Silicon, macOS 26.0+, Rust 1.89+. | [Rust TUI stack](../research/rust-tui-stack.md), [toolchain probe](../research/p1-m1-validation-evidence.md) |
| Background work | Dedicated bounded worker threads for filesystem, model, and blocking Notion work; typed completion events return to the sole TUI owner. (The server later moved to Tokio; the TUI keeps this model.) | [Rust TUI stack](../research/rust-tui-stack.md) |
| TUI/editor | Ratatui/Crossterm-compatible multiline plain-Markdown editing; one Crossterm version; in-memory tests for paste, key filtering, and rendering. | [Rust TUI stack](../research/rust-tui-stack.md), [toolchain probe](../research/p1-m1-validation-evidence.md) |
| Credentials | Store the Notion token in plaintext in an owner-only config file (now the repo-local, git-ignored `config/global_config.yaml`); the researched Keychain probe remains evidence, not the chosen storage. | [Rust TUI stack](../research/rust-tui-stack.md), [Keychain probe](../research/p1-m1-validation-evidence.md) |
| IME label | TIS notification plus language property; 500 ms polling fallback; `IME: ?` on unknown; never claim Japanese conversion mode. | [IME research](../research/macos-ime-language-status.md) |

## Remaining bounded uncertainty

- The owner deferred inspection of the configured Notion root page to a
  first-run read-only preflight. It must paginate, display the root and target
  year/month path, and fail closed on an incompatible or ambiguous existing
  path before creating pages or appending an entry.
- A follow-up synthetic fixture covers missing and existing year/month pages,
  pagination, duplicate or incompatible paths, and ambiguous page-creation
  responses. It does not prove live Notion page-creation behavior. The
  connector must test that request mapping on a disposable hierarchy before
  use with the configured root.
- Japanese sources were installed but not enabled during the live IME probe.
  The Rust probe read `ja` from installed Japanese sources and verified live
  notification delivery in both directions with another enabled non-English
  source. Japanese is now out of scope; implementation acceptance repeats
  `en ↔ ko` instead.
- One guided synthetic generation passed; this does not prove model quality on
  every diary. The manual editor, source-linked review, marked inferred
  reflections, runtime availability check, and manual path after failure are
  therefore part of the decision rather than optional polish.
- Real-terminal restoration after panic/cancellation cannot be proven by an
  in-memory backend. It remains an implementation acceptance test using only
  synthetic text.

The owner accepted the other checks as bounded for later implementation and
connector planning. The owner reaffirmed on 2026-10-28 that the configured
root is inspected only in the runtime read-only preflight.

## Privacy review

The research documents contain no real diary prose, precise location, audio,
attachment path, credential, Notion page ID, or private prompt. The probes used
invented text and a disposable Keychain value; the Keychain item was removed.
No source diary or Notion content was modified.

## Milestone boundary

No production dependency, Rust module, stable interface, or later Phase 1
milestone was added. P1-M1 retains its documented research-only exit-gate
exemption; the standard Cargo gate is not completion evidence.
