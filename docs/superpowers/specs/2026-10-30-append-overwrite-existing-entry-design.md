# Append and Overwrite existing entry design

Status: Approved for implementation (P1-M21, owner 2026-10-30).

Adds Append and Overwrite as explicit Preview resolutions when a destination already holds content the app did not write, narrowing the "never overwrite" rule to "never overwrite without an explicit, confirmed choice".

## Intent

- **Outcome:** when a diary entry already exists for the day, the person can keep it and add the new text (Append) or replace it (Overwrite), at local, iCloud, and Notion, from the Preview stage.
- **Owner decisions (2026-10-30):** applies to all three destinations; Append inserts a `---` separator and an `*Appended YYYY-MM-DD HH:MM*` line; Overwrite needs an extra confirmation and a local backup; the choice is made per publication phase and the iCloud mirror always follows the final local bytes.
- **Success:** a Conflict on any destination offers Append and Overwrite beside reread, discard, and save-under-another-day; nothing is written until the person approves the resulting plan; changed content since the choice falls back to Conflict.

## Design

### Two phases, one choice each

Publication is two-phase: local file and iCloud mirror are previewed, approved and written first; Notion gets its own preview and approval after the local file is verified. So the choice is made in the phase where the conflict shows:

- **Local preview** (`POST /v1/workflows/{id}/output-previews`): an optional body `{"existing_mode":"append"|"overwrite"}`. Without it a differing local or mirror file is a Conflict, as today. With it, every destination of this phase that holds different content is planned for the chosen mode.
- **Notion preview** (`POST .../notion-previews`): the same optional body, applied to a Notion conflict on a single canonical same-day section. Duplicate, ISO-only or wrong-weekday headings remain a plain conflict with no commands.

Resolutions in this code base are routes and request bodies, not an enum, so the choice is a request field, not a new conflict-resolution type. Reread, discard and save-under-another-day are unchanged.

### No stored mode

No schema change. The plan's creation time and payload digests imply the mode, so approval, publication, mirror retry and recovery re-derive the exact bytes:

- The planned local bytes are the plain rendering (Overwrite, or no existing file), the current file when it already equals the planned digest (after publication), or the existing file plus the Append block built from the plan's `created_at`.
- Existing content is bound by the plan's observed-state digest (the digest of the file the person saw), so any change since the preview voids the plan as Conflict.

### Append, local and iCloud

The result is the existing bytes, a newline if missing, then a blank line, `---`, a blank line, `*Appended YYYY-MM-DD HH:MM*`, a blank line and the new body (a blank line before `---` keeps it a thematic break and not a heading underline). The existing front matter is not touched; a hand-written file without front matter stays without. The time is Mac-local, taken once from the plan's creation time and shown in Preview, so approval binds exactly what was previewed.

### Overwrite, local and iCloud

The current file is copied to `<name>.md.bak-<first 12 hex of its digest>` beside it (the name never ends in `.md`, so it is not mistaken for an entry), read back and compared, and only then replaced atomically while the file still matches the previewed digest. A failed or unverifiable backup blocks the write with the file untouched. The backup name is deterministic, so a replay finds the same backup.

### iCloud follows local

The mirror always receives the final local bytes, so the decided byte-identical rule holds. A mirror file that differs from those bytes is backed up and replaced under its own previewed digest; a mirror with no content is created as before. A mirror failure never changes local.

### Notion

- **Append:** the existing section stays. Notion's projection has no divider block, so the separator is a literal `---` paragraph, then an italic `Appended YYYY-MM-DD HH:MM` paragraph, then the new body blocks, inserted after the section's last block (chunked, read back).
- **Overwrite:** the section's body is replaced with the amendment sequence (insert, read back, then trash), guarded by the digest of the section the person saw. The section's blocks need no receipt: the guard is the previewed block IDs and texts, re-read at write time.
- An interrupted Overwrite leaves duplicate content, never lost content (the old blocks are only trashed after the new ones read back); a repeat Overwrite after a re-preview clears it. Notion's trash is the safety net, so there is no backup file.
- A completed Append or Overwrite saves ordinary receipts for the resulting section.

### Receipts and recovery

Completed writes record ordinary receipts (version 1 for a day with no earlier receipt). An interrupted file replace cannot leave a partial file (atomic rename); if it never happened the file still holds the previewed content and the intent resolves as Conflict for a new preview.

### TUI

Preview lists Append and Overwrite after the existing controls, only while a local, iCloud or Notion conflict shows the existing content (or a mode is already chosen, so the person can switch). They are reached by focus and Enter, as every control is. Overwrite asks for a second Enter and names the backup. Existing and replacement content, the chosen mode, and the exact result are shown.

### Docs

Update `AGENTS.md` (the "never overwrite" wording and the iCloud mirror line), `architecture.md`, `product-behavior.md`, `ux-tui.md`, the user guide and the changelog.

## Testing

- Core: Append composition (no trailing newline, hand-written file without front matter, empty file, CRLF kept) and the blank line before `---`.
- Local and iCloud: exact bytes, atomic replace, a changed file voids the plan, backup content equals the original, a failed backup blocks the write, the mirror follows the final local bytes, a mirror conflict is backed up.
- Notion: Append extends the section and reads back; Overwrite insert-then-trash; a moved section is a Conflict; duplicates offer nothing; interrupted Overwrite keeps all content.
- Control API and TUI: the optional request body, the commands only when an entry exists, the second confirmation, and the mode surviving approval, publish and mirror retry.

## Out of scope

Merging or diffing text; different modes per destination in one phase; changing amendments of completed days.
