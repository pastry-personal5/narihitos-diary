# Notion ascending date order design

Status: Draft (proposed P1-M20). Awaiting owner review.

Places a new daily Notion section in ascending date order within its month page instead of always appending it at the bottom. Existing out-of-order sections are left untouched.

## Intent

- **Outcome:** a month page reads `2026-10-28`, `2026-10-29`, `2026-10-30`, and so on, even when days are published out of order.
- **Owner decisions (2026-10-30):** ascending order by diary date; existing out-of-order sections stay where they are (Notion cannot move blocks, and re-creating them risks data loss).
- **Success:** publishing a day whose date falls between two existing sections inserts it between them; a day later than all existing sections still goes last; a day earlier than all goes first.

## Design

### Placement

- The adapter already paginates the month page and finds same-day sections. It now also parses every `heading_2` matching `YYYY-MM-DD (DDD)` into `(date, heading_block_id, last_body_block_id)`.
- The insert anchor is the last block of the section with the greatest date strictly less than the new day. If none exists, the section is inserted before the first dated section (anchor: the block preceding it, or the page start if nothing precedes it). With no dated sections it appends at the end, as today.
- Insertion uses append-children `position` of type `after_block` (at most 100 blocks per request), the same primitive the amendment path uses. A page with no dated section before the new day and no preceding block uses the `start` position.
- Sections already out of order are not reordered; the anchor rule only guarantees the new section never makes the page less sorted.
- Non-dated blocks between sections are treated as part of the preceding section's body only up to the next dated heading; unrelated hierarchy content stays allowed.

### Preview, receipt, recovery

- The Notion preview shows the insert position (`after YYYY-MM-DD` or `first` or `end`) beside the existing path display.
- The receipt records the anchor block ID. Reconciliation re-reads the month page and looks for the heading of this day; it never replays blindly. Duplicate same-day headings stay a conflict.
- The plan digest includes the position so an approved plan is invalid if the anchor changed before the write.

### Docs

- Update the "Notion publication format" decided item in `AGENTS.md`, `architecture.md` (Publication and recovery), `product-behavior.md`, `ux-tui.md`, and P1-M10 references that say the section is appended at the bottom.

## Testing

- Unit: anchor selection for first, middle, last, empty, and already out-of-order pages; non-dated blocks between sections; duplicate headings.
- Connector fixtures: insertion request shape (`after_block`, `start`), 100-block chunking, read-back of the inserted section.
- Recovery: lost reply after insertion resumes without a duplicate; anchor drift between approval and write is a re-preview.

## Out of scope

- Re-sorting existing sections; changing the heading format.
