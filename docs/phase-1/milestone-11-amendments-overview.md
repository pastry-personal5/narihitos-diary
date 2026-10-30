# P1-M11: Amendments overview

Status: Done

This milestone adds the architecture's one intentional replacement path: a
completed entry can be amended, and only content the app itself wrote and
receipted is replaced. Release qualification of the assembled product is
[P1-M13](milestone-13-release-qualification-overview.md).

## Goal

Prove that a person can amend a `Completed` day, locally and in Notion, without
overwriting content the app does not own and without losing content on any
interruption.

## Entry criteria

- P1-M10 is `Done`: its receipts retain the Notion heading block ID, owned body
  block IDs, per-block digests, and the local file digest this milestone relies
  on. If any is missing, the first task is a forward-migrating receipt change
  recorded in the changelog.
- Treat [product behavior](../product-behavior.md) "Amendments" and the
  [architecture](../architecture.md) "Amendments" section as the authorities.

## Owner decisions fixed by this plan

- A conflict during an amendment offers only **reread** or **abandon the
  amendment**. Abandoning keeps the workflow `Completed` with its existing
  receipts and files untouched. Save-under-a-different-day is not offered
  because the day already has a completed entry.
- Replacement is local-first. The local file is replaced first. If the Notion
  replacement then fails or is uncertain, the local file keeps the new text, the
  workflow becomes `Partial`, and Notion shows pending or blocked. A
  Notion-only retry reuses the approval only while the plan is unchanged, and an
  uncertain write is inspected before any retry.
- The amendment editor is seeded with the local file **body** only. The
  `date` and `language` front matter is re-rendered by the app and is never
  offered for editing. A hand-edited front-matter block counts as a hand edit
  and is flagged by the banner.

## In scope

- Seed state: persist the seed-time local-file digest and the receipt version
  the amendment was seeded from, bind approval to them, and keep the original
  source digests in the receipt without reading sources.
- Hand-edit detection (file digest differs from the receipt) with a
  changed-since-publication banner that is not a conflict.
- Replacement plan and preview showing current and replacement content per
  destination; approval bound to seed-time and current destination digests.
- Atomic local replacement with an immediate pre-rename digest guard.
- Receipted Notion section replacement: locate by heading block ID, verify
  heading and body digests, insert new body blocks after the heading, read back,
  then trash old body blocks; persist intent with old block IDs.
- Interruption reconciliation that trashes only the stale set, never retries
  blindly, and never touches unrelated blocks.
- New receipt version on completion and restart of the seven-day text-retention
  interval.
- Control API amendment route and contract version bump with forward migration,
  and TUI amendment offer, banner, preview, Conflict view, and completion state.

## Out of scope

- Re-selecting sources, rerunning discovery, or assistant actions on the
  amendment's source text.
- Overwriting or adopting any unreceipted content, or a force-overwrite option.
- Save-under-a-different-day from an amendment conflict.
- New dependencies, broad restructuring, MCP, remote access, or other connectors.
- Whole-product qualification and live or human acceptance (see P1-M13).
- Live model calls, live Notion calls, or private diary prose in automated tests.

## Completion checklist

Every item is verified by automated tests or the standard gate. No item depends
on a human performing a manual run.

- [x] (Entry criterion waived by the owner on 2026-10-30 while P1-M10 is
  `In progress`; its live Notion checks remain open there.) P1-M10 is `Done`
  and its receipts are confirmed to carry the heading block
  ID, owned body block IDs and digests, and local file digest (or a recorded
  forward migration adds them).
- [x] Choosing a `Completed` day offers amendment instead of the existing-entry
  check, seeds the editor with the current local body, preserves the original
  source digests, waives source freshness, and never reads sources.
- [x] The seed-time file digest and receipt version are persisted; a hand-edited
  body or front matter is flagged by the banner and is not treated as a conflict.
- [x] Local amendment rechecks the seed-time digest immediately before the
  atomic rename and refuses any intervening change, leaving the file untouched.
- [x] Notion amendment locates only the receipted section by heading block ID,
  requires heading and body digests to match, inserts and reads back
  replacement blocks before trashing old body blocks, and reports a conflict on
  any ownership mismatch without writing.
- [x] Deterministic crash-boundary tests cover every local and Notion
  replacement boundary (temp file written, renamed, receipt not yet stored,
  Notion insert sent, read-back done, partway through trashing, new receipt not
  yet stored) and show no data loss, no blind replay, and no deletion of
  unrelated blocks.
- [x] A local success followed by a Notion failure or uncertain write leaves the
  workflow `Partial` with Notion pending or blocked; a Notion-only retry reuses
  the approval only for an unchanged plan.
- [x] A conflict offers exactly reread and abandon-amendment; abandoning leaves
  the `Completed` workflow, files, and receipts unchanged.
- [x] A completed amendment writes a new receipt version and restarts the
  seven-day purge, verified with an injected clock.
- [x] The Control API enables the amendment route, bumps the contract version
  with forward migration of persisted data, and the capabilities list and
  contract tests no longer treat amendment routes as disabled.
- [x] TUI state tests cover the amendment offer, banner, replacement preview,
  Conflict view, partial state, and completion over rendered state.
- [x] README, user guide, `architecture.md`, `product-behavior.md`,
  `control-api-v1.md`, and `ux-tui.md` reflect the shipped behavior.
- [x] The standard Rust and Swift gate passes, and the dated results are
  recorded here.

## Gate results

2026-10-30: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features --workspace` (456 tests), and `swift build` + `swift test` in `helpers/apple-bridge` (13 tests) all pass.

Known limits: the iCloud mirror is not updated by an amendment; the Notion paths are tested against fakes only (no live Notion call, per the plan).
