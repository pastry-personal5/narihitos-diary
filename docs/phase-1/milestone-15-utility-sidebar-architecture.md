# P1-M15: Utility Sidebar architecture plan

Status: Done

This plan adds a right Utility Sidebar to the adaptive shell with two Widgets,
a read-only Diary Day Widget above an Assistant Widget that takes over assistant
backend selection, using only existing Control API calls.

## Affected modules and interfaces

- `diary-tui` `focus.rs`: add `FocusZone::Utility`, `SidebarKind::Utility`, a
  widget row index, and extend `next_zone`/`previous_zone` to screen order
  Workflow, Resources, Main, Utility.
- `diary-tui` `layout.rs`: `AdaptiveAreas` gains `utility_sidebar:
  Option<Rect>`; `WidthBand` gains a band between Wide and Medium. Add
  `UTILITY_SIDEBAR_WIDTH` (24).
- `diary-tui` new `widgets.rs`: an ordered list of Widgets, each with a title,
  a render function, key handling, and a focusable row count (zero for
  read-only). `DiaryDayWidget` then `AssistantWidget` exist. No registration or
  configuration system.
- `diary-tui` `assistant_view.rs`: remove the `AssistantResource::Backend` rows
  from `resources()`; expose the backend rows and "None (manual only)" to the
  widget. Selection state still comes from `WorkflowDto.assistant_backend_id`.
- `diary-tui` `events.rs`, `worker.rs`, `client/mod.rs`: route Enter in the
  Utility zone to the existing `set_assistant_selection` with the workflow
  revision ETag; `None` row sends `backend_id: None`. No new endpoint.
- `diary-control`, `diary-control-contract`, SQLite: unchanged.

## Layout bands

The body today is Workflow 24 + Resources (`body/4 + 16`) + Main. Adding a
24-column right sidebar at 120 columns leaves the Main Pane about 26 columns,
which is unusable, so the inline threshold is not 120.

| Band | Columns | Inline | Overlay (focused only) |
|---|---|---|---|
| Full | ≥ `FULL_COLUMNS` (151) | Workflow, Resources, Main, Utility | none |
| Wide | 120 to 150 | Workflow, Resources, Main | Utility |
| Medium | 100 to 119 | Workflow, Main | Resources, Utility |
| Narrow | 80 to 99 | Main | Workflow, Resources, Utility |

The floor is the Main Pane width the shipped Wide band already accepts: 120
columns leave it 50 (`MIN_MAIN_PANE_WIDTH`). `FULL_COLUMNS` is the narrowest
width that keeps the Main Pane at or above that floor with all four columns
inline: 151 (Main 50 at 151; at 150 the fourth column would leave 49). A layout
test pins both sides of the threshold. Existing 80/100/120 behavior is otherwise
unchanged. An overlay is right-aligned and clears the body beneath it, like the
Resources overlay.

## Diary Day Widget behavior

- Input: the open `WorkflowDto` date and state. Rendering uses `format_diary_day`
  with the Widget's inner width, so the weekday drops when it does not fit.
- Shows the date and the state word as the server reports it. There is no
  `today` marker: "today" is server-owned and the client has no date source.
  With no workflow it shows "No day chosen".
- Read-only with zero focusable rows; focus movement skips it. It reads only
  state the client already holds, so no request or contract change.

## Assistant Widget behavior

- Rows: each backend from capabilities as `<display label> · <composition
  path>` with a state word (`selected`, `available`, `unavailable`,
  `unsupported language`, `selected; unavailable`), reusing the current state
  rules; then "None (manual only)", selected when `assistant_backend_id` is
  null. State is words, never color alone.
- Up/Down move, Enter selects. Selecting an unavailable backend, or the row that is
  already selected, is refused with a short message; an unsupported-language
  backend stays selectable, as today.
- Read-only when there is no workflow, an assistant job is active, or the
  connection is lost; the widget says why.
- Manual editing stays always available and is not a backend.

## Approach and sequence

1. Fix the Main Pane floor from the shipped layout and set `FULL_COLUMNS`.
2. Failing layout and focus tests, then the focus and layout changes. Keep the
   existing 80/100/120 assertions passing.
3. Add the Widget list, `DiaryDayWidget`, and `AssistantWidget` rendering with snapshot-style
   buffer tests.
4. Move backend rows out of the Resources Sidebar; keep the Compose, Setup
   check, and Status Line labels; update affected tests.
5. Wire Enter and the read-only conditions; test stale revision, refused
   selection, and the `None` row against the existing server test harness.
6. Add real-key tests for the new zone and for Korean-IME editor input.
7. Update docs and statuses, then run the gate.

## Risks

- A fourth column makes the layout dense. Mitigation: the measured threshold
  and the overlay fallback.
- Moving the picker could orphan a Compose hint or keybinding text; step 4 greps
  `ux-tui.md`, `user-guide-tui.md`, and the Command Line help for it.
