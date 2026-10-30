# P1-M19: Publish readiness

Status: Done

This milestone lets a person ask, from the Publish stage, whether Notion and
the iCloud mirror are ready and why not. The check is read-only, runs only on
request, and explains each destination in plain language.

## Goal

Publish shows a per-destination readiness (`ready`, `not ready`, `conflict`,
or `published`) with a reason and a next step, so a blocked destination is
never an unexplained label. A ready Notion never reads as permission to
publish: it points to Preview and approval.

## Scope

- A read-only Control API command, `POST /v1/workflows/{id}/readiness`, with
  the usual workflow revision precondition. It writes nothing, records no
  intents, and leaves the workflow revision unchanged.
- Notion readiness reuses the connector preflight directly (never the preview
  that rewrites the plan). iCloud readiness inspects the configured mirror
  target. Both report `local-file-not-verified` until the local file is
  verified.
- The response carries closed, content-free reason and next-step tokens; the
  TUI owns the wording. An unknown token shows a generic message and the raw
  token.
- The TUI adds a `Check Notion and iCloud readiness` control at the end of the
  Publish controls. It is manual only, shows `Not checked` before a check and
  `Checking...` while one runs, and clears when the workflow revision changes.
- A content-free `ndlog-v1` record per destination (readiness and reason).
- No dependency, connector, assistant prompt, or provider change.

## Completion checklist

- [x] The readiness command reports each case: Notion ready, blocked (each
  closed reason), conflict, not configured; iCloud ready, folder unavailable,
  different file present, not configured; and local not verified.
- [x] The command is read-only: workflow revision and intents are unchanged,
  and a stale revision is refused with a precondition failure.
- [x] The Publish stage renders `Not checked`, `Checking...`, and each result;
  results clear on a revision change; existing controls keep their behavior.
- [x] Readiness records use only closed tokens; free text is rejected.
- [x] The Publish stage no longer sends a Notion publish for a partial
  workflow whose Notion plan is not approved.
- [x] User guide, Terminal UX, architecture, and README index describe the
  behavior.
- [x] Gate passes: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`.

## Acceptance evidence

- `apps/nd/tests/publish_readiness.rs` covers every readiness case against the
  command service: local not verified, Notion ready, each blocked reason and
  next step, conflict, unlisted connector text collapsing to
  `notion-unavailable`, unconfigured Notion and iCloud, and iCloud ready,
  different file, and unavailable folder. It also asserts the check is
  read-only (revision, intents, and receipts unchanged) and refuses a stale
  revision.
- `readiness_round_trips_through_the_real_server_without_changing_the_workflow`
  drives the real loopback server through the TUI worker, confirms the `read`
  capability accepts the POST, and reads the log file back to prove the
  `readiness`, `readiness-check`, and `readiness-refused` records were written
  with closed tokens and no diary day or text.
- `readiness_view` and the Publish screen tests cover `Not checked`,
  `Checking...`, each result, clearing on a revision or day change, and the
  new control at index 2 with the existing controls unchanged.
- `publish_waits_for_a_notion_plan_that_is_previewed_and_approved` covers the
  extracted `needs_notion_approval` guard that stops Publish from sending an
  unapproved Notion plan.
- The contract version is 6; the route inventory test lists the new route.
- Related fix found while diagnosing: Notion receipt IDs collided across days
  of one month and left the second day reconciling. Covered by
  `two_days_in_one_month_each_save_their_own_notion_receipt` and
  `restart_adopts_a_landed_write_when_another_day_of_the_month_is_published`;
  recorded in the [changelog](changelog.md).
- `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features
  -- -D warnings`, and `cargo test --all-features` (547 tests) passed on
  2026-10-30. The Swift helper is untouched, so its gate does not apply.
