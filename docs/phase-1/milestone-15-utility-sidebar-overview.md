# P1-M15: Utility Sidebar overview

Status: Done

This milestone adds a fourth TUI column, the multi-purpose right Utility
Sidebar, and ships two Widgets in it: a read-only Diary Day Widget above the
Assistant Widget, which replaces the backend rows that Compose currently lists
in the Resources Sidebar. Later milestones can add Widgets without changing the
layout again. Names are canonical in [UX terms](../ux-terms.md).

## Goal

The person can see the diary day they are working on and choose the assistant
backend, or none, from the same place in every stage. Layout becomes Workflow
Sidebar, Resources Sidebar, Main Pane, Utility Sidebar, with the Main Pane never squeezed below a usable width.

## Entry criteria

- P1-M7 and P1-M8 are `Done` (the layout and backend selection exist).

## In scope

- `FocusZone::Utility` and `SidebarKind::Utility`, a fixed 24-column right
  Utility Sidebar, and its place in the Tab/Shift-Tab cycle (screen order).
- Adaptive bands that keep the Main Pane usable (see the architecture plan);
  the inline threshold is set from a measured minimum Main Pane width.
- A minimal ordered Widget list in `diary-tui`: Diary Day Widget, then Assistant
  Widget.
- Diary Day Widget: read-only. Shows the workflow's diary day through
  `format_diary_day` (`YYYY-MM-DD`, with the weekday when it fits) and the
  workflow state word; shows "No day chosen" when no workflow is open. It stays visible in every stage,
  including while a source note with another date is inspected. Skipped by
  focus.
- Assistant Widget: backends from `GET /v1/assistant-backends` with label,
  composition path, availability and safe reason, and unsupported language; a
  "None (manual only)" row; Enter selects through the existing
  `PUT /v1/workflows/{id}/assistant-selection` (`backend_id: null` deselects).
- Read-only while an assistant job runs, while no workflow is open, or while
  the connection is lost.
- Compose, Setup check, and Status Line keep showing the selected backend label
  and composition path; the Resources Sidebar loses its backend rows.
- Docs: `ux-terms.md`, `ux-tui.md`, `user-guide-tui.md`, `README.md` if user-visible,
  `docs/README.md`, phase doc, changelog.

## Out of scope

- Control API, contract, or SQLite changes (the selection endpoint already
  accepts a null backend ID).
- Any Widget other than Diary Day and Assistant, and a widget plugin or configuration system.
- Client-side persistence of a backend choice made before a workflow exists
  (the existing last-backend preference is unchanged), and a `today` marker
  (the server owns "today"; the client has no date source).
- Changes to the six stages, workflow state machine, assistant policy, or the
  Korean-IME input constraint.

## Completion checklist

- [x] Main Pane floor (50, from the shipped layout) recorded in the architecture plan and
  the inline four-column threshold set from it (a layout test fails if a
  narrower Main Pane would be produced at the threshold).
- [x] Layout tests at 80, 100, 119, 120, the threshold, and a wide width:
  correct inline/overlay set, Work Status four rows, no overlap.
- [x] Focus cycles Workflow → Resources → Main → Utility in both directions;
  a collapsed Utility Sidebar opens as a focused overlay and never becomes
  unreachable.
- [x] Diary Day Widget shows the date with weekday when it fits and the
  workflow state; shows "No day chosen" with no workflow; is
  never focusable; and follows the workflow when another is opened.
- [x] Assistant Widget rows show label, path, availability, safe reason, and
  unsupported-language state; "None (manual only)" is always present.
- [x] Enter on a backend or on "None" sends one conditional selection request;
  a stale revision is rejected
  (412) and never overwrites; the request carries the workflow ETag (client
  tests; core `expect_revision`).
- [x] Assistant Widget is read-only (selection refused with a visible reason) with no
  workflow, during an assistant job, and when disconnected.
- [x] Resources Sidebar no longer lists backends; Compose, Setup check, and
  Status Line still show the selected label and path.
- [x] The Utility zone adds no bare-letter keys (Tab, Shift-Tab, arrows, Enter
  only) and editor input is still dispatched before zone keys, so Korean-IME
  composition is unaffected (inspection of the `main.rs` dispatch order;
  real-IME runs remain the documented limit in P1-M13).
- [x] Docs updated (`docs/README.md`, `ux-terms.md`, `ux-tui.md`, `user-guide-tui.md`,
  phase doc, changelog, `README.md` if user-visible).
- [x] Gate passes: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`.
