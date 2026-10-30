# P1-M17: Assistant settings continuity

Status: Done

This milestone keeps writing settings usable across workflows opened in one TUI
session and restores the last assistant choice after a TUI restart. It fixes the
writing-settings path clearing an already selected assistant.

## Goal

Changing the diary day or returning to Start does not unexpectedly reset the
assistant choice, writing language, or style. A newly created workflow uses the
saved assistant choice; an existing workflow retains its own stored settings.

## Scope

- Keep the selected assistant, language, and style for new workflows in the
  running TUI. Persist the assistant choice through the existing preferences
  store and restore it on the next launch.
- Treat "None (manual only)" as a real choice. A missing or unavailable saved
  backend leaves a new workflow manual and never silently sends content. The
  existing SQLite preference stores the last selected backend ID; `null`
  represents manual mode.
- The TUI sends optional session language and style on start. The server
  applies them only when creating a workflow. The current backend ID is sent
  with later writing-settings changes so those changes keep the selection.
- Keep existing workflows' stored settings when they are reopened. Preserve a
  selected assistant when only language or style changes.
- No dependency, connector, assistant prompt, or provider change.

## Completion checklist

- [x] Changing language or style keeps the selected assistant, and the saved
  preference matches the workflow after either action.
- [x] Selecting a backend or manual mode persists the choice; a new TUI session
  restores that choice for a newly created workflow.
- [x] A new workflow in the same TUI session inherits the current language and
  style; opening an existing workflow does not overwrite its stored settings.
- [x] Unavailable or missing saved backends fail closed to manual mode, and
  assistant actions still require an explicit user action.
- [x] User guide and affected design docs describe the behavior.
- [x] Gate passes: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`, `swift build`, and `swift test` in
  `helpers/apple-bridge`.

## Acceptance evidence

- `assistant_choice_survives_restart_and_new_workflows_inherit_session_settings`
  exercises the real loopback server, SQLite reopen, backend selection,
  writing-settings preservation, session settings on a new workflow, manual
  mode, and existing-workflow retention.
- `real_loopback_enforces_headers_authorization_and_conditional_mutations`
  also checks that a saved backend missing from the registry leaves a new
  workflow manual. The registry availability filter applies the same behavior
  to an unavailable backend.
- `start_and_writing_settings_carry_session_choices` checks the TUI client's
  request fields and conditional revision header. The full Rust gate and Swift
  helper build/test gate passed on 2026-10-30.
