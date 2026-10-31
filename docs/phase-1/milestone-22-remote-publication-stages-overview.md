# P1-M22: Remote publication stages

Status: In progress

This milestone cleans up the end of the workflow: Preview becomes local-only, a
Publish to Local stage saves the entry, and a Publish to Remote stage lists the
remotes (iCloud, Notion) and pushes to each on request. The iCloud copy stops
being an automatic mirror. The owner requested the behavior on 2026-10-31; the
[design](../superpowers/specs/2026-10-31-remote-publication-stages-design.md)
holds the rules.

## Goal

A person previews and saves the local file, then deliberately pushes the same
entry to each remote from its own menu. An existing remote entry is shown
without asking and resolved with Notion Append or Overwrite, or iCloud
Overwrite; no one is stalled in
"Conflict".

## Scope

- Stages `Preview → Publish to Local → Publish to Remote`. Preview shows only the
  local file; it never mentions a remote.
- A verified local write completes the day. Remotes can be pushed at any later
  time, including after reopening the day.
- The Resources Sidebar lists remotes with a status (`Not pushed`, `Exists`,
  `Pushed`, `Blocked`). Selecting one opens a menu: what will be pushed, check
  readiness, push. An existing remote entry is previewed automatically and the
  Notion menu offers Append and Overwrite; iCloud offers only Overwrite, per the
  owner's 2026-10-31 byte-identity clarification.
- Push is the approval. Overwrite keeps its second Enter.
- No dead-end "conflict" anywhere a person works (owner request, 2026-10-31):
  Preview defaults to Append when the local file already holds something else,
  an amendment preview shows the current file and binds approval to it, a file
  that changed after approval is previewed again, and a refused approval
  re-previews automatically.
- Amendments are local-only. An amended day is pushed again from Publish
  Remote; a Notion section the app wrote is updated in place.
- A Notion month page with duplicate or malformed same-day sections blocks:
  the Main Pane shows a warning and a URL to that page, so the person can fix it
  by hand in the Notion app or on Notion Web. No Append or Overwrite is offered;
  a re-check after the fix unblocks it.
- Control API contract version 9: `GET /v1/workflows/{id}/remotes` and
  `POST .../remotes/{name}/readiness|previews|pushes`. The local plan loses all
  mirror fields; the Notion-only routes and `notion-retries` are removed.
- No dependency, connector library, assistant prompt, or provider change.

## Completion checklist

- [x] Existing and proposed diary-entry blocks in the Publish Local and Publish
  Remote Main Panes are indented consistently beneath their plain-text headings.
- [x] When Publish Local or Publish Remote finds an existing entry, the Main
  Pane shows that existing content in a foreground color distinct from both
  ordinary UI text and the diary entry to publish; headings preserve the
  distinction under `NO_COLOR`.
- [x] Publish Local and Publish Remote show the confirmed diary entry in the
  Main Pane, with the entry block rendered in a distinct foreground color and
  a plain-text heading that preserves meaning under `NO_COLOR`.
- [x] Review follow-up: no public workflow, readiness, diagnostic, or TUI status
  leaves a day in `conflict` or `stall`; a changed destination offers a fresh
  preview or an explicit repair path, including after restart.
- [x] Review follow-up: the Notion sidebar and readiness reflect the current
  local entry and remote content after an amendment, rather than an old receipt.
- [x] Review follow-up: add route-level evidence for a malformed Notion page's
  warning URL and for refreshing after a changed remote.
- [x] Local publication never writes iCloud or contacts Notion, with or without
  `paths.icloud_entries_root` configured.
- [x] Preview shows only the local file and the P1-M21 local Append/Overwrite
  choice; the local preview carries no iCloud or Notion field.
- [x] The Workflow Sidebar shows `Preview`, `Publish Local`, and `Publish
  Remote`; the Remote stage is locked until the day is completed and reachable
  after reopening it.
- [ ] The Resources Sidebar lists the configured remotes with their statuses.
  Implemented in the session loop; no automated test drives that loop, so verify
  in a real terminal.
- [ ] Selecting a remote previews it automatically; an existing remote entry is
  shown in the Main Pane with Notion Append/Overwrite or iCloud Overwrite; Overwrite needs
  a second Enter and Esc cancels. The menu and body are tested
  (`remote_view`) and the server side is tested; the automatic preview and the
  second Enter live in the session loop and need a real-terminal check (this
  also closes P1-M21's open Overwrite item).
- [x] Readiness is available per remote, manual and read-only.
- [ ] Push approves and publishes in one job bound to the preview's plan digest;
  a remote that changed since the preview refreshes the preview instead of
  stalling. The job and the stale-digest refusal are tested; the TUI's refresh
  after a refused push is untested.
- [x] iCloud push ends byte-identical to the local file and backs up differing
  content; Notion push keeps ascending order and the P1-M21 overwrite sequence
  (the unchanged connector tests plus a push through the worker).
- [x] A blocked Notion month page shows a warning and a working page URL (shown
  only in that state, never logged); a re-check after a manual fix unblocks it.
  The rendering, connector `month_page_url`, blocked route, and repaired
  readiness/preview path are covered by automated tests.
- [x] A failure at one remote never affects the local file or the other remote.
- [x] Preview and the amendment preview never stop at a conflict: an existing
  file defaults to an Append plan that can be approved as it stands, a changed
  file is previewed again, and the TUI re-previews when an approval is refused.
  (Server tests in `local_publication.rs`, `existing_entry.rs`, and
  `amendments.rs`; the automatic re-preview lives in the session loop and needs
  the real-terminal check.)
- [x] Amendments are local-only and a later push updates iCloud's earlier copy
  (`amendments.rs`) and recognises a Notion section the app wrote
  (`existing_notion.rs`). The in-place Notion update itself reuses the P1-M21
  Overwrite machinery; the route-level automatic choice has no end-to-end test.
- [x] `AGENTS.md`, architecture, product behavior, Terminal UX, user guide,
  Control API doc, roadmap, phase plan, and changelog describe the behavior;
  P1-M14 is marked superseded in part.
- [ ] Gate passes: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`, plus `swift build` and `swift test`. The Rust
  gate is clean after this review; the current environment blocks SwiftPM's
  manifest sandbox before compilation (`sandbox_apply: Operation not permitted`).

## Acceptance evidence

- Diary-entry indentation (2026-10-31): the shared Main Pane helper indents
  every prose line by two spaces, including blank lines; Publish Local and
  Publish Remote body tests cover the inset without changing stored text.
- Existing-entry visibility (2026-10-31): Publish Local renders the local
  preview's `existing_text`; Publish Remote renders either iCloud's existing
  file text or Notion's current blocks. Main Pane buffer tests prove existing
  content uses cyan while the proposed diary entry remains magenta, and theme
  tests prove both collapse to default under `NO_COLOR`.
- Publish-stage entry visibility (2026-10-31): `screen_body` and `remote_view`
  tests show the confirmed entry under `Diary entry to publish` in both stages;
  the `main_pane` buffer test proves only the entry uses the palette's magenta
  foreground, and the palette test proves `NO_COLOR` restores the default.
- Review follow-up (2026-10-31): old `conflict` destination rows parse as
  `changed`; status vocabularies and the TUI no longer present `CONFLICT`.
  `notion_publication.rs` proves a changed remote can be previewed again;
  `existing_notion.rs` proves a foreign section is ready for a choice and an
  amended local entry makes the old Notion copy `Not pushed`. `stage.rs` keeps
  Publish Remote open while a Notion plan is stored. `remote_view.rs` and
  `icloud_publication.rs` cover iCloud Overwrite without Append; the Notion
  connector test resolves a duplicate same-day page to its month URL.
- Gate (2026-10-31): `cargo fmt --all -- --check` and `cargo clippy
  --all-targets --all-features -- -D warnings` clean; `cargo test --workspace
  --all-features` 576 passed, 0 failed; `swift build` complete and `swift test`
  13 passed, 0 failed.
- Review gate (2026-10-31): formatting, Clippy with warnings denied, the focused
  M22 integration suites, and `cargo test --all-features` pass. Retrying the
  Swift helper with isolated writable Clang and SwiftPM module caches still
  stops before package compilation because this environment denies SwiftPM's
  `sandbox-exec` manifest sandbox.
- `apps/nd/tests/icloud_publication.rs`: local publication completes the day and
  never touches iCloud; push writes the local bytes; missing folder and evicted
  placeholder block only the remote; existing content is `Exists` and resolved
  with Overwrite (backup kept); a folder changed since the preview
  refuses the push; iCloud and Notion are pushed independently; an ambiguous
  write is reconciled after restart without writing again.
- `apps/nd/tests/publish_readiness.rs` and `notion_publication.rs`: per-remote
  readiness and a Notion preview and push through the TUI worker and server;
  the route-level flow carries a malformed page's repair URL, rechecks after
  repair, rejects a stale push with `precondition-failed`, and accepts a fresh
  preview instead of leaving a stalling state.
- `apps/nd/tests/amendments.rs`: an amendment leaves iCloud alone and a later
  push updates the copy the app wrote.
- `crates/diary-core` workflow tests: a verified local write completes the day.
