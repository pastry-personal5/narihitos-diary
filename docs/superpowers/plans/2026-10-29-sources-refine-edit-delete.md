# Source refine, edit, and delete Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a person, from the Sources stage, ask an assistant backend to refine an Ayumi source note into a cleaner snippet, approve/reject that suggestion, edit a source directly, or hard-delete a source (cascading to its linked audio) — with every write landing on the real Ayumi file.

**Architecture:** A fourth assistant action (`RefineSource`) reuses the existing `DraftAssistant`/`AssistantBackendDescriptor` core abstraction and the existing job/SSE machinery in `diary-control`'s Axum server (the same pattern as the `discovery` and `source-preparation` jobs). Direct edit and delete are synchronous, binding-checked writes through a new write/delete capability on the Ayumi connector, resolved through its existing safe-path `resolve`/`base` logic. This plan is the first concrete M8 vertical slice: it introduces the minimal `AssistantRegistry` and per-workflow job-exclusivity wiring that the still-unimplemented extraction/draft/paragraph-review actions will later reuse, but it implements only the source-refinement slice end-to-end — it does not implement extraction, drafting, or paragraph review, and it does not register a real Codex or Apple backend (those remain gated by their own separate M8 packets).

**Tech Stack:** Rust workspace (`diary-core`, `diary-connectors`, `diary-control`, `diary-control-contract`, `diary-tui`), Axum/Tokio server, `rusqlite`-based SQLite store, no new external dependency.

**Spec:** [docs/superpowers/specs/2026-10-29-sources-refine-edit-delete-design.md](../specs/2026-10-29-sources-refine-edit-delete-design.md)

## Global Constraints

- Hard delete is approved: delete removes the real Ayumi `.md` file and its linked audio from the mounted iCloud directory (spec "Owner decisions").
- Approving a refinement or saving a direct edit writes back to the real Ayumi file; `nd` never keeps a silently-diverging app-side copy.
- Refine goal: only correct clear transcription/grammar artifacts (nonsensical words/phrases, homophone slips, broken sentence structure), inferring the most plausible intended meaning from context without adding new facts; leave coherent, meaningful prose — including true-but-unusual statements — untouched.
- No audio bytes are ever read, decoded, or sent anywhere, in this feature or any other; delete acts on a file path only.
- Refine/approve/reject/edit/delete are all blocked while any assistant job is active for the workflow (reuses the one-active-job-per-workflow rule named in the M8 overview, implemented fresh in this plan since no prior action uses it yet).
- All new SQLite schema changes land in a new `MIGRATION_5` and bump `CURRENT_SCHEMA` from `4` to `5` (`crates/diary-connectors/src/sqlite.rs`).
- No automated test deletes or rewrites a real Ayumi/iCloud file outside a disposable fixture directory created by the test itself.
- Follow existing code style: `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features -- -D warnings` must stay clean; run `cargo test --all-features` for affected crates per task.

## Review Focus

- **A refine job completes after the source's content changed underneath it** (e.g. the person edited the source directly while the job was running) — the suggestion must bind to the content-digest it was generated from and surface as stale on resolution, not silently apply to new text. Covered in Task 6 (`resolve_refine_suggestion` stale-binding test) and Task 10 (control-layer binding test).
- **A note with an ambiguous attachment line is targeted for delete** — the existing Ayumi parser already withholds ambiguous lines as unreadable; delete must not guess at an ambiguous destination's audio-ness. Covered in Task 2 (`audio_links` only ever contains lines the parser already classified `StandaloneLocalAudio`) and Task 3 (`delete_source_with_ambiguous_attachment_line_deletes_note_but_no_audio`, asserting only the safe link's audio file is removed and anything behind an ambiguous line is left alone).
- **Deleting a source that is already part of the confirmed selection or has dependent memories/draft/suggestions** — delete must remove it from both `candidates()` and the confirmed selection and invalidate dependents, not leave a dangling reference. Covered in Task 5 (`Workflow::delete_source` test with a confirmed selection containing the deleted source).
- **Two assistant actions racing on the same workflow** (e.g. a refine job already running, then a second refine request or an edit/delete arrives) — must fail closed with a typed conflict, not queue silently or corrupt state. Covered in Task 11 (`one active job per workflow` test with two concurrent requests).
- **A `confirm: true` is missing or false on delete** — the route must reject the call outright rather than treating an absent/false flag as a safe no-op that still deletes. Covered in Task 10 (`delete_source` route test asserting `confirm: false` and missing `confirm` both return `source_delete_not_confirmed` with no filesystem change).

---

## Task 1: Core — refine action, request/response, and error shapes

**Files:**
- Modify: `crates/diary-core/src/types.rs` (near `AssistantAction`, `DraftAssistantRequest`, `DraftAssistantResponse`, `WorkflowError`-adjacent error enums)
- Test: `crates/diary-core/src/types.rs` (inline `#[cfg(test)] mod tests`, following the file's existing convention)

**Interfaces:**
- Produces: `AssistantAction::RefineSource` variant (`as_str() -> "refine-source"`); `DraftAssistantRequest.source_body: Option<String>` new field (populated only for `RefineSource`, `None` for the other three actions); `DraftAssistantResponse::SourceRefinement(String)` new variant holding the suggested body text.

- [ ] **Step 1: Write failing tests** for: `AssistantAction::RefineSource.as_str() == "refine-source"`; a `DraftAssistantRequest` literal with `source_body: Some(..)` and the other three action-specific fields left at their defaults still constructs (no validation ties `source_body` to a specific action — that binding lives in the caller, not the type); `DraftAssistantResponse::SourceRefinement("body".to_owned())` round-trips through a `match`.
- [ ] **Step 2: Run tests, confirm they fail** to compile (`cargo test -p diary-core --lib types:: 2>&1 | tail -20`) with "no variant `RefineSource`" / "no field `source_body`".
- [ ] **Step 3: Add the variant, field, and enum arm** exactly as named above. `source_body` goes after `focused_paragraph` on `DraftAssistantRequest`; `SourceRefinement(String)` goes after `ParagraphReview { .. }` on `DraftAssistantResponse`.
- [ ] **Step 4: Run tests, confirm pass** (`cargo test -p diary-core --lib types:: `).
- [ ] **Step 5: Commit** (`git add crates/diary-core/src/types.rs && git commit -m "feat(core): add RefineSource assistant action"`).

## Task 2: Connectors — recognize and carry audio-link destinations

**Files:**
- Modify: `crates/diary-connectors/src/ayumi.rs` (the `Parsed` struct returned by the module's Markdown parser, and `classify_attachment_line`'s `StandaloneLocalAudio` arm, around lines 580–650 and 680–730)
- Test: `crates/diary-connectors/src/ayumi.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing new; this task only widens data already computed during parsing.
- Produces: `Parsed.audio_links: Vec<String>` — the exact `destination` string (as `link_destination` already computes it) for every line the parser classified `StandaloneLocalAudio` and stripped from the body, in source order. Used by Task 8's delete path.

- [ ] **Step 1: Write a failing test** `parser_collects_standalone_local_audio_links_removed_from_body` asserting that parsing `b"prose\n[audio](recording.m4a)\nmore prose"` returns `Parsed { audio_links: vec!["recording.m4a".to_owned()], .. }` with `body == "prose\nmore prose"`, and that parsing a body with an ambiguous attachment line (reuse the existing `private/recording.mp3`-style ambiguous fixture from `parser_removes_only_standalone_local_audio_and_withholds_ambiguous_paths`) leaves `audio_links` empty.
- [ ] **Step 2: Run test, confirm it fails** — `Parsed` has no `audio_links` field yet.
- [ ] **Step 3: Add `audio_links: Vec<String>` to `Parsed`**, and in the parser's line loop, when `classify_attachment_line` returns `AttachmentLine::StandaloneLocalAudio`, capture the same destination string the classifier already parsed (the classifier currently discards it after returning the variant — change its return type to carry the destination out, e.g. `AttachmentLine::StandaloneLocalAudio(String)`, and push it into `audio_links` before `continue`).
- [ ] **Step 4: Run test, confirm pass.**
- [ ] **Step 5: Commit.**

## Task 3: Connectors — safe write-back and delete on the Ayumi adapter

**Files:**
- Modify: `crates/diary-connectors/src/ayumi.rs` (`AyumiCatalog` — add alongside `resolve`/`base` around line 314–348)
- Test: `crates/diary-connectors/src/ayumi.rs` real-filesystem tests, following the existing fixture-directory pattern used by `bounded_discovery_parses_dates_and_filters_audio` and similar

**Interfaces:**
- Consumes: `AyumiCatalog::resolve(&self, source: &SourceRef) -> Result<PathBuf, SourceReadError>` (existing, unchanged).
- Produces:
  - `AyumiCatalog::overwrite_body(&self, source: &SourceRef, new_body: &str, expected_content_digest: &Digest) -> Result<FreshnessStamp, SourceWriteError>` — re-reads and re-parses the current file first; if its current `Digest::of_bytes` of the readable body does not equal `expected_content_digest`, returns `SourceWriteError::Stale` without writing; otherwise replaces the body portion of the file (front matter preserved verbatim, including any `created_at`) and returns the new `FreshnessStamp`.
  - `AyumiCatalog::delete_source(&self, source: &SourceRef) -> Result<Vec<PathBuf>, SourceWriteError>` — re-reads and re-parses the current file itself (reusing Task 2's `Parsed.audio_links`, so the caller never needs to supply them — the stored `SourceSnapshot.body` is already audio-stripped and cannot be used for this), resolves each recognized link relative to the note's own parent directory using the same symlink/traversal/`starts_with(&base)` checks as `resolve`, rejecting (without deleting anything) if any resolved audio path escapes that directory; deletes each resolved audio file, then the note file itself; returns the list of paths actually deleted (audio first, note last) for the caller to log safely (paths only, never content).
  - `SourceWriteError` enum: `Stale`, `Io`, `UnsafePath`, matching the existing `SourceReadError` naming style.

- [ ] **Step 1: Write failing tests**: (a) `overwrite_body_replaces_text_and_returns_new_stamp` — write a fixture note, call `overwrite_body` with the current digest, assert the file on disk now contains the new body under the preserved front matter and the returned stamp's `content_digest` matches a fresh read; (b) `overwrite_body_rejects_stale_digest` — call with a wrong digest, assert `Err(SourceWriteError::Stale)` and the file is unchanged; (c) `delete_source_removes_note_and_its_audio` — fixture note whose body links one local audio file, call `delete_source`, assert both files are gone and the returned path list names both; (d) `delete_source_rejects_traversal_audio_link` — fixture note whose body links an audio destination like `"../../etc/passwd.m4a"` (this is already withheld as `AttachmentLine::Ambiguous`/not `StandaloneLocalAudio` by the existing parser, since a leading `..` component fails `valid_source_path`-style checks — assert the same safety holds end-to-end here: `Err(SourceWriteError::UnsafePath)` or the link is simply not in `audio_links` at all, and neither the note nor anything outside the fixture directory is touched either way); (e) `delete_source_with_ambiguous_attachment_line_deletes_note_but_no_audio` — fixture note with one `StandaloneLocalAudio` link and one separately `Ambiguous` attachment line, assert only the safe link's audio file is deleted and the ambiguous line's target file (if any) is left untouched.
- [ ] **Step 2: Run tests, confirm they fail** to compile (no such methods yet).
- [ ] **Step 3: Implement `overwrite_body`, `delete_source`, and `SourceWriteError`** on `AyumiCatalog`, reusing `base()`/`resolve()` for path safety and the existing front-matter split logic (`split_front_matter`) to preserve the header when writing.
- [ ] **Step 4: Run tests, confirm pass** (`cargo test -p diary-connectors --lib ayumi::`).
- [ ] **Step 5: Commit.**

## Task 4: Core — per-source refine suggestion and body-edit state on `Workflow`

**Files:**
- Modify: `crates/diary-core/src/types.rs` (new `RefineSuggestion` struct, `WorkflowError` variants)
- Modify: `crates/diary-core/src/workflow.rs` (new `Workflow` field and methods, near `confirm_selected_sources`/`save_entry`, lines ~370–625)
- Test: `crates/diary-core/src/workflow.rs` (existing `#[cfg(test)] mod tests`, using the file's existing `FakeSourceReader`-style fixtures)

**Interfaces:**
- Consumes: `Workflow::expect_revision`, `Workflow::bump` (existing private helpers, called from the new methods), `SourceRef`, `FreshnessStamp`, `Digest` (existing).
- Produces, all on `impl Workflow`:
  - `pub fn record_refine_suggestion(&mut self, expected_revision: u64, source: &SourceRef, suggested_body: String, binding_digest: Digest, now: Timestamp) -> Result<(), WorkflowError>` — errors `UnknownSource` if `source` is not in `self.snapshots()`; otherwise stores/replaces the pending suggestion for that source and bumps revision.
  - `pub fn refine_suggestion(&self, source: &SourceRef) -> Option<&RefineSuggestion>`.
  - `pub fn resolve_refine_suggestion(&mut self, expected_revision: u64, source: &SourceRef, accept: bool, now: Timestamp) -> Result<Option<String>, WorkflowError>` — errors `RefineSuggestionNotFound` if none pending; on `accept = true`, errors `RefineBindingStale` if the suggestion's `binding_digest` no longer matches the source's current snapshot `content_digest`, else clears the suggestion and returns `Ok(Some(suggested_body))` (the caller writes it back and then calls `update_source_body`); on `accept = false`, clears the suggestion and returns `Ok(None)`.
  - `pub fn update_source_body(&mut self, expected_revision: u64, source: &SourceRef, new_body: String, new_stamp: FreshnessStamp, now: Timestamp) -> Result<(), WorkflowError>` — errors `UnknownSource` if not present; replaces the matching entry in `self.snapshots` with the new body/stamp, clears any pending refine suggestion for that source, calls the existing `self.invalidate_sources(InvalidationReason::SourceChanged(source.clone()), now)`, and bumps revision.
  - `pub fn delete_source(&mut self, expected_revision: u64, source: &SourceRef, now: Timestamp) -> Result<(), WorkflowError>` — errors `UnknownSource` if not present in `candidates()`; removes the entry from `candidates`, from `snapshots` if present, from the confirmed selection if present, clears any pending refine suggestion for that source, calls `self.invalidate_sources(InvalidationReason::SourceChanged(source.clone()), now)`, and bumps revision.
  - New `RefineSuggestion { suggested_body: String, binding_digest: Digest }` (in `types.rs`, `#[derive(Clone, Debug, Eq, PartialEq)]`).
  - New `WorkflowError` variants: `RefineSuggestionNotFound`, `RefineBindingStale`, `SourceBodyBindingStale` (for a future direct-edit precondition check reused in Task 5), each with a `Display` arm following the existing style.

- [ ] **Step 1: Write failing tests**, each starting from a `Workflow` with one confirmed, selected source (reuse the file's existing `workflow_with_one_selected_source`-style helper or the nearest existing fixture builder): `record_then_fetch_refine_suggestion`; `resolve_refine_suggestion_accept_returns_body_and_clears_it`; `resolve_refine_suggestion_rejects_stale_binding`; `resolve_refine_suggestion_missing_returns_not_found`; `update_source_body_replaces_snapshot_and_invalidates`; `delete_source_removes_from_candidates_snapshots_and_selection`; `delete_source_unknown_returns_error`.
- [ ] **Step 2: Run tests, confirm they fail** to compile.
- [ ] **Step 3: Add the `RefineSuggestion` type, `WorkflowError` variants, the new `Workflow` field (e.g. `refine_suggestions: std::collections::BTreeMap<SourceRef, RefineSuggestion>`), and the four methods** above, following `save_entry`'s revision/`bump` pattern.
- [ ] **Step 4: Run tests, confirm pass** (`cargo test -p diary-core --lib workflow::`).
- [ ] **Step 5: Commit.**

## Task 5: SQLite — schema v5 for refine suggestions and source-body edits

**Files:**
- Modify: `crates/diary-connectors/src/sqlite.rs` (`CURRENT_SCHEMA`, add `MIGRATION_5`, the `WorkflowStore` impl's load/save of `source_snapshots` and the new suggestion table)
- Test: `crates/diary-connectors/src/sqlite.rs` (existing migration tests, following the `schema_version(...)` assertion pattern at lines ~1710–1950)

**Interfaces:**
- Consumes: `Workflow`'s new `refine_suggestions` field and the existing `snapshots()` persistence path (whatever private helper currently round-trips `source_snapshots` rows — read it before writing this task's steps in detail, since its exact name isn't pinned here).
- Produces: `CURRENT_SCHEMA = 5`; a new `source_refine_suggestions` table (`day, connector, source_id, suggested_body, binding_digest, PRIMARY KEY(day, connector, source_id), FOREIGN KEY(day) REFERENCES workflows(day) ON DELETE CASCADE`); `source_snapshots` round-trips unchanged in shape (body/digest already covers the edited-body case since `update_source_body` replaces the row's existing `readable_body`/`content_digest` columns in place — no new column needed).

- [ ] **Step 1: Write a failing test** `migrates_to_schema_5_and_round_trips_a_refine_suggestion` — open a schema-4 database (reuse the existing "open a prior-version fixture" helper), load/save a `Workflow` carrying one `RefineSuggestion`, reopen, and assert both `schema_version(..) == 5` and the suggestion reads back equal.
- [ ] **Step 2: Run test, confirm it fails** (`CURRENT_SCHEMA` is still 4; no such table).
- [ ] **Step 3: Add `MIGRATION_5`** (the `CREATE TABLE source_refine_suggestions` above) to the migration list, bump `CURRENT_SCHEMA` to `5`, and extend the existing snapshot/workflow load+save functions to also read/write `refine_suggestions`.
- [ ] **Step 4: Run test, confirm pass**, then run the full existing migration suite to confirm no regression (`cargo test -p diary-connectors --lib sqlite::`).
- [ ] **Step 5: Commit.**

## Task 6: Control contract — DTOs, bounds, and problem codes

**Files:**
- Modify: `crates/diary-control-contract/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub const MAX_SOURCE_REFINE_BYTES: usize = MAX_SOURCE_BYTES;` (reuse the existing per-note cap; refine's input and output are each one source body).
  - `dto! { pub struct RefineRequestDto { pub content_digest: String } }` (POST body for starting a refine job; binds the job to the source's current digest).
  - `dto! { pub struct RefineSuggestionDto { pub source_id: String, pub original_body: String, pub suggested_body: String, pub binding_digest: String } }` (GET response).
  - `dto! { pub struct RefineResolutionRequest { pub accept: bool, pub binding_digest: String } }` (PUT body for accept/reject).
  - `dto! { pub struct SourceBodyEditRequest { pub text: String, pub expected_content_digest: String } }` (PUT body for direct edit).
  - `dto! { pub struct SourceDeleteRequest { pub confirm: bool } }` (DELETE body).
  - No new problem-code type exists yet in this crate (problem codes today are plain `&'static str` passed to `ApiError::new`, per `crates/diary-control/src/server.rs`'s `ApiError`); this task adds no enum, only documents the five new string codes as a doc comment next to the new DTOs: `source_refine_already_active`, `source_binding_stale`, `source_suggestion_already_resolved`, `source_delete_not_confirmed`, `source_delete_partial_failure`.

- [ ] **Step 1: Write failing tests** (following this file's existing DTO test style, e.g. `assistant_backend_descriptor_has_collection_shaped_action_contract`): one serialization round-trip test per new DTO, plus one test that `SourceDeleteRequest` with `deny_unknown_fields` rejects an extra field (the `dto!` macro should already enforce this — confirm it does for the new structs, matching the existing macro's behavior).
- [ ] **Step 2: Run tests, confirm they fail** (types don't exist).
- [ ] **Step 3: Add the constant and five `dto!` structs** exactly as named above, placed near `SourceDetailDto`/`SaveEntryRequest`.
- [ ] **Step 4: Run tests, confirm pass** (`cargo test -p diary-control-contract`).
- [ ] **Step 5: Commit.**

## Task 7: Control — minimal `AssistantRegistry` and per-workflow job exclusivity

**Files:**
- Create: `crates/diary-control/src/assistants.rs`
- Modify: `crates/diary-control/src/lib.rs` (module declaration), `crates/diary-control/src/commands.rs` (`CommandService` struct/constructor), `crates/diary-control/src/server.rs` (`ServerConfig`/`Inner`, `new_job`)
- Test: `crates/diary-control/src/assistants.rs` inline tests; `crates/diary-control/src/server.rs` existing job tests

**Interfaces:**
- Consumes: `diary_core::{AssistantBackendId, DraftAssistant}` (existing trait from Task's unchanged `ports.rs`).
- Produces:
  - `pub struct AssistantRegistry { backends: Vec<std::sync::Arc<dyn DraftAssistant + Send + Sync>> }` with `pub fn new(backends: Vec<Arc<dyn DraftAssistant + Send + Sync>>) -> Self`, `pub fn backend(&self, id: &AssistantBackendId) -> Option<&(dyn DraftAssistant + Send + Sync)>` (matches by `descriptor().id`), `pub fn descriptors(&self) -> Vec<AssistantBackendDescriptor>`. This plan registers no production backend; tests construct it with fakes only, and the capabilities route (Task 9) populates its previously-hardcoded empty `assistant_backends: Vec::new()` from `descriptors()`.
  - `CommandService::new` gains one parameter, `registry: Arc<AssistantRegistry>`, stored as a new field; update the one call site that constructs `CommandService` (server startup) to pass `Arc::new(AssistantRegistry::new(Vec::new()))` until a later M8 task registers a real backend.
  - In `server.rs`, a new helper `fn assistant_job_active_for(state: &AppState, workflow_id: &str) -> bool` — true if any `JobRecord` with `kind` in `["source-refinement"]` (extend this list as future actions add kinds) and `workflow_id` matching and `status` in `["queued", "running", "cancellation-requested"]` exists. `new_job` is unchanged; this check is called explicitly by the refine-start handler (Task 9) before calling `new_job`, returning `source_refine_already_active` on conflict — kept as its own function rather than folded into `new_job` because `new_job`'s existing callers (`discovery`, `source-preparation`) are untouched by this plan's exclusivity rule.

- [ ] **Step 1: Write failing tests**: `registry_finds_backend_by_id_and_lists_descriptors` (two fake `DraftAssistant` impls, assert `backend()` finds each by ID and `descriptors()` returns both); `assistant_job_active_for_detects_a_running_source_refinement_job` (insert a `JobRecord` with `kind: "source-refinement"`, `status: "running"`, assert the helper returns `true` for its `workflow_id` and `false` for another).
- [ ] **Step 2: Run tests, confirm they fail** to compile.
- [ ] **Step 3: Implement `AssistantRegistry` in the new file**, wire the `CommandService` constructor parameter through its one call site, and implement `assistant_job_active_for`.
- [ ] **Step 4: Run tests, confirm pass** (`cargo test -p diary-control`).
- [ ] **Step 5: Commit.**

## Task 8: Control — refine job: start, poll, read suggestion

**Files:**
- Modify: `crates/diary-control/src/commands.rs` (new `CommandService` methods)
- Modify: `crates/diary-control/src/server.rs` (new routes and handlers, job runner)
- Test: `crates/diary-control/src/server.rs` / `commands.rs` existing synthetic command-test style

**Interfaces:**
- Consumes: `AssistantRegistry::backend`, `Workflow::record_refine_suggestion`/`refine_suggestion`, `DraftAssistant::invoke(AssistantAction::RefineSource, &DraftAssistantRequest { source_body: Some(body), .. })` (all from prior tasks).
- Produces:
  - `CommandService::start_source_refinement(&self, day: &DiaryDay, expected_revision: u64, source: &SourceRef, backend_id: &AssistantBackendId) -> Result<DraftAssistantResponse, CommandError>` — loads the workflow, finds the source's current snapshot, errors `CommandError::NotFound` if absent, looks up the backend via the registry (errors `CommandError::UnavailableAssistantBackend` if missing), builds the refine system-prompt request (the exact prompt text is this task's one judgment call — write it inline as a `const REFINE_SYSTEM_PROMPT: &str` near the function, encoding the Global Constraints' refine-goal wording), calls `invoke`, and on success calls `workflow.record_refine_suggestion(..)` and persists via `compare_and_swap`.
  - New route `POST /v1/workflows/{id}/sources/{source_id}/refine-requests` — handler `start_refine`, following the `prepare_sources` handler's shape: validates the request body (`RefineRequestDto`), calls `assistant_job_active_for`; if active, returns `ApiError::new(StatusCode::CONFLICT, "source_refine_already_active")`; otherwise `new_job(&state, "source-refinement", &id)`, spawns `run_refine_job`, returns `(StatusCode::ACCEPTED, Json(JobAcceptedDto { job_id }))`.
  - `run_refine_job` (async fn, mirrors `run_prepare_job`'s shape): calls `start_source_refinement` via `blocking`, updates the `JobRecord` status/`result_handle` on completion exactly as `run_prepare_job` does for its own result, and maps `DraftAssistantError` variants to job `error_code`s (`"assistant_backend_unavailable"`, `"assistant_backend_unsupported_language"`, etc., reusing the M8 overview's named problem codes).
  - New route `GET /v1/workflows/{id}/sources/{source_id}/refine-suggestion` — handler `get_refine_suggestion`, returns `RefineSuggestionDto` built from `Workflow::refine_suggestion`, `404` with `"refine-suggestion-not-found"` if none pending.

- [ ] **Step 1: Write failing tests**: `start_source_refinement_calls_backend_and_records_suggestion` (fake `DraftAssistant` returning a canned `SourceRefinement`); `start_source_refinement_unavailable_backend_returns_error`; an Axum integration test (reusing this file's existing in-process server test helper) `post_refine_requests_then_get_suggestion_round_trips`; `post_refine_requests_conflicts_when_already_active` (start one, assert a second returns 409 with `source_refine_already_active`).
- [ ] **Step 2: Run tests, confirm they fail.**
- [ ] **Step 3: Implement** `start_source_refinement`, the route, `run_refine_job`, and `get_refine_suggestion` as specified above.
- [ ] **Step 4: Run tests, confirm pass** (`cargo test -p diary-control`).
- [ ] **Step 5: Commit.**

## Task 9: Control — resolve refinement, direct edit, delete routes

**Files:**
- Modify: `crates/diary-control/src/commands.rs`, `crates/diary-control/src/server.rs`

**Interfaces:**
- Consumes: `AyumiCatalog::overwrite_body`/`delete_source` (Task 3, `delete_source` taking only `source: &SourceRef` after this plan's fix — see Task 3) via a new write port; define `pub trait SourceWriter { fn overwrite_body(&self, source: &SourceRef, new_body: &str, expected_digest: &Digest) -> Result<FreshnessStamp, SourceWriteError>; fn delete_source(&self, source: &SourceRef) -> Result<Vec<PathBuf>, SourceWriteError>; }` in `diary-core/src/ports.rs` (implemented by `AyumiCatalog` in `diary-connectors`), matching the existing `SourceReader`/`SourceCatalog` port-trait convention. `CommandService` gains a constructor parameter `writer: Arc<dyn SourceWriter + Send + Sync>`, stored as a new field, updating the same `CommandService::new` call site Task 7 already touched for `registry`.
- Produces:
  - `CommandService::resolve_source_refinement(&self, day, expected_revision, source, accept: bool, binding_digest: &Digest) -> Result<Workflow, CommandError>` — calls `workflow.resolve_refine_suggestion`; on accept, calls the writer's `overwrite_body`, then `workflow.update_source_body`, then persists; on reject, persists the cleared suggestion.
  - `CommandService::edit_source_body(&self, day, expected_revision, source, new_text: String, expected_digest: &Digest) -> Result<Workflow, CommandError>` — same write-then-core-update sequence as accept above, without a prior suggestion.
  - `CommandService::delete_source(&self, day, expected_revision, source, confirm: bool) -> Result<Workflow, CommandError>` — errors `CommandError::DeleteNotConfirmed` (new variant, code `"source_delete_not_confirmed"`) if `!confirm`; else calls the writer's `delete_source(source)` first (mapping a partial failure to a new `CommandError::DeletePartialFailure(String)`, code `"source_delete_partial_failure"`, naming the failed step), then `workflow.delete_source`, then persists.
  - Routes: `PUT /v1/workflows/{id}/sources/{source_id}/refine-suggestion/resolution` (handler `resolve_refine`), `PUT /v1/workflows/{id}/sources/{source_id}/body` (handler `edit_source_body`), `DELETE /v1/workflows/{id}/sources/{source_id}` (handler `delete_source_route`) — each checks `assistant_job_active_for` first and fails closed with `source_refine_already_active` if any assistant job is active for the workflow (Global Constraints), consistent with `workflow_precondition`/`buffer_precondition` header handling used by `save_entry`/`confirm_entry`.

- [ ] **Step 1: Write failing tests**, one per new `CommandService` method and one per new route, covering: successful accept writes the file and updates the snapshot; reject leaves the file untouched; a stale `binding_digest` on either path returns `source_binding_stale`; delete with `confirm: false` returns `source_delete_not_confirmed` and touches nothing; delete with a traversal-shaped audio link returns `source_delete_partial_failure` (or the writer's `UnsafePath` mapped the same way) and does not delete the note; any of the three routes while an assistant job is active for that workflow returns `source_refine_already_active`.
- [ ] **Step 2: Run tests, confirm they fail.**
- [ ] **Step 3: Implement** the `SourceWriter` port, its `AyumiCatalog` impl (delegating to Task 3's methods), the three `CommandService` methods, the three routes, and the `CommandError` variants/codes.
- [ ] **Step 4: Run tests, confirm pass** (`cargo test -p diary-control`).
- [ ] **Step 5: Commit.**

## Task 10: Control — capabilities route reflects the registry

**Files:**
- Modify: `crates/diary-control/src/server.rs` (the `capabilities` handler, currently hardcoding `assistant_backends: Vec::new()`)
- Test: `crates/diary-control/src/server.rs`

**Interfaces:**
- Consumes: `AssistantRegistry::descriptors` (Task 7).
- Produces: `capabilities` handler now returns `state.0.config.commands`'s registry descriptors mapped to `AssistantBackendDto`, instead of the hardcoded empty vector.

- [ ] **Step 1: Write a failing test** registering one fake backend in the server's test setup and asserting `GET /v1/capabilities` lists it.
- [ ] **Step 2: Run test, confirm it fails** (still empty).
- [ ] **Step 3: Wire the mapping** from `AssistantBackendDescriptor` to `AssistantBackendDto` (fields already line up per Task 1/existing DTO shape — map `supported_actions`/`supported_languages` to their DTO string-list form the same way the existing `AssistantBackendDto` test expects).
- [ ] **Step 4: Run test, confirm pass.**
- [ ] **Step 5: Commit.**

## Task 11: TUI client — five new API calls

**Files:**
- Modify: `crates/diary-tui/src/client/mod.rs`

**Interfaces:**
- Consumes: the five new routes (Tasks 8–9) and their DTOs (Task 6).
- Produces, on `ApiClient<T>` (mirroring the existing `discover`/`prepare_sources`/`confirm_selection` method shapes — same `attachment`, `workflow_id`, `revision` parameters, same `ClientOutcome<..>` return convention):
  - `pub fn start_refine(&self, attachment: &Attachment, workflow_id: &str, source_id: &str, content_digest: &str) -> ClientOutcome<JobAcceptedDto>`
  - `pub fn refine_suggestion(&self, attachment: &Attachment, workflow_id: &str, source_id: &str) -> ClientOutcome<Option<RefineSuggestionDto>>` (`None` on a 404 `refine-suggestion-not-found`, matching this client's existing "typed absence" convention if one exists — check `SourceDetailResponse`'s handling of 404 for the exact idiom to mirror).
  - `pub fn resolve_refine(&self, attachment: &Attachment, workflow_id: &str, source_id: &str, accept: bool, binding_digest: &str) -> ClientOutcome<WorkflowDto>`
  - `pub fn edit_source_body(&self, attachment: &Attachment, workflow_id: &str, source_id: &str, text: &str, expected_content_digest: &str) -> ClientOutcome<WorkflowDto>`
  - `pub fn delete_source(&self, attachment: &Attachment, workflow_id: &str, source_id: &str) -> ClientOutcome<()>` (always sends `confirm: true`; the TUI's own confirmation prompt, Task 12, is what gates calling this method at all).

- [ ] **Step 1: Write failing tests** against this file's existing fake-`Transport` test harness: one per method, happy path plus one typed-problem path (e.g. `source_binding_stale`) each.
- [ ] **Step 2: Run tests, confirm they fail** to compile.
- [ ] **Step 3: Implement the five methods**, matching the request/response wiring of the nearest existing method (`prepare_sources` for the two POST/job-returning ones, `confirm_selection` for the PUT-returning-`WorkflowDto` ones).
- [ ] **Step 4: Run tests, confirm pass** (`cargo test -p diary-tui --lib client::`).
- [ ] **Step 5: Commit.**

## Task 12: TUI Sources stage — refine, approve/reject, edit, delete actions

**Files:**
- Modify: `crates/diary-tui/src/sources_view.rs` (`SourcesViewModel`)
- Test: `crates/diary-tui/src/sources_view.rs`

**Interfaces:**
- Consumes: Task 11's five client methods; the existing `poll_job` method (for the refine job, same pattern as `pending_discovery_job`/`pending_preparation_job`).
- Produces, on `SourcesViewModel`:
  - New fields: `pub pending_refine_job: Option<String>`, `pub active_refine_suggestion: Option<RefineSuggestionDto>`, `pub pending_delete_confirmation: Option<String>` (a source ID awaiting the TUI's own confirm keypress before `delete_source` is called).
  - `pub fn start_refine<T: Transport>(&mut self, client: &ApiClient<T>, attachment: &Attachment, workflow_id: &str, source_id: &str)` — uses `self.active_detail`'s `content_digest`; mirrors `start_discovery`'s error-handling shape exactly.
  - `pub fn load_refine_suggestion<T: Transport>(&mut self, ..)`, `pub fn resolve_refine<T: Transport>(&mut self, .., accept: bool)`, `pub fn edit_source_body<T: Transport>(&mut self, .., new_text: String)`, `pub fn request_delete(&mut self, source_id: &str)` (sets `pending_delete_confirmation`, no network call), `pub fn confirm_delete<T: Transport>(&mut self, ..)` (clears `pending_delete_confirmation` and calls the client's `delete_source`), `pub fn cancel_delete(&mut self)` (clears `pending_delete_confirmation` without calling anything) — each following `start_discovery`'s `ClientOutcome` match-and-record-`last_error` shape.

- [ ] **Step 1: Write failing tests** for each new method against the existing fake-`Transport`/fake-`ApiClient` test harness in this file: refine start records `pending_refine_job`; a resolved suggestion clears `active_refine_suggestion`; `request_delete` sets but does not act, `confirm_delete` calls through, `cancel_delete` clears without calling.
- [ ] **Step 2: Run tests, confirm they fail** to compile.
- [ ] **Step 3: Implement the fields and methods** above.
- [ ] **Step 4: Run tests, confirm pass** (`cargo test -p diary-tui --lib sources_view::`).
- [ ] **Step 5: Commit.**

## Task 13: TUI rendering and keybindings for the new Sources-stage actions

**Files:**
- Modify: whichever file renders the Sources-stage Main Pane and dispatches its keybindings today (locate it from `crates/diary-tui/src/sources_view.rs`'s callers — search the crate for where `SourcesViewModel` methods are invoked from input handling, e.g. an `app.rs`/`stages/sources.rs`-shaped file; this plan does not guess its exact name since it wasn't inspected, but the implementer finds it by following `start_discovery`'s existing call site)
- Test: that file's existing input-handling test style

**Interfaces:**
- Consumes: Task 12's `SourcesViewModel` methods.
- Produces: new keybindings for Refine, Approve, Reject, Edit (reusing the Draft editor's existing multiline-edit component — find and reuse it rather than building a second editor), and Delete (showing a confirmation prompt naming the source's `source_note_name` and, once available, the audio paths the delete will remove — the exact prompt copy is this task's judgment call, kept short and factual per the project's existing UX copy style) before calling `request_delete`/`confirm_delete`.

- [ ] **Step 1: Write a failing test** per new keybinding/action, following this file's existing pattern for an action like include/exclude toggling or the Draft editor's save keybinding.
- [ ] **Step 2: Run tests, confirm they fail.**
- [ ] **Step 3: Implement the bindings and rendering**, reusing the existing editor component for Edit and the existing per-source inspection rendering as the base to extend.
- [ ] **Step 4: Run tests, confirm pass.**
- [ ] **Step 5: Commit.**

## Task 14: Documentation and gate

**Files:**
- Modify: `AGENTS.md`, `docs/phase-1/milestone-08-draft-assistants-overview.md`, `docs/product-behavior.md`, `docs/research/markdown-input-format.md`, `docs/phase-1/changelog.md` (per the spec's "Documentation impact" section)

- [ ] **Step 1:** Add the one-line `AGENTS.md` pointer for the hard-delete/write-back exceptions (spec "Owner decisions carried into this spec").
- [ ] **Step 2:** Add this feature's scope to the M8 overview's completion checklist, decision/approval gate, and in/out-of-scope sections.
- [ ] **Step 3:** Update `docs/product-behavior.md`'s Sources-stage step and audio/attachment boundary bullet, and `docs/research/markdown-input-format.md`'s "does not... delete... audio" sentence, per the spec's narrowing.
- [ ] **Step 4:** Add a `docs/phase-1/changelog.md` entry recording this as an implemented owner decision.
- [ ] **Step 5:** Run the complete gate: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features`, `swift build`, `swift test`. Fix anything that fails before committing.
- [ ] **Step 6: Commit.**
