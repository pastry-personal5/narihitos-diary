# Append and Overwrite existing entry design

Status: Draft (proposed P1-M21). Awaiting owner review. Depends on P1-M20 for Notion insertion logic.

Adds Append and Overwrite as explicit Preview resolutions when a destination already holds content the app did not write, narrowing the "never overwrite" rule to "never overwrite without an explicit, confirmed choice".

## Intent

- **Outcome:** when a diary entry already exists for the day, the person can keep it and add the new text (Append) or replace it (Overwrite), at local, iCloud, and Notion, from the Preview stage.
- **Owner decisions (2026-10-30):** applies to all three destinations; Append inserts a `---` separator and an `*Appended YYYY-MM-DD HH:MM*` line using the local clock at approval; Overwrite needs an extra confirmation and a local backup.
- **Success:** a Conflict on any destination offers Append and Overwrite beside reread, discard, and save-under-another-day; nothing is written until the person approves the resulting plan; changed content since the choice falls back to Conflict.

## Design

### Resolutions and plans

- Add `Append` and `Overwrite` to the conflict resolution set in `diary-control-contract` (new contract version). Choosing one builds a new plan bound to the digest of the existing content the person saw, plus `existing_mode` per destination and `appended_at`; all are part of the approved plan digest.
- The choice applies to every destination currently holding existing content. Destinations without existing content publish normally. The existing local-first order, `Partial` state, and retry-alone behavior are unchanged.
- Server logic lives in a new `existing_entry` command module beside `amendment`, reusing the digest-guarded replace primitives. Existing-content ownership is the plan's content digest rather than a receipt, so the amendment ownership rule is untouched.

### Append

- Local and iCloud: existing bytes (front matter kept once, nothing above the separator rewritten), then `---`, `*Appended YYYY-MM-DD HH:MM*`, and the new body, written atomically only while the file still matches the observed digest.
- Notion: the existing section stays; a divider block, the italic Appended line, and the new body blocks are added at the end of that section via `after_block`. No second heading.

### Overwrite

- Local and iCloud: copy the current file to a backup beside it (`YYYY-MM-DD.md.bak-<timestamp>`), verify the copy, then replace atomically under the digest guard. A failed or unverifiable backup blocks the write.
- Notion: the amendment sequence (insert new body, read back, trash old blocks); Notion's trash is the safety net, so no backup file.
- Preview shows existing and replacement content per destination, and the TUI requires a second explicit confirmation naming each backup path.

### Receipts and recovery

- Completed writes record receipts as an amendment does (new version, `#v<n>` receipt ID, restarted purge window). Intents record the observed digest and, for Notion, old block IDs so reconciliation can trash whichever set is stale.
- A crash after the backup but before the replace leaves the original file intact; a crash mid-Notion-overwrite leaves duplicate content rather than lost content.

### TUI

- Preview's conflict view lists Append, Overwrite, reread, discard, and save-elsewhere. Overwrite opens a confirmation. Conflict, blocked, and partial states are shown plainly, with no modal for mirror failures.

### Docs

- Update the "Decided" index in `AGENTS.md`, `architecture.md` (Publication and recovery), `product-behavior.md`, `ux-tui.md`, and add a P1-M21 overview with a completion checklist and the standard gate.

## Testing

- Local and iCloud: separator and timestamp format, front matter preserved once, digest drift falls back to Conflict, backup failure blocks the write, backup content equals the original.
- Notion: append extends the section without a second heading (using P1-M20 anchors), overwrite insert-then-trash ordering, interrupted overwrite reconciliation.
- Control API and TUI: new resolutions in the contract, second confirmation, clock injection so previews are exact.

## Out of scope

- Merging or diffing text; per-destination different modes; changing amendments of completed days.
