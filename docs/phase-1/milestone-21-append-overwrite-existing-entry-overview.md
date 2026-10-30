# P1-M21: Append and Overwrite an existing entry

Status: In progress

This milestone adds **Append** and **Overwrite** commands to the Preview stage
for every output connector (local file, iCloud mirror, Notion) when a diary
entry already exists for the day. The owner approved the behavior on
2026-10-30; the [design](../superpowers/specs/2026-10-30-append-overwrite-existing-entry-design.md)
holds the rules. It narrows "never overwrite" to "never overwrite without an
explicit, confirmed choice".

## Goal

Where today a differing file or Notion section is a dead-end Conflict, Preview
offers Append (keep the existing entry, add the new text under a dated
separator) and Overwrite (replace it, with a backup for files). Nothing is
written until the person approves the resulting exact plan.

## Scope

- Two phases, as publication already works. The local preview (local file and
  iCloud mirror) offers the choice on a local or mirror conflict; the Notion
  preview offers its own choice on a Notion conflict.
- The iCloud mirror always ends byte-identical to the final local file. Differing
  mirror content is backed up and replaced under a digest guard.
- Append adds `---` and `*Appended YYYY-MM-DD HH:MM*` (Mac-local time, fixed when
  the plan is built and shown in Preview) before the new text. Overwrite needs a
  second Enter in the TUI and copies the old file to `<name>.md.bak-<digest12>`
  first.
- No SQLite migration: the plan's creation time and payload digests imply the
  mode, so approval, publication, and recovery re-derive the exact bytes.
- Contract version 8: an optional preview request body and the echoed mode.

## Completion checklist

- [x] Local: Append and Overwrite produce the exact planned bytes, are written
  atomically only while the file still matches the previewed digest, and a
  changed file falls back to Conflict.
- [x] Overwrite backs up the old file first; a failed or unverifiable backup
  blocks the write and leaves the file untouched.
- [x] iCloud mirror: ends byte-identical to the final local file; differing
  mirror content is backed up first; mirror failure never fails local.
- [x] Notion: Append adds the separator and new body to the existing section;
  Overwrite replaces the section's body (insert, read back, then trash); a
  moved or edited section falls back to Conflict; duplicates stay a conflict.
- [x] Preview shows the commands only when an entry already exists, and the
  exact result with the chosen mode.
- [ ] Overwrite asks for a second Enter and Esc cancels it. Implemented in the
  TUI event loop but not covered by an automated test; verify in a real
  terminal before closing the milestone.
- [x] Approval, publish, mirror retry, and recovery all work for a chosen mode
  (local file, iCloud mirror, and Notion).
- [x] `AGENTS.md`, architecture, product behavior, Terminal UX, user guide, and
  the changelog describe the commands; the "never overwrite" decision is
  updated.
- [x] Gate passes: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`.

## Acceptance evidence

- `apps/nd/tests/existing_entry.rs` (11 tests) covers the local file and iCloud
  mirror against the real `LocalOutput`: a differing file is still a conflict
  unless a mode is chosen; Append keeps the existing bytes and adds the blank
  line, `---`, dated line and body; Overwrite backs up to the digest-named copy
  then replaces; a chosen mode is ignored when nothing exists; a file edited
  after the preview or after approval voids the plan and nothing is written; a
  backup that cannot be written leaves the file untouched; the mirror ends
  byte-identical to the final local file with its old content backed up; a
  mirror conflict from a plain preview is never replaced, even on retry; and a
  blocked mirror retries with the re-derived Append bytes.
- `apps/nd/tests/existing_notion.rs` (6 tests) drives the Notion preview,
  approval and publication with a stateful fake: Append and Overwrite results,
  the receipt vouching for the whole resulting section, a section edited after
  the preview or after approval refused for both modes, and a chosen mode doing
  nothing when the day has no section.
- `notion/tests.rs` covers the adapter: observing a canonical section and
  refusing a non-canonical one, Append placement after the section's last block
  with an italic dated line, a replay inserting nothing twice, the previewed
  result digest equal to the digest after the write, a section edited since the
  preview refused, a long Append chunked and resumed after a lost reply without
  duplicates, and Overwrite keeping the heading and its neighbors.
- `diary-core` covers the Append composition (missing final newline, CRLF, empty
  file, the blank line before `---`); `local_output` covers the backup (exact
  bytes, idempotent replay, an interrupted copy rewritten, a changed file
  refused); `session_runtime/tests.rs` covers when the commands are offered in
  each phase and the Preview screen lines.
- Contract version 8: the route-inventory test and the real server's `/health`.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features
  -- -D warnings`, and `cargo test --all-features` (591 tests) passed on
  2026-10-30. The Swift helper is untouched, so its gate does not apply.
- Not covered: a live Notion workspace (the adapter is exercised against the
  fake remote only), the TUI second-Enter confirmation (above), and recovery of
  an Append interrupted by a crash, which resolves as a conflict that needs a
  new preview.
