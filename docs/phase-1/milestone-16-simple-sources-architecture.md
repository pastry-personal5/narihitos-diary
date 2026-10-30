# P1-M16: Simple Sources stage architecture plan

Status: Planned

This plan collapses the Sources stage's 15 controls into a flat contextual
menu and moves preparation into one Confirm Source List action, using only
existing Control API calls. The server needs no change (see the Confirm Source
List finding).

## Affected modules and interfaces

- `diary-tui` `sources_view.rs`: drop the `reviewed` gate and counter; add a
  menu model that lists the visible items for the current state; add a
  "confirm in progress" state that sequences prepare then confirm.
- `apps/nd` `main.rs`: replace the fixed `SOURCES_CONTROL_COUNT` menu and
  `sources_screen_body` with the flat menu, map the focused menu index to an
  action through the visible items, and start discovery on stage entry.
- `diary-tui` `worker.rs`, `client/mod.rs`: reuse the existing discover,
  prepare, preview, confirm, refine, approve, reject, edit, and delete intents.
  No new endpoint.
- `diary-tui` new `source_shortcuts.rs` (pure gate, key-to-action map, and the
  help text generated from it) and a `help_overlay` widget; `sources_view.rs`
  gains the hidden-ID set and
  `main.rs`, which feeds it the focus zone, stage, input modes, and the IME
  tracker's current state.
- `diary-control`, `diary-control-contract`, `diary-core`, SQLite: unchanged.

## Flat menu

Items are listed in this order and omitted when they do not apply. The focused
index is by visible position and is clamped when items appear or disappear.
"Shown" means the Main Pane displays the note's text (`active_detail`) and that
note is the focused row; moving focus to another row closes the shown text and
any suggestion, so Refine, Accept, Reject, Edit, and Delete can never act on a
note other than the one on screen. Enter or `v` shows a note.

| Item | Shown when |
|---|---|
| Include / exclude this note | A candidate is focused and readable or downloadable |
| View content | A readable candidate is focused and not already shown |
| Hide note | A candidate is focused |
| Show hidden notes | At least one note is hidden |
| Refine with assistant | A readable note is shown and a usable backend is selected |
| Accept refinement, Reject refinement | A refinement suggestion is shown |
| Edit note | A readable note is shown |
| Delete note and linked audio | A note is shown |
| Confirm Source List | Always; reads "Continue without notes" when nothing is included |
| Refresh notes | Always |
| Add a date | Always (the extra-date field opens in place; Enter runs discovery for that date, Esc cancels) |
| Load more | More candidates exist |
| Shortcuts help | Always (opens the Help Overlay) |
| Back to Start | Always |

Delete keeps the Enter/Esc confirmation naming the linked-audio consequence.
Enter on a note in the Resources Sidebar still shows its text and never
changes inclusion.

## Confirm Source List

Step 1 finding (read from `diary-control` on 2026-10-30; re-verify when
implementing):

- `POST /v1/workflows/{id}/source-preparations` (`prepare_sources`) reads each
  source, requests the iCloud download itself for a `NeedsDownload` source and
  rereads, and fails the job if the note is still not readable. The client never
  downloads. The job has a 120 s server timeout and a 64-source cap
  (`MAX_SELECTED_SOURCES`).
- `confirm_selection` needs only a `preparation_handle` plus the same
  `source_ids`. It does not require any per-note preview or review. The
  `reviewed` set is a client-only gate, so removing it needs no server change.
- The handle is single-use (removed when confirmation starts, even if the
  binding then mismatches) and bound to the workflow, its revision, and the
  `source_ids` vector compared in order. It also expires. Any failed
  confirmation therefore needs a fresh prepare.

Sequence:

1. Build one sorted `source_ids` list from the included set (hidden notes are
   never in it) and use that identical list for both prepare and confirm.
   More than 64 included notes disables the control with a visible reason.
2. Prepare, watch the job through Work Status, then confirm with the returned
   handle. No per-note preview is run. A preparation failure names the note
   when the server gives a reason (unreadable, permission, still downloading)
   and leaves the workflow unconfirmed.
3. A `preparation-expired` or `preparation-binding-changed` response, or a
   revision conflict, discards the handle and reports it; the next Confirm
   Source List starts again at prepare. The client never reuses a handle.
4. While Confirm Source List is running, View content and the inclusion
   controls are disabled; while a View job runs, Confirm is disabled. A View
   job uses its own single-note preparation and must not replace the handle the
   confirmation depends on (today's `preview_source_id` handling).
5. With no included note, the item reads "Continue without notes" and sends the
   existing explicit-empty confirmation after the existing explicit
   confirmation step.
6. Any later change to the included set, or an Accept/Edit/Delete, drops the
   preparation exactly as today, and a confirmed list still requires a new
   confirmation before any later output.

## Source-list shortcuts

Owner decision (2026-10-30): bare letters are allowed in exactly one place, the
Resources Sidebar in the Sources stage, and only while the IME indicator reads
`IME: EN`. This narrows the "no bare-letter hotkeys" rule in
[ux-tui.md](../ux-tui.md#input-constraint); everywhere else it still holds.

| Key (either case) | Action | Same as menu item |
|---|---|---|
| `i` | Include the focused note | Include / exclude (set to included) |
| `x` | Exclude the focused note | Include / exclude (set to excluded) |
| `v` | View the focused note's content | Enter on the note |
| `h` | Hide the focused note from the list | (no menu item; Hide) |
| `u` | Show hidden notes | (no menu item; Show hidden) |
| `r` | Refine with assistant | Refine with assistant |
| `n` | Reject the shown refinement | Reject refinement |
| `e` | Edit the shown note | Edit note |
| `d` | Open the delete confirmation | Delete note and linked audio |
| `f` | Refresh notes | Refresh notes |
| `a` | Add a date | Add a date |
| `m` | Load more | Load more |
| `?` | Open the Help Overlay | Shortcuts help |

Accept refinement and Confirm Source List deliberately have no letter, because
they write the real note and confirm the selection; they stay focus plus Enter
(the same reasoning as the Command Line, which cannot confirm a selection).
`d` only opens the existing Enter/Esc confirmation.

Gate, evaluated before any letter is interpreted: focus zone is Resources,
stage is Sources, no editor, Command Line, extra-date field, delete
confirmation, or Help Overlay is active, and the current IME indicator is `EN`.
Only a plain key event counts: an ASCII letter (or `?`) with at most Shift, and
no Ctrl, Alt, or Super modifier; composed or non-ASCII characters never match.
Otherwise the key is ignored in this zone (never inserted anywhere, and it does
not move focus or the list). Letters act on the same state and disabled
conditions as the menu item; a shortcut for a hidden item does nothing. The
gate is a pure function of (zone, stage, input modes, IME state, key) so it is
unit-tested without a terminal. Risk: the indicator can lag a just-made IME
switch; the destructive paths are guarded anyway (delete confirms, Accept and
Confirm have no letter).

## Hide (session only)

The Sources view model keeps a set of hidden source IDs. `h` adds the focused
note: if it was included it is excluded first through the normal toggle (so the
preparation and any confirmation are invalidated exactly as for `x`), then the
row is filtered out of the list, focus moves to the nearest visible row (the
shown text and any suggestion of the hidden note close; an unsaved source edit
blocks hiding), and the
header shows `N hidden (u: show)`. `u` clears the set. Hidden IDs are never part
of Confirm Source List. The set survives Refresh notes and Load more (IDs are
stable) and is cleared when the TUI restarts or the open workflow changes. It is
presentation state: nothing is written and no Ayumi note is touched.

## Help Overlay

`?` opens a modal overlay over the whole frame (cleared beneath, like sidebar
overlays), sized to the frame with a margin (up to 76 x 26 inside the 80x30
minimum, larger when the terminal allows), titled `Sources shortcuts` and
showing: the letter shortcuts from the table above grouped as List, Note, and
Assistant; the always-available keys (Tab/Shift-Tab, arrows, Enter, Ctrl-G,
Ctrl-P/Ctrl-N, Ctrl-S, Ctrl-C twice); the rule that letters need `IME: EN`
and the current IME state; and the actions that have no letter (Accept
refinement, Confirm Source List). Text is generated from the shortcut table so
it cannot drift. Content longer than the pane scrolls with Up/Down and
PageUp/PageDown; Esc, Enter, or `?` closes it. While open it is modal: no key
reaches any buffer, menu, or shortcut. `?` obeys the same gate as the letters;
the "Shortcuts help" menu item opens it under any IME.

## Automatic discovery

On entering Sources for a workflow, start the existing three-day discovery job
once when there is no current accepted discovery, the workflow state allows
discovery, and the client can mutate it (so a completed or read-only workflow
is not rediscovered). Re-entering with current results does nothing; a failed
discovery is reported and not retried automatically; Refresh notes starts it
unconditionally. A discovery already running is not duplicated.

## Approach and sequence

1. Re-verify the Confirm Source List finding above against the code at
   implementation time and tick the overview item. If the server turns out to
   require a change, stop for an owner decision.
2. Failing tests for the menu model (visible items per state, focus clamping),
   then the model.
3. Failing tests for Confirm Source List's sequencing and failure reasons
   against the fake transport, then the sequencer.
4. Wire the flat menu and automatic discovery into `main.rs`; delete the
   reviewed gate and the superseded controls and their tests.
5. Re-verify refine/accept/reject/edit/delete reachability and the stale-note
   failure.
6. Add the hidden set, the pure shortcut gate and mapping, and the Help Overlay
   text and widget with tests first (every key under
   every IME state and zone), then route it in `main.rs` ahead of the
   existing Char handling only when the gate allows it.
7. Update docs and statuses (including the Input constraint section of
   `ux-tui.md`, `user-guide-tui.md`, and the AGENTS.md index line), then run the
   gate.

## Risks

- The single-use, ordered-ids preparation handle can fail a confirmation that
  looks valid. Mitigation: one sorted id list, never reuse a handle, tests for
  expiry and binding mismatch.
- A hidden-item menu can strand focus. Mitigation: clamp by visible position
  and test appear/disappear transitions.
- Automatic discovery could race a user toggle. Mitigation: it only starts when
  no discovery is current or running, and toggles during a running job behave
  as today.
