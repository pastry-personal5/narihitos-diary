# P1-M20: Notion ascending date order

Status: Done

This milestone places a new daily Notion section in ascending date order inside
its month page instead of always appending it at the bottom. The owner approved
the behavior on 2026-10-30; the [design](../superpowers/specs/2026-10-30-notion-ascending-order-design.md)
holds the rules.

## Goal

A month page reads `2026-10-28`, `2026-10-29`, `2026-10-30`, and so on, even
when days are published out of order. Existing out-of-order sections are left
alone, because Notion cannot move blocks.

## Scope

- The Notion adapter computes an insertion point from the month page's
  `YYYY-MM-DD (DDD)` headings and inserts the first chunk of the new section
  there (`after_block`, `start`, or `end`). Later chunks keep chaining after the
  last verified block.
- The Notion preview shows the placement (`before YYYY-MM-DD`, `after YYYY-MM-DD`, or
  `end`).
- Contract version 7 adds the optional `placement` field to the Notion preview.
- No new dependency, connector, assistant prompt, SQLite migration, or receipt
  shape. Verification and reconciliation already find the heading wherever it
  sits.

## Completion checklist

- [x] Insertion point: a day later than every section goes last, earlier than
  every section goes first, and between two sections goes after the greatest
  earlier one; an empty or missing month page appends as before.
- [x] A page that is already out of order is not rewritten, and the new section
  never makes it less sorted.
- [x] A section longer than 100 blocks inserted mid-page reads back verified.
- [x] A lost reply and a moved anchor still reconcile or conflict without a
  duplicate section.
- [x] The Notion preview and the TUI show the placement; contract version 7.
- [x] `AGENTS.md`, architecture, product behavior, Terminal UX, user guide,
  Notion research, and the changelog describe ascending order.
- [x] Gate passes: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`.

## Acceptance evidence

- `notion/tests.rs` covers the insertion rule (`insertion_goes_after_the_greatest_earlier_section`,
  `a_later_day_goes_last_and_an_empty_page_appends`,
  `an_earlier_day_goes_before_the_first_later_section`,
  `an_unsorted_page_is_never_made_less_sorted`,
  `only_canonical_dated_headings_count_and_any_heading_ends_a_section`) and the
  adapter against the fake remote: a day inserted between two days, first, and
  last; a 121-block section inserted mid-page that reads back verified with the
  second chunk chained after the first; an unsorted page keeping its order; a
  preflight placement label with a moved anchor voiding the plan as a conflict;
  and a lost reply mid-page adopted without a second insert.
- The TUI test `notion_preview_shows_where_the_day_goes_on_the_month_page` covers
  the `Position` line, including a server that sends no placement.
- Contract version 7 is asserted by the route-inventory test and by the real
  server's `/health` response in `control_api_assembly`.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features
  -- -D warnings`, and `cargo test --all-features` (559 tests) passed on
  2026-10-30. The Swift helper is untouched, so its gate does not apply.
- Not covered by an automated test: a live Notion workspace. The `start` and
  `after_block` position types are taken from the current API reference and the
  existing amendment path, and are exercised only against the fake remote.
