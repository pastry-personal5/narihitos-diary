# Remote publication stages design

Status: Active (P1-M22).

Splits the Preview/Publish tail of the workflow into a local-only Preview, a Publish to Local stage, and a Publish to Remote stage that lists remotes (iCloud, Notion), shows what each already holds, and pushes on request. The iCloud copy is no longer an automatic mirror.

## Intent

- **Outcome:** a person finishes a day by previewing and saving the local file, then separately and deliberately pushes the same entry to each remote. Remotes are invisible until the Remote stage.
- **Owner decisions (2026-10-31):**
  - Stages are `Preview → Publish to Local → Publish to Remote`.
  - Preview shows only what is written to the local filesystem.
  - Remotes are listed in the Resources Sidebar (source-list pane): iCloud and Notion for now, extensible.
  - Selecting a remote opens a main-pane menu: see what will be pushed, check readiness, push.
  - Push is the approval; there is no separate approve step and remotes are not approved in Preview.
  - The iCloud copy is never made automatically.
  - A day is completed by the verified local write; remotes can be pushed at any time afterward, including in later sessions.
  - When a remote already holds an entry, the Main Pane shows it automatically. Notion offers **Append** and **Overwrite**; iCloud offers **Overwrite** only (owner clarification, 2026-10-31), preserving byte identity with the local file.
- **Success:** local publish never touches a remote; every remote action starts from the menu; an existing remote entry is visible without asking and resolvable in one choice; nothing reaches a remote without a push.

## Design

### Stages

Workflow sidebar: `Start, Sources, Compose, Review, Preview, Publish to Local, Publish to Remote`. `Stage::Publish` becomes `PublishLocal` and `PublishRemote`.

- **Preview** shows the local file path and exact bytes only. It never stops at a conflict (owner decision, 2026-10-31): when the local file already holds something else, the plan defaults to Append, which keeps what is there and adds the entry below it, and Overwrite stays an explicit, confirmed choice. No iCloud or Notion line, no mirror status. Confirming the local plan is the local approval. A file that changes after it was previewed or approved is previewed again, never stuck.
- **Publish to Local** writes and verifies the file and shows the receipt. A verified write completes the day and unlocks the Remote stage. Amend still starts here. Local conflict handling (differing file at approval time) re-previews with the existing file shown and the same Append/Overwrite choice rather than a dead end.
- **Publish to Remote** is reachable once the day is completed, in this or a later session. Before that it shows "Publish to Local first".

### Remote stage UX

The Resources Sidebar lists remotes, each with a status: `Not pushed`, `Exists` (the remote holds content that is not our receipt), `Pushed`, or `Blocked` (unconfigured or readiness failed, with a reason). Selecting a remote loads its menu in the Main Pane:

1. **What will be pushed.** The exact plan: target (iCloud path; Notion root/year/month path, `Position` line, projected blocks).
2. **Check readiness.** Manual, read-only (P1-M19 behavior, now per remote).
3. **Push to <remote>.**

On selection the TUI runs a preview automatically (read-only). If the remote already holds an entry for the day, the Main Pane shows the existing content above the planned result. Notion offers **Append** and **Overwrite** (P1-M21 rules); iCloud offers only **Overwrite**, because its copy must match the local file byte for byte. Choosing a mode re-previews the exact result; Enter pushes; Overwrite needs a second Enter and Esc cancels. When nothing exists, only Push is offered. Push is the approval of the shown plan.

"Conflict" no longer exists as a stalling state. If the remote changed between preview and push, the push does not fail: the TUI refreshes the preview, shows the new existing content, keeps the chosen mode if still valid, and asks again. Duplicate or malformed Notion sections (P1-M20) cannot be appended to or overwritten safely, so that remote shows `Blocked`. The Main Pane then shows a warning that names the problem (for example, two sections for the same day) and a URL to the Notion month page, so the person can fix it by hand in the Notion app or on Notion Web (owner decision, 2026-10-31); the menu offers only Check readiness, and a re-check after the fix unblocks it. The URL is derived from the page the connector already resolved. It appears only in this blocked state, as a single Notion page URL, and is never logged. This narrows the earlier rule that Notion previews never carry remote IDs, because a page URL contains the page ID. It is the only blocked-by-content case.

### Server and contract (v9)

- A `RemoteConnector` port (id, label, readiness, preview, push) is implemented by the iCloud mirror code and the Notion connector. A registry lists the configured remotes.
- Routes: `GET /v1/workflows/{id}/remotes` (id, label, status); `POST .../remotes/{name}/readiness`; `POST .../remotes/{name}/previews` (optional `existing_mode`); `POST .../remotes/{name}/pushes` (plan digest, mode; returns a job). A push approves and publishes in one job, bound to the preview's plan digest and the remote's observed-state digest.
- The local plan drops `icloud_target`/`icloud_status` and every mirror row. Local publication no longer reads `paths.icloud_entries_root`.
- Removed or folded in: Notion-only previews/approvals/publications, the combined P1-M19 readiness check, and `amendments/notion-retries`. An amendment is local-only: it previews the current file against its replacement (never a conflict; approval binds exactly the content shown), replaces the file, and leaves every remote alone. Pushing again then updates iCloud's copy the app saved earlier, and a Notion section the app wrote (its page, blocks and digest match its latest receipt) in place, with no prompt. Anything else already at a remote is `Exists`.
- Existing receipts and approvals map to `Pushed`; no data is deleted.
- Guarantees that stay: confirmed local entry is the source; a remote failure never affects the local file or the other remote; iCloud ends byte-identical to the local file with a backup of differing content; Notion keeps ascending order (P1-M20) and the P1-M21 insert, read-back, trash overwrite.

### Docs and decisions

Update `AGENTS.md` (iCloud mirror, Notion publication, Product behavior, Terminal UX index lines), `product-behavior`, `ux-tui`, `user-guide-tui`, `architecture`, `control-api-v1`, the roadmap, and the changelog. P1-M14 is superseded in part. This reverses the decided "iCloud mirror under the same approval" behavior on the owner's 2026-10-31 instruction.

### Testing

- Local publish never writes to iCloud or contacts Notion.
- Stage gating: Remote locked until the day is completed; reachable after reopening.
- Per-remote readiness, preview, and push, including an existing remote entry, Notion Append, Overwrite, and a remote changed between preview and push (refresh, no stall).
- TUI: sidebar remote list and statuses, automatic existing-content display, menu controls, Overwrite second Enter and Esc (closes P1-M21's open item).
- Gate: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features`, plus `swift build` and `swift test`.
