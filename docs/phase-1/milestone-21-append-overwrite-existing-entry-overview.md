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
- [ ] Notion: Append adds the separator and new body to the existing section;
  Overwrite replaces the section's body (insert, read back, then trash); a
  moved or edited section falls back to Conflict; duplicates stay a conflict.
- [ ] Preview shows the commands only when an entry already exists, the exact
  result, and the second confirmation for Overwrite.
- [x] Approval, publish, mirror retry, and recovery all work for a chosen mode
  (local file and iCloud mirror; Notion follows in the next step).
- [ ] `AGENTS.md`, architecture, product behavior, Terminal UX, user guide, and
  the changelog describe the commands; the "never overwrite" decision is
  updated.
- [ ] Gate passes: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`.

## Progress

Local file and iCloud mirror are implemented and tested (`apps/nd/tests/existing_entry.rs`,
`diary-core` Append composition, `LocalOutput::backup`, TUI `existing_entry_view`).
Notion Append and Overwrite, and the remaining docs, are still open.

## Acceptance evidence

Recorded when the checklist is complete.
