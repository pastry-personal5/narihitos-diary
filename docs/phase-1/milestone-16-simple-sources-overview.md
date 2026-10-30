# P1-M16: Simple Sources stage overview

Status: Done

This milestone simplifies the TUI Sources stage to a short flat menu: include
or exclude notes, optionally refine a note with the assistant and accept or
reject the result, then press Confirm Source List. Reviewing each note's text
is no longer required, and note discovery starts on entering the stage.

## Goal

A person can get from entering Sources to a confirmed source list with only
the choices that matter: which notes to include, and whether to accept an
assistant refinement. No server, Control API, or decided-behavior change.

## Entry criteria

- P1-M7, P1-M8, and P1-M15 are `Done`.

## In scope

- One flat Sources menu with inapplicable items hidden (see the architecture
  plan): Include/exclude this note; View content; Hide note / Show hidden
  notes; Refine with assistant; Accept refinement and Reject refinement (only
  while a suggestion is shown); Edit note; Delete note and linked audio
  (Enter/Esc confirmation kept); Confirm Source List; Refresh notes; Add a
  date; Load more; Shortcuts help; Back to Start.
- Automatic discovery of the three-day window when the stage is entered for a
  workflow with no current discovery.
- **Confirm Source List** replaces "Confirm reviewed source set". It prepares
  the included notes (download cloud-only, read, snapshot) and then confirms,
  with progress in Work Status (the server performs the download and read), and
  stops with the reason if an included note is unreadable or still
  downloading. The single-use preparation handle is never reused. With nothing included the same control
  reads "Continue without notes" and keeps its explicit confirmation.
- Remove the "reviewed" gate and counter, and the separate "Prepare included
  sources" and "Inspect active prepared source" menu steps. Enter on a note
  still shows its text and still never changes inclusion.
- Source-list letter shortcuts, active only while the Resources Sidebar is
  focused in Sources and the IME indicator reads `IME: EN`: `i` include, `x`
  exclude, `v` view the note's content, `h` hide the note from the list, `u`
  show hidden notes, `r` refine, `n` reject refinement, `e` edit, `d` delete
  (opens the existing Enter/Esc confirmation), `f` refresh notes, `a` add a
  date, `m` load more; either letter case, plain keys only (no Ctrl or Alt).
  Accept refinement and Confirm Source List have no letter: they write the real
  note or confirm the selection, so they stay focus plus Enter. Every action also stays reachable by the flat menu and Enter.
- Hide: a session-only, client-side filter. A hidden note leaves the list and
  is forced excluded; `u` shows hidden notes again; the list header counts them.
  It resets when the TUI restarts or another workflow opens, writes nothing, and
  never touches the Ayumi note.
- Help Overlay: `?` (or a "Shortcuts help" menu item) opens a large modal
  overlay listing every Sources-stage shortcut and the non-letter keys, built
  from the same table that routes the keys; Esc, Enter, or `?` closes it.
- Work Status tip: the second Work Status row shows a one-line `Tip:` telling an
  end user what to do to reach the next stage, for every stage, generated from
  workflow state by a pure function.
- Canonical terms `Confirm Source List` (`confirm_source_list`) and `Help
  Overlay` (`help_overlay`) in [UX terms](../ux-terms.md); docs for the Sources
  stage updated.

## Out of scope

- Control API, contract, SQLite, or connector changes. The server does not
  require per-note previews or review before confirmation (architecture plan,
  Confirm Source List); if implementation finds otherwise, a server change
  stops the milestone for an owner decision.
- Changing what Accept, Edit, and Delete do to the real Ayumi note.
- Persisting hidden notes across restarts or days, and any server field for it.
- Changing discovery rules, date handling, or download behavior.
- Letter shortcuts anywhere other than the Sources-stage source list, and any
  letter that confirms a selection or writes a note.
- Other stages, the Utility Sidebar, and the Command Line vocabulary.

## Completion checklist

- [x] Step 1 finding re-verified against the code at implementation time
  (confirmation needs only a fresh, single-use handle bound to the ordered
  `source_ids`; no per-note preview or review).
- [x] Confirm Source List confirms a source list in which no note's text was
  opened (client/headless test and the existing server harness).
- [x] An included note that is unreadable, unavailable, or still downloading
  blocks confirmation with a visible reason; nothing is confirmed.
- [x] Cloud-only included notes are downloaded by Confirm Source List, and only
  included ones.
- [x] With nothing included the control reads "Continue without notes", keeps
  its explicit confirmation, and the existing explicit-empty rule is unchanged.
- [x] More than 64 included notes disables Confirm Source List with a visible
  reason; an expired or mismatched handle fails visibly and the next attempt
  prepares again; View and Confirm cannot run at the same time.
- [x] Changing the included set after confirmation still requires a new
  confirmation before any later output.
- [x] Entering Sources starts discovery once for a workflow that allows it;
  re-entering with current results, a read-only or completed workflow, or after
  a failed discovery does not rediscover automatically; Refresh notes forces it.
- [x] Refine, Accept, and Reject still work; Accept/Reject appear only while a
  suggestion is shown; a changed note still fails Accept safely.
- [x] Edit and Delete remain reachable in the flat menu; Delete keeps its
  Enter/Esc confirmation naming the linked-audio consequence.
- [x] The flat menu hides inapplicable items and keeps focus on a valid item
  after items appear or disappear; Refine, Accept, Reject, Edit, and Delete act
  only on the note shown for the focused row, and moving focus closes the shown
  text and suggestion.
- [x] Shortcut letters act only with the Resources Sidebar focused in Sources,
  no editor or Command Line open, and `IME: EN`; under `IME: KO`, `IME: ?`, or
  another zone or stage, or while the delete confirmation, extra-date field, or
  Help Overlay is open, or with a Ctrl/Alt modifier, they do nothing and are
  never typed into a buffer.
- [x] Each shortcut maps to the same action as its menu item (include, exclude,
  refine, reject, edit, delete-with-confirmation, refresh, add a date, load
  more), respects the same disabled conditions, and is case-insensitive.
- [x] Hide closes the hidden note's shown text and suggestion and is refused
  while that note has an unsaved edit.
- [x] Hide removes a note from the list and from the included set (so the
  preparation is invalidated if it was included), `u` restores it, the header
  counts hidden notes, hidden notes are never sent to Confirm Source List, and
  hiding survives Refresh notes but not a restart or a different workflow.
- [x] `?` opens the Help Overlay under the same gate; it is modal (other keys
  ignored, nothing typed through it), closes on Esc/Enter/`?`, and is reachable
  without a letter from the menu.
- [x] The Help Overlay text is generated from the shortcut table; a test fails
  if a shortcut is missing from it, and the overlay fits and is readable at
  80x30 and at larger sizes (scrolls if content exceeds the pane).
- [x] No letter exists for Accept refinement or Confirm Source List.
- [x] The second Work Status row shows a `Tip:` for the next step in every
  stage (Start through Publish, with and without a workflow), never displaces a
  prompt, confirmation, or error on the notice row, and names only controls that
  exist.
- [x] Docs updated (`docs/README.md`, `product-behavior.md`, `ux-terms.md`,
  `ux-tui.md` including the Input constraint section, `user-guide-tui.md`
  including its "no bare-letter hotkeys" line and the Sources stage steps,
  `AGENTS.md` index line, phase doc, changelog).
- [x] Gate passes: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`, plus `swift build` and `swift test` in
  `helpers/apple-bridge`.

## Acceptance evidence

- `sources_view` and `sources_menu` tests cover single-use ordered handles, the
  64-note limit, an unopened-note confirmation, hidden-note exclusion, and the
  visible disabled reason. The server preparation loop reads and requests a
  download only for the submitted included IDs and fails without a handle for
  an unreadable or still-downloading note. The connected TUI reports the job
  error code and does not send confirmation after a failed preparation.
- `source_shortcuts`, `terminal`, `help_overlay`, and `apps/nd` tests cover the
  IME and modifier gate, modal help content and scrolling, and automatic
  discovery conditions. The TUI blocks simultaneous View and Confirm jobs and
  refuses Hide while a source edit is active.
- Final gate: Rust format, Clippy, and all-feature tests passed; Swift bridge
  build and tests passed. The Rust fixture tests and SwiftPM ran outside the
  workspace sandbox because it denies local sockets and SwiftPM sandbox setup.
