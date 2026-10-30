# iCloud output connector Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Automatically and quietly mirror each published diary entry to a configured iCloud Drive folder as a byte-identical Markdown file.

**Architecture:** Add `DestinationKind::Mirror` (destination name `"icloud"`) beside `Local` and `Remote`. A second `LocalOutput` instance with an iCloud-validated root is the adapter. `publish_local_guarded` runs the mirror right after the local write is verified, inside the same job and under the same approval. A failed or blocked mirror leaves the day `Partial` and never touches local or Notion.

**Tech Stack:** Rust workspace (`diary-core`, `diary-connectors`, `diary-control`, `diary-control-contract`, `diary-tui`, `apps/nd`), SQLite, hand-rolled YAML config parser.

**Spec:** `docs/superpowers/specs/2026-10-30-icloud-output-design.md`

## Global Constraints

- Gate before any milestone is done: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features` (AGENTS.md).
- Mirror root must resolve under `~/Library/Mobile Documents/`, with no `..` components and no symlink escape.
- Mirror bytes equal the local file's bytes (`render_local_markdown`); layout `YYYY/MM/YYYY-MM-DD.md`.
- No new approval step; the existing plan digest covers the mirror target.
- Logs and diagnostics are content-free: reason tokens only, never paths or diary text.
- Config key: `paths.icloud_entries_root`, optional; absent means no mirror target in any plan.
- Do not change the Notion format, local layout, YAML keys, or the Swift helper.
- Ask before changing dependencies or build tools; this plan adds none.

## Review Focus

- Config set but iCloud folder missing or unreadable: mirror is `blocked`, local still publishes and the day is `Partial`.
- Target is an evicted `.<name>.icloud` placeholder: `blocked`, never treated as absent, never written over.
- Different content already at the mirror path: `Conflict`, never overwritten; local and Notion unaffected.
- Process dies between mirror write and read-back: startup reconciliation resolves it without a duplicate write.
- Config changed between preview and publish (mirror added or removed): plan digest mismatch forces a new preview.

---

### Task 1: `Mirror` kind and workflow semantics

**Files:**
- Modify: `crates/diary-core/src/types.rs` (`DestinationKind`, ~line 596)
- Modify: `crates/diary-core/src/workflow.rs` (add `finish_mirror_retry`; leave the `Local`/`Remote` filters alone)
- Test: unit tests in the same two files

**Interfaces:**
- Produces: `DestinationKind::Mirror` with `as_str() == "mirror"` and `parse("mirror")`; `Workflow::finish_mirror_retry(&mut self, expected_revision: u64, status: DestinationStatus, now: Timestamp) -> Result<(), WorkflowError>`. It is valid only in `Partial` with an approval, `"local"` progress `Verified`, and `"icloud"` progress not `Verified`. It sets only `"icloud"`, moves to `Completed` and sets `all_verified_at` when every destination is then `Verified`, and otherwise stays `Partial` (bump revision). It rejects other states with `WrongState` and a missing `"icloud"` entry with `UnknownDestination`.

- [ ] **Step 1: Write failing tests**
  - `destination_kind_mirror_round_trips`: `DestinationKind::parse(DestinationKind::Mirror.as_str()) == Ok(Mirror)`.
  - `mirror_blocked_does_not_block_notion_approval`: a plan with Local (verified), Mirror (blocked), and Remote (blocked) lets `approve_blocked_retry(..., "notion", ...)` succeed.
  - `mirror_conflict_leaves_local_verified_and_day_partial`: `finish_publication` with local `Verified` and icloud `Conflict` ends in `Partial`.
  - `finish_mirror_retry_completes_when_all_verified`, `finish_mirror_retry_rejects_non_partial`, `finish_mirror_retry_cannot_regress_verified`.
- [ ] **Step 2:** Run `cargo test -p diary-core mirror` → FAIL (variant and method missing).
- [ ] **Step 3:** Add the variant and string mapping. Implement `finish_mirror_retry`. Check `validate_progress` and `finish_publication` accept a `Mirror` target and mutate nothing for it.
- [ ] **Step 4:** Run `cargo test -p diary-core` → PASS.
- [ ] **Step 5:** `git commit -m "feat(core): Mirror destination kind and mirror retry transition"` (attribution trailer per the session reminder).

### Task 2: Config and iCloud-validated adapter

**Files:**
- Modify: `crates/diary-control/src/lifecycle.rs` (`parse_config` ~line 611, `LifecycleConfig`, `resolve_entries_root` neighbor)
- Modify: `crates/diary-connectors/src/local_output.rs` (add `LocalOutput::new_icloud`, `LocalOutput::inspect` placeholder rule)
- Modify: `config/global_config.example.yaml`
- Test: unit tests in `lifecycle.rs` and `local_output.rs`

**Interfaces:**
- Consumes: existing `LocalOutput::new(root) -> Result<Self, LocalError>`, `resolve_entries_root`.
- Produces: `LifecycleConfig.icloud_entries_root: Option<PathBuf>` (`~` expanded as `entries_root` is; duplicate key → `InvalidConfig`); `LocalOutput::new_icloud(root: PathBuf, home: &Path) -> Result<Self, LocalError>` returning `LocalError::UnsafePath` unless the canonicalized existing ancestor of `root` is under `home/Library/Mobile Documents`. `inspect` and `write` return `LocalError::Unavailable` when `<dir>/.<name>.icloud` exists for the target (a placeholder).

- [ ] **Step 1: Write failing tests**
  - `parses_optional_icloud_entries_root`, `absent_icloud_root_is_none`, `duplicate_icloud_root_is_invalid`.
  - `new_icloud_rejects_root_outside_mobile_documents`, `new_icloud_rejects_parent_dir_component`, `new_icloud_rejects_symlink_escaping_mobile_documents`.
  - `icloud_placeholder_makes_inspect_and_write_unavailable` (create `.2026-10-30.md.icloud`; assert the real file is never created).
  - Use a scratch `home` containing `Library/Mobile Documents/com~apple~CloudDocs/`.
- [ ] **Step 2:** Run `cargo test -p diary-control -p diary-connectors icloud` → FAIL.
- [ ] **Step 3:** Implement. Reuse the existing `UnsafePath` checks, then add the home-prefix check.
- [ ] **Step 4:** Run the tests → PASS. Add the commented `icloud_entries_root` example from the spec to `global_config.example.yaml`.
- [ ] **Step 5:** Commit `feat(connectors): iCloud-validated mirror output root and config`.

### Task 3: Plan, publish, and reconcile in the control layer

**Files:**
- Modify: `crates/diary-control/src/commands.rs` (`with_local_output` neighbor, plan construction ~line 940–1010, `publish_local_guarded` ~line 1092, `reconcile_local_intents` ~line 1522)
- Modify: `apps/nd/src/main.rs` (~line 276)
- Test: `apps/nd/tests/p1_m14_icloud.rs` (new, modeled on `p1_m9_local.rs`)

**Interfaces:**
- Consumes: Task 1 `finish_mirror_retry`, `DestinationKind::Mirror`; Task 2 `LocalOutput::new_icloud`, `LifecycleConfig.icloud_entries_root`.
- Produces: `CommandService::with_mirror_output(self, Arc<dyn LocalOutputPort>) -> Self`; `CommandService::publish_mirror_guarded(&self, day: &DiaryDay, expected_revision: u64, commit: impl FnOnce() -> bool) -> Result<Workflow, CommandError>`; `CommandService::reconcile_mirror_intents(&self) -> Result<(), CommandError>`. Mirror destination name is `"icloud"`.

- [ ] **Step 1: Write failing integration tests** in `p1_m14_icloud.rs` using synthetic scratch directories:
  - `mirror_is_byte_identical_and_day_completes_without_extra_approval`.
  - `no_mirror_configured_leaves_plan_and_behavior_unchanged`.
  - `missing_icloud_root_blocks_mirror_but_local_publishes_day_partial`.
  - `evicted_placeholder_blocks_mirror`.
  - `different_mirror_content_is_conflict_and_local_notion_unaffected`.
  - `mirror_failure_does_not_block_notion_preview_and_publish`.
  - `restart_after_ambiguous_mirror_write_reconciles_to_verified` (fake output returns `Absent` after write; reconcile must verify without a second write).
  - `mirror_retry_after_blocked_completes_day` (via `publish_mirror_guarded`).
  - `config_change_between_preview_and_publish_forces_new_preview`.
- [ ] **Step 2:** Run `cargo test -p nd --test p1_m14_icloud` → FAIL.
- [ ] **Step 3:** In plan construction, when a mirror output is set, add a `DestinationPlan::new(DestinationKind::Mirror, "icloud", mirror.target(day), digest, digest, observed)` (same payload digest as local; `Blocked` status plus content-free reason when the preflight fails). Generalize `LocalDestination` use so `publish_local_guarded` executes a second `PublicationCoordinator::execute` for the mirror intent after local is `Verified`, skipping it when its preview progress is `Blocked`/`Conflict`. Add its `DestinationProgress { destination: "icloud", .. }` to the `finish_publication` vector before `"notion"`. `publish_mirror_guarded` re-inspects, requires a verified local receipt (reuse `verify_local_receipt`), runs the coordinator, then calls `finish_mirror_retry`. `reconcile_mirror_intents` mirrors `reconcile_local_intents` filtered on `"icloud"`.
- [ ] **Step 4:** In `main.rs`, build the mirror with `LocalOutput::new_icloud(root, &home).map_err(|_| "icloud-root-invalid")?` when configured, attach it, and call `reconcile_mirror_intents` after the local reconcile.
- [ ] **Step 5:** Run the new tests plus `cargo test -p nd` and `-p diary-control` → PASS (existing P1-M9/M10 tests must stay green, which proves the unchanged behavior).
- [ ] **Step 6:** Commit `feat(control): automatic iCloud mirror publication and recovery`.

### Task 4: Persistence, contract, and quiet TUI

**Files:**
- Modify: `crates/diary-connectors/src/sqlite.rs` (round-trip test; add `MIGRATION_6` appended to `MIGRATIONS` only if a destination-kind column carries a constraint)
- Modify: `crates/diary-control/src/server.rs`, `crates/diary-control-contract/src/lib.rs` (additive `mirror` kind in destination DTOs and the Output preview)
- Modify: `crates/diary-tui` Output preview and Publication status views (`compose_view.rs`, `work_status.rs`; find the publication row rendering)
- Test: existing test modules in those files

**Interfaces:**
- Consumes: Task 3 destination `"icloud"` with kind `mirror`.

- [ ] **Step 1: Write failing tests**
  - sqlite: `mirror_destination_round_trips_through_store` (saving a plan with a Mirror target and reading it back). No schema change if this passes without a migration.
  - contract/server: `mirror_kind_serializes_as_mirror`, and an unknown-kind decode test showing clients treat it as non-blocking.
  - TUI: `verified_mirror_collapses_into_local_row_note`, `pending_blocked_or_conflict_mirror_shows_own_row`, `mirror_failure_raises_no_modal`.
- [ ] **Step 2:** Run the three suites → FAIL where behavior is missing.
- [ ] **Step 3:** Implement the DTO field and TUI rendering. The Output preview adds one line with the mirror path label (no raw filesystem path outside the preview, per the content-free log rule).
- [ ] **Step 4:** Run `cargo test --all-features` → PASS.
- [ ] **Step 5:** Commit `feat(tui): quiet iCloud mirror status and contract kind`.

### Task 5: Docs, milestone record, and gate

**Files:**
- Create: `docs/phase-1/milestone-14-icloud-output-overview.md` (title, `Status:` line, summary, goal, scope, and a verifiable checklist mapping to Tasks 1–4 tests)
- Modify: `docs/product-behavior.md`, `docs/architecture.md` ("Adding connectors", "Publication and recovery"), `docs/control-api-v1.md`, `docs/ux-tui.md`, `docs/user-guide-tui.md`, `README.md`, `docs/phase-1/changelog.md`, `docs/roadmap.md`, `docs/README.md`, `AGENTS.md` (one-line "Decided" entry)
- Modify: `docs/superpowers/specs/2026-10-30-icloud-output-design.md` (set `Status: Implemented`; note whether a migration was needed)

- [ ] **Step 1:** Write the milestone overview with the checklist before implementation starts (AGENTS.md requires it); check items only with test evidence.
- [ ] **Step 2:** Update each listed doc; link to the spec instead of copying facts.
- [ ] **Step 3:** Run `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features`; all must pass. Report any blocker unchecked.
- [ ] **Step 4:** Commit `docs: P1-M14 iCloud output`.

## Self-review notes

- Spec coverage: destination kind (T1), adapter and preflight (T2), quiet automatic approval/run, failure isolation, reconciliation (T3), config (T2), persistence and contract (T4), docs (T5), tests (all).
- Deviation from spec: the SQLite migration is conditional. No kind constraint was found on the destination column, so T4 expects a round-trip test to pass with no schema change.
- Milestone ordering: AGENTS.md says to add the checklist before implementation, so execute T5 Step 1 first, then T1–T4.
