# Notion ascending date order design

Status: Implemented (P1-M20).

Places a new daily Notion section in ascending date order within its month page instead of always appending it at the bottom. Existing out-of-order sections are left untouched.

## Intent

- **Outcome:** a month page reads `2026-10-28`, `2026-10-29`, `2026-10-30`, and so on, even when days are published out of order.
- **Owner decisions (2026-10-30):** ascending order by diary date; existing out-of-order sections stay where they are (Notion cannot move blocks, and re-creating them risks data loss).
- **Success:** publishing a day whose date falls between two existing sections inserts it between them; a day later than all existing sections still goes last; a day earlier than all goes first.

## Design

### Placement

- The adapter already paginates the month page and finds same-day sections. It now also parses every `heading_2` whose text is exactly `YYYY-MM-DD (DDD)` into a dated section. A section ends at the next `heading_1` or `heading_2`, the same boundary read-back already uses, so unrelated headings also delimit sections.
- The insertion point is the end of the dated section with the greatest date strictly less than the new day (the last such section on a page with equal dates). If no earlier section exists, it is just before the dated section with the smallest later date. With no dated sections it appends at the end, as today.
- The Notion append-children `position` is `after_block` for a block anchor, `start` when the new section must precede the page's first block, and `end` when the anchor is the last block (so a sorted page behaves exactly as before). `start` is documented for API `2026-03-11`.
- Only the first chunk uses the computed insertion point; later chunks chain after the last verified block, as they do today.
- Sections already out of order are not reordered; the rule only guarantees the new section never makes the page less sorted.

### Preview, receipt, recovery

- The Notion preview shows the placement (`before YYYY-MM-DD`, `after YYYY-MM-DD`, or `end`) beside the path. This adds an optional `placement` field to the Notion preview DTO and bumps the contract to version 7.
- No receipt or SQLite change: the receipt keeps the returned block IDs, and inspection and reconciliation find the canonical heading wherever it sits, so a lost reply is adopted rather than replayed. Duplicate or non-canonical same-day headings stay a conflict.
- No digest change is needed: the observed-state digest already hashes every block ID and text on the month page, so an anchor that moved between approval and write voids the plan as a conflict and needs a new preview.

### Docs

- Update the "Notion publication format" decided item in `AGENTS.md`, `architecture.md` (Publication and recovery), `product-behavior.md`, `ux-tui.md`, and P1-M10 references that say the section is appended at the bottom.

## Testing

- Unit: anchor selection for first, middle, last, empty, and already out-of-order pages; non-dated blocks between sections; duplicate headings.
- Connector fixtures: insertion request shape (`after_block`, `start`), 100-block chunking, read-back of the inserted section.
- Recovery: lost reply after insertion resumes without a duplicate; anchor drift between approval and write is a re-preview.

## Out of scope

- Re-sorting existing sections; changing the heading format.
