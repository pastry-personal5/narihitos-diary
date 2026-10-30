# P1-M14: iCloud output overview

Status: Done

This milestone mirrors each published diary entry to a configured iCloud Drive
folder as a byte-identical Markdown file, automatically and quietly, without
changing the local or Notion outputs. Design: the
[iCloud output spec](../superpowers/specs/2026-10-30-icloud-output-design.md);
task plan: [iCloud output plan](../superpowers/plans/2026-10-30-icloud-output.md).

## Goal

With `paths.icloud_entries_root` configured, approving the local plan produces a
verified iCloud file with no extra prompt. Without it, nothing changes.

## Entry criteria

- P1-M9 and P1-M10 are `Done`.

## In scope

- `DestinationKind::Mirror` (destination `icloud`) and a mirror retry transition.
- Optional config key, iCloud-validated output root, evicted-placeholder rule.
- Automatic mirror write after the verified local write, failure isolation,
  startup reconciliation, and a mirror-only retry.
- Additive `mirror` destination kind in the Control API and quiet TUI status.

## Out of scope

- CloudKit, the Swift helper, making iCloud the source of truth.
- Mirror replacement during amendments: P1-M11 (amendments) is still `Planned`,
  so no replace path exists to extend. P1-M11 must cover the mirror receipt.
- A user-facing command that triggers `publish_mirror_guarded`; the service
  method and its tests exist, wiring a trigger is left to a later milestone.

## Completion checklist

- [x] `Mirror` kind round-trips and mirror states never block Notion approval
  (diary-core `mirror_*`, `destination_kind_mirror_round_trips`).
- [x] `finish_mirror_retry` completes, rejects wrong states, never regresses
  (diary-core `finish_mirror_retry_*`).
- [x] Config key parses, is optional, rejects duplicates (diary-control).
- [x] Root outside Mobile Documents, `..`, and symlink escape are rejected;
  evicted placeholder and vanished root are unavailable and never written
  (diary-connectors `new_icloud_*`, `icloud_*`).
- [x] Byte-identical mirror completes the day with no extra approval; no mirror
  configured leaves behavior unchanged (`apps/nd/tests/p1_m14_icloud.rs`).
- [x] Missing root, placeholder, and different content leave local published and
  the day `Partial`/unaffected; Notion still previews and publishes.
- [x] Ambiguous mirror write reconciles at startup without a second write.
- [x] Mirror retry after a blocked mirror completes the day.
- [x] A config change between preview and publish forces a new preview.
- [x] SQLite round-trip of the `mirror` kind passes (no migration needed or one
  added), contract serializes `mirror`, TUI shows mirror only when not verified.
- [x] Docs updated (`docs/README.md`, product-behavior, architecture,
  control-api-v1, ux-tui, user-guide-tui, README, changelog, roadmap, AGENTS.md).
- [x] Gate passes: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`.
