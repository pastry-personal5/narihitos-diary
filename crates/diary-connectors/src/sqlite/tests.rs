use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

use diary_core::{
    Clock, DateEvidence, Destination, InspectionOutcome, PublicationCoordinator, PublicationResult,
    SourceCandidate, WriteOutcome, start_default_workflow,
};

use super::*;

static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn empty_database_migrates_reopens_and_every_prior_schema_upgrades() {
    let scratch = Scratch::new("migration");
    let store = SqliteStore::open(scratch.db()).unwrap();
    drop(store);
    assert_eq!(schema_version(scratch.db()), CURRENT_SCHEMA);
    SqliteStore::open(scratch.db()).unwrap();

    let prior = Scratch::new("migration-v1");
    let connection = Connection::open(prior.db()).unwrap();
    connection.execute_batch(MIGRATION_1).unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    drop(connection);
    SqliteStore::open(prior.db()).unwrap();
    assert_eq!(schema_version(prior.db()), CURRENT_SCHEMA);

    let prior = Scratch::new("migration-v2");
    let connection = Connection::open(prior.db()).unwrap();
    connection.execute_batch(MIGRATION_1).unwrap();
    connection.execute_batch(MIGRATION_2).unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
    drop(connection);
    SqliteStore::open(prior.db()).unwrap();
    assert_eq!(schema_version(prior.db()), CURRENT_SCHEMA);

    let prior = Scratch::new("migration-v3-with-data");
    let connection = Connection::open(prior.db()).unwrap();
    connection.execute_batch(MIGRATION_1).unwrap();
    connection.execute_batch(MIGRATION_2).unwrap();
    connection.execute_batch(MIGRATION_3).unwrap();
    connection
        .execute(
            "INSERT INTO workflows (
                   day, state, revision, created_at, updated_at, language, style,
                   composition_path, model_material_stale, explicit_empty_selection,
                   selection_digest, invalidation_kind, invalidation_connector,
                   invalidation_source_id, job_status, all_verified_at, discarded_at,
                   entry_text, buffer_version, confirmed_buffer_version,
                   confirmed_entry_digest, confirmed_at, model_draft
                 ) VALUES (
                   '2026-10-29', 'discovered', 1, 1, 2, 'en-US', 'concise',
                   'manual', 0, 0, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
                   'Recovered text', 0, NULL, NULL, NULL, 'Legacy draft'
                 )",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO source_candidates (
                   day, position, connector, source_id, directory_day, header_day,
                   warning, availability
                 ) VALUES (
                   '2026-10-29', 0, 'ayumi', '2026/10/29/note.md',
                   '2026-10-29', NULL, NULL, 'resident'
                 )",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO reviewed_memories (day, position, memory_text)
                 VALUES ('2026-10-29', 0, 'Legacy reviewed memory')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO workflows (
                   day, state, revision, created_at, updated_at, language, style,
                   composition_path, model_material_stale, explicit_empty_selection,
                   selection_digest, invalidation_kind, invalidation_connector,
                   invalidation_source_id, job_status, all_verified_at, discarded_at,
                   entry_text, buffer_version, confirmed_buffer_version,
                   confirmed_entry_digest, confirmed_at, model_draft
                 ) VALUES (
                   '2026-10-28', 'started', 0, 1, 1, NULL, 'concise',
                   'hosted', 0, 0, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
                   '', 0, NULL, NULL, NULL, NULL
                 )",
            [],
        )
        .unwrap();
    let preserved_digest = Digest::of_bytes(b"preserved-synthetic");
    connection
        .execute(
            "INSERT INTO source_snapshots (
                   day, position, connector, source_id, file_identity, byte_size,
                   modified_at, content_digest, readable_body
                 ) VALUES (
                   '2026-10-28', 0, 'fake', 'snapshot-1', 'file-1', 19,
                   2, ?1, 'Preserved snapshot.'
                 )",
            [preserved_digest.as_str()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO output_plans (
                   day, plan_id, entry_digest, selection_digest, plan_digest,
                   created_at, approval_plan_digest, approval_revision,
                   approved_at, approval_destination_only
                 ) VALUES (
                   '2026-10-28', 'plan-preserved', ?1, ?1, ?1,
                   3, NULL, NULL, NULL, NULL
                 )",
            [preserved_digest.as_str()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO destination_plans (
                   day, position, kind, destination, target_identity,
                   intended_state_digest, payload_digest, inspected_state_digest
                 ) VALUES (
                   '2026-10-28', 0, 'local', 'local', 'target-preserved',
                   ?1, ?1, ?1
                 )",
            [preserved_digest.as_str()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO destination_progress (day, destination, status)
                 VALUES ('2026-10-28', 'local', 'pending')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO operation_intents (
                   attempt_id, plan_id, day, destination, target_identity,
                   entry_digest, selection_digest, payload_digest, created_at,
                   cancellation_requested, terminal
                 ) VALUES (
                   'attempt-migrated', 'plan-migrated', '2026-10-29', 'local',
                   'target-migrated', ?1, ?1, ?1, 3, 0, NULL
                 )",
            [Digest::of_bytes(b"synthetic").as_str()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO operation_intents (
                   attempt_id, plan_id, day, destination, target_identity,
                   entry_digest, selection_digest, payload_digest, created_at,
                   cancellation_requested, terminal
                 ) VALUES (
                   'attempt-receipted', 'plan-receipted', '2026-10-29', 'notion',
                   'target-receipted', ?1, ?1, ?1, 4, 0, 'verified'
                 )",
            [Digest::of_bytes(b"receipted-synthetic").as_str()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO receipts (
                   receipt_id, attempt_id, day, destination, target_identity,
                   version, entry_digest, selection_digest, payload_digest, verified_at
                 ) VALUES (
                   'receipt-migrated', 'attempt-receipted', '2026-10-29', 'notion',
                   'target-receipted', 1, ?1, ?1, ?1, 5
                 )",
            [Digest::of_bytes(b"receipted-synthetic").as_str()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO receipt_returned_ids (receipt_id, position, returned_id)
                 VALUES ('receipt-migrated', 0, 'block-synthetic')",
            [],
        )
        .unwrap();
    connection.pragma_update(None, "user_version", 3).unwrap();
    drop(connection);

    let store = SqliteStore::open(prior.db()).unwrap();
    let migrated = store.load(&day("2026-10-29")).unwrap().unwrap();
    assert_eq!(migrated.state(), WorkflowState::Discovered);
    assert_eq!(migrated.entry().text, "Recovered text");
    assert_eq!(migrated.assistant_selection(), None);
    assert_eq!(
        migrated.candidates()[0].source_note_name,
        "2026/10/29/note.md"
    );
    assert_eq!(store.load_preferences().unwrap(), Preferences::default());
    assert_eq!(
        store
            .unresolved_intents(Some(&day("2026-10-29")))
            .unwrap()
            .len(),
        1
    );
    let receipts = store.receipts(&day("2026-10-29")).unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].receipt_id, "receipt-migrated");
    assert_eq!(receipts[0].returned_ids, ["block-synthetic"]);
    let connection = Connection::open(prior.db()).unwrap();
    let migrated_draft: String = connection
        .query_row(
            "SELECT assistant_draft FROM workflows WHERE day = '2026-10-29'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(migrated_draft, "Legacy draft");
    let migrated_memory_artifact: (String, String, bool) = connection
        .query_row(
            "SELECT artifact_id, reviewed_text, stale
                 FROM reviewed_memory_artifacts WHERE day = '2026-10-29'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        migrated_memory_artifact,
        (
            "migrated-memory-20261029".to_owned(),
            "Legacy reviewed memory".to_owned(),
            true,
        )
    );
    let migrated_draft_artifact: (String, String, bool) = connection
        .query_row(
            "SELECT artifact_id, draft_text, stale
                 FROM draft_proposals WHERE day = '2026-10-29'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        migrated_draft_artifact,
        (
            "migrated-draft-20261029".to_owned(),
            "Legacy draft".to_owned(),
            true,
        )
    );
    let preserved: (String, String, String, String) = connection
        .query_row(
            "SELECT s.readable_body, p.plan_id, d.target_identity, g.status
                 FROM source_snapshots s
                 JOIN output_plans p ON p.day = s.day
                 JOIN destination_plans d ON d.day = p.day
                 JOIN destination_progress g ON g.day = p.day
                 WHERE s.day = '2026-10-28'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        preserved,
        (
            "Preserved snapshot.".to_owned(),
            "plan-preserved".to_owned(),
            "target-preserved".to_owned(),
            "pending".to_owned(),
        )
    );
    assert_eq!(schema_version(prior.db()), CURRENT_SCHEMA);
}

#[test]
fn unsupported_and_corrupt_schema_fail_without_reset() {
    let newer = Scratch::new("newer-schema");
    let connection = Connection::open(newer.db()).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);
    assert_eq!(
        SqliteStore::open(newer.db()).unwrap_err(),
        StoreError::UnsupportedSchema(99)
    );
    assert_eq!(schema_version(newer.db()), 99);

    let incomplete = Scratch::new("incomplete-schema");
    let connection = Connection::open(incomplete.db()).unwrap();
    connection
        .pragma_update(None, "user_version", CURRENT_SCHEMA)
        .unwrap();
    drop(connection);
    assert!(matches!(
        SqliteStore::open(incomplete.db()),
        Err(StoreError::InvalidRecord(_))
    ));
    assert_eq!(schema_version(incomplete.db()), CURRENT_SCHEMA);

    let corrupt = Scratch::new("corrupt");
    fs::write(corrupt.db(), b"not a sqlite database").unwrap();
    assert!(matches!(
        SqliteStore::open(corrupt.db()),
        Err(StoreError::Corrupt(_))
    ));
    assert_eq!(fs::read(corrupt.db()).unwrap(), b"not a sqlite database");
}

#[test]
fn current_schema_rejects_partial_assistant_selection_and_invalid_source_note_name() {
    let partial = Scratch::new("partial-assistant-selection");
    let store = SqliteStore::open(partial.db()).unwrap();
    create_started(&store, "2026-10-29");
    drop(store);
    let connection = Connection::open(partial.db()).unwrap();
    connection
        .execute(
            "UPDATE workflows SET assistant_backend_id = 'hosted-default'
                 WHERE day = '2026-10-29'",
            [],
        )
        .unwrap();
    drop(connection);
    let store = SqliteStore::open(partial.db()).unwrap();
    assert_eq!(
        store.load(&day("2026-10-29")).unwrap_err(),
        StoreError::InvalidRecord("partial assistant selection".to_owned())
    );

    let invalid_name = Scratch::new("invalid-source-note-name");
    let store = SqliteStore::open(invalid_name.db()).unwrap();
    let mut workflow = create_started(&store, "2026-10-29");
    workflow
        .discover(
            workflow.revision(),
            vec![SourceCandidate {
                source: SourceRef::new("fake", "note-1").unwrap(),
                source_note_name: "note-1.md".to_owned(),
                date_evidence: DateEvidence {
                    directory_day: Some(day("2026-10-29")),
                    header_day: None,
                    warning: None,
                },
                availability: SourceAvailability::Resident,
            }],
            Timestamp(1),
        )
        .unwrap();
    store.compare_and_swap(0, Some(0), &workflow).unwrap();
    drop(store);
    let connection = Connection::open(invalid_name.db()).unwrap();
    connection
        .execute(
            "UPDATE source_candidates SET source_note_name = ''
                 WHERE day = '2026-10-29'",
            [],
        )
        .unwrap();
    drop(connection);
    let store = SqliteStore::open(invalid_name.db()).unwrap();
    assert!(matches!(
        store.load(&day("2026-10-29")),
        Err(StoreError::InvalidRecord(_))
    ));
}

#[test]
fn mirror_destination_round_trips_through_store() {
    let scratch = Scratch::new("mirror-kind");
    let store = SqliteStore::open(scratch.db()).unwrap();
    let clock = FixedClock {
        now: Timestamp(0),
        day: day("2026-10-29"),
    };
    let mut workflow = match start_default_workflow(&store, &clock).unwrap() {
        StartOutcome::Created(workflow) => workflow,
        _ => panic!("expected a new workflow"),
    };
    make_confirmed(&mut workflow);
    let plan = OutputPlan::new(
        "mirror-plan",
        workflow
            .entry()
            .confirmed
            .as_ref()
            .unwrap()
            .entry_digest
            .clone(),
        workflow.selection_digest().unwrap().clone(),
        vec![
            target("local"),
            DestinationPlan::new(
                DestinationKind::Mirror,
                "icloud",
                "icloud-target",
                Digest::of_bytes(b"icloud-state"),
                Digest::of_bytes(b"icloud-payload"),
                Digest::of_bytes(b"icloud-inspected"),
            )
            .unwrap(),
        ],
        Timestamp(7),
    )
    .unwrap();
    workflow
        .preview_output(workflow.revision(), plan, Timestamp(7))
        .unwrap();
    store.compare_and_swap(0, Some(0), &workflow).unwrap();
    let reopened = SqliteStore::open(scratch.db())
        .unwrap()
        .load(&day("2026-10-29"))
        .unwrap()
        .unwrap();
    // Progress rows reload ordered by destination name, so compare as sets.
    assert_eq!(reopened.output_plan(), workflow.output_plan());
    let sorted = |workflow: &Workflow| {
        let mut progress: Vec<_> = workflow
            .destination_progress()
            .iter()
            .map(|item| (item.destination.clone(), item.status))
            .collect();
        progress.sort_by(|left, right| left.0.cmp(&right.0));
        progress
    };
    assert_eq!(sorted(&reopened), sorted(&workflow));
    assert!(
        reopened
            .output_plan()
            .unwrap()
            .targets
            .iter()
            .any(|target| target.kind == DestinationKind::Mirror)
    );
}

#[test]
fn fixed_clock_starts_one_local_day_and_completed_day_is_preserved() {
    let scratch = Scratch::new("default-day");
    let store = SqliteStore::open(scratch.db()).unwrap();
    let clock = FixedClock {
        now: Timestamp(0),
        day: day("2026-10-29"),
    };
    assert!(matches!(
        start_default_workflow(&store, &clock).unwrap(),
        StartOutcome::Created(_)
    ));
    let resumed = match start_default_workflow(&store, &clock).unwrap() {
        StartOutcome::Resumed(workflow) => workflow,
        _ => panic!("unfinished day must resume"),
    };
    let mut completed = resumed;
    make_confirmed(&mut completed);
    make_completed(&mut completed, "default-day-plan", Timestamp(50));
    store.compare_and_swap(0, Some(0), &completed).unwrap();
    assert!(matches!(
        start_default_workflow(&store, &clock).unwrap(),
        StartOutcome::Completed(_)
    ));
}

#[test]
fn preferences_and_full_workflow_reopen_without_prohibited_metadata() {
    let scratch = Scratch::new("roundtrip");
    let store = SqliteStore::open(scratch.db()).unwrap();
    store
        .save_preferences(&Preferences {
            last_language: Some(WritingLanguage::KoKr),
            last_assistant_backend_id: Some(
                AssistantBackendId::parse("apple-foundation-models").unwrap(),
            ),
        })
        .unwrap();
    drop(store);

    let store = SqliteStore::open(scratch.db()).unwrap();
    assert_eq!(
        store.load_preferences().unwrap(),
        Preferences {
            last_language: Some(WritingLanguage::KoKr),
            last_assistant_backend_id: Some(
                AssistantBackendId::parse("apple-foundation-models").unwrap(),
            ),
        }
    );
    let mut workflow = create_started(&store, "2026-10-28");
    make_confirmed(&mut workflow);
    workflow
        .set_assistant_selection(
            workflow.revision(),
            Some(AssistantSelection::new(
                AssistantBackendId::parse("apple-foundation-models").unwrap(),
                CompositionPath::AppleOnDevice,
            )),
            Timestamp(6),
        )
        .unwrap();
    workflow
        .confirm_entry(workflow.revision(), workflow.entry().version, Timestamp(6))
        .unwrap();
    let plan = OutputPlan::new(
        "persisted-plan",
        workflow
            .entry()
            .confirmed
            .as_ref()
            .unwrap()
            .entry_digest
            .clone(),
        workflow.selection_digest().unwrap().clone(),
        vec![target("local")],
        Timestamp(7),
    )
    .unwrap();
    workflow
        .preview_output(workflow.revision(), plan.clone(), Timestamp(7))
        .unwrap();
    workflow
        .approve(workflow.revision(), &plan.plan_digest, Timestamp(8))
        .unwrap();
    store.compare_and_swap(0, Some(0), &workflow).unwrap();
    let reopened = SqliteStore::open(scratch.db())
        .unwrap()
        .load(&day("2026-10-28"))
        .unwrap()
        .unwrap();
    assert_eq!(reopened, workflow);
    assert_eq!(reopened.entry().text, "An invented ordinary day.");

    let schema: String = Connection::open(scratch.db())
        .unwrap()
        .query_row(
            "SELECT group_concat(sql, ' ') FROM sqlite_master WHERE sql IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    for prohibited in [
        "credential",
        "token",
        "front_matter",
        "location",
        "weather",
        "attachment",
        "audio",
        "notion_root",
    ] {
        assert!(!schema.to_ascii_lowercase().contains(prohibited));
    }
}

#[test]
fn compare_and_swap_rejects_concurrent_workflow_and_buffer_updates() {
    let scratch = Scratch::new("concurrency");
    let store = SqliteStore::open(scratch.db()).unwrap();
    let workflow = create_started(&store, "2026-10-28");
    let mut first = workflow.clone();
    let mut second = workflow;
    first.discover(0, Vec::new(), Timestamp(1)).unwrap();
    second.discover(0, Vec::new(), Timestamp(1)).unwrap();
    store.compare_and_swap(0, None, &first).unwrap();
    assert_eq!(
        store.compare_and_swap(0, None, &second),
        Err(StoreError::StaleRevision)
    );

    let mut current = store.load(&day("2026-10-28")).unwrap().unwrap();
    current
        .confirm_sources(current.revision(), Vec::new(), true, Timestamp(2))
        .unwrap();
    current
        .begin_manual_composition(current.revision(), Timestamp(3))
        .unwrap();
    current
        .set_language(current.revision(), WritingLanguage::EnUs, Timestamp(4))
        .unwrap();
    store.compare_and_swap(1, Some(0), &current).unwrap();
    let mut editor_a = store.load(&day("2026-10-28")).unwrap().unwrap();
    let mut editor_b = editor_a.clone();
    let revision = editor_a.revision();
    editor_a
        .save_entry(0, "editor A".to_owned(), Timestamp(5))
        .unwrap();
    editor_b
        .save_entry(0, "editor B".to_owned(), Timestamp(5))
        .unwrap();
    store
        .compare_and_swap(revision, Some(0), &editor_a)
        .unwrap();
    assert_eq!(
        store.compare_and_swap(revision, Some(0), &editor_b),
        Err(StoreError::StaleBuffer)
    );
    let recovered = SqliteStore::open(scratch.db())
        .unwrap()
        .load(&day("2026-10-28"))
        .unwrap()
        .unwrap();
    assert_eq!(recovered.entry().text, "editor A");
    assert_eq!(recovered.entry().version, 1);

    let race = create_started(&store, "2026-10-30");
    let mut left = race.clone();
    let mut right = race;
    left.discover(0, Vec::new(), Timestamp(1)).unwrap();
    right.discover(0, Vec::new(), Timestamp(1)).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let left_store = store.clone();
    let left_barrier = Arc::clone(&barrier);
    let left_result = std::thread::spawn(move || {
        left_barrier.wait();
        left_store.compare_and_swap(0, None, &left)
    });
    let right_store = store.clone();
    let right_barrier = Arc::clone(&barrier);
    let right_result = std::thread::spawn(move || {
        right_barrier.wait();
        right_store.compare_and_swap(0, None, &right)
    });
    let results = [left_result.join().unwrap(), right_result.join().unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(StoreError::StaleRevision)))
            .count(),
        1
    );
}

#[test]
fn intent_precedes_effect_crash_recovery_is_per_day_and_duplicate_is_idempotent() {
    let scratch = Scratch::new("intent");
    let store = SqliteStore::open(scratch.db()).unwrap();
    let target = target("local");
    let intent = OperationIntent::new(
        "attempt-1",
        "plan-1",
        day("2026-10-28"),
        &target,
        Digest::of_bytes(b"entry"),
        Digest::of_bytes(b"selection"),
        Timestamp(1),
    )
    .unwrap();
    assert!(store.record_intent(&intent).unwrap());
    assert!(!store.record_intent(&intent).unwrap());
    let mut reused_attempt = intent.clone();
    reused_attempt.plan_id = "different-plan".to_owned();
    assert!(matches!(
        store.record_intent(&reused_attempt),
        Err(StoreError::InvalidRecord(_))
    ));
    let mismatched_receipt = Receipt {
        receipt_id: "bad-receipt".to_owned(),
        attempt_id: intent.attempt_id.clone(),
        day: day("2026-10-29"),
        destination: intent.destination.clone(),
        target_identity: intent.target_identity.clone(),
        version: 1,
        entry_digest: intent.entry_digest.clone(),
        selection_digest: intent.selection_digest.clone(),
        payload_digest: intent.payload_digest.clone(),
        returned_ids: Vec::new(),
        verified_at: Timestamp(1),
    };
    assert!(matches!(
        store.record_receipt(&mismatched_receipt),
        Err(StoreError::InvalidRecord(_))
    ));
    let mut destination = FakeDestination {
        store: store.clone(),
        effect_applied: false,
        inspection: InspectionOutcome::Matching {
            receipt_id: "receipt-1".to_owned(),
            returned_ids: vec!["external-1".to_owned()],
        },
    };
    assert!(matches!(
        destination.write(&intent),
        WriteOutcome::Verified { .. }
    ));
    drop(store);

    let reopened = SqliteStore::open(scratch.db()).unwrap();
    assert_eq!(
        reopened.reconciliation_gate(&day("2026-10-28")).unwrap(),
        ReconciliationGate::Reconciling
    );
    assert_eq!(
        reopened.reconciliation_gate(&day("2026-10-29")).unwrap(),
        ReconciliationGate::Clear
    );
    reopened.request_cancellation("attempt-1").unwrap();
    assert!(reopened.unresolved_intents(None).unwrap()[0].cancellation_requested);
    let result =
        PublicationCoordinator::reconcile(&reopened, &mut destination, &intent, 1, Timestamp(2))
            .unwrap();
    assert!(matches!(result, PublicationResult::Verified(_)));
    assert_eq!(
        reopened.reconciliation_gate(&day("2026-10-28")).unwrap(),
        ReconciliationGate::Clear
    );
    let duplicate =
        PublicationCoordinator::execute(&reopened, &mut destination, &intent, 1, Timestamp(3))
            .unwrap();
    assert_eq!(duplicate, PublicationResult::DuplicateCommand);
    assert_eq!(reopened.receipts(&day("2026-10-28")).unwrap().len(), 1);

    let absent_intent = OperationIntent::new(
        "attempt-absent",
        "plan-absent",
        day("2026-10-29"),
        &target,
        Digest::of_bytes(b"entry"),
        Digest::of_bytes(b"selection"),
        Timestamp(4),
    )
    .unwrap();
    assert!(reopened.record_intent(&absent_intent).unwrap());
    let mut absent_destination = FakeDestination {
        store: reopened.clone(),
        effect_applied: true,
        inspection: InspectionOutcome::Absent,
    };
    assert_eq!(
        PublicationCoordinator::reconcile(
            &reopened,
            &mut absent_destination,
            &absent_intent,
            1,
            Timestamp(5),
        )
        .unwrap(),
        PublicationResult::SafeToRetry
    );
    assert_eq!(
        reopened.reconciliation_gate(&absent_intent.day).unwrap(),
        ReconciliationGate::Clear
    );
}

#[test]
fn retention_boundaries_restart_and_preserve_receipts_and_unresolved_intents() {
    let scratch = Scratch::new("retention");
    let store = SqliteStore::open(scratch.db()).unwrap();
    let mut workflow = create_started(&store, "2026-10-28");
    make_confirmed(&mut workflow);
    make_completed(&mut workflow, "plan-v1", Timestamp(100));
    store.compare_and_swap(0, Some(0), &workflow).unwrap();

    let first_intent = OperationIntent::new(
        "verified-v1",
        "plan-v1",
        day("2026-10-28"),
        &target("local"),
        Digest::of_bytes(b"entry-v1"),
        Digest::of_bytes(b"selection"),
        Timestamp(100),
    )
    .unwrap();
    store.record_intent(&first_intent).unwrap();
    store
        .record_receipt(&Receipt {
            receipt_id: "receipt-v1".to_owned(),
            attempt_id: first_intent.attempt_id.clone(),
            day: first_intent.day.clone(),
            destination: first_intent.destination.clone(),
            target_identity: first_intent.target_identity.clone(),
            version: 1,
            entry_digest: first_intent.entry_digest.clone(),
            selection_digest: first_intent.selection_digest.clone(),
            payload_digest: first_intent.payload_digest.clone(),
            returned_ids: vec!["local-v1".to_owned()],
            verified_at: Timestamp(100),
        })
        .unwrap();

    let current = store.load(&day("2026-10-28")).unwrap().unwrap();
    let mut amended = current.clone();
    amended
        .seed_amendment(
            amended.revision(),
            "An amended invented day.".to_owned(),
            AmendmentSeed {
                file_digest: Digest::of_bytes(b"file"),
                receipt_version: 1,
                hand_edited: false,
            },
            Timestamp(150),
        )
        .unwrap();
    amended
        .confirm_entry(amended.revision(), amended.entry().version, Timestamp(160))
        .unwrap();
    make_completed(&mut amended, "plan-v2", Timestamp(200));
    store
        .compare_and_swap(current.revision(), Some(current.entry().version), &amended)
        .unwrap();
    let second_intent = OperationIntent::new(
        "verified-v2",
        "plan-v2",
        day("2026-10-28"),
        &target("local"),
        Digest::of_bytes(b"entry-v2"),
        Digest::of_bytes(b"selection"),
        Timestamp(200),
    )
    .unwrap();
    store.record_intent(&second_intent).unwrap();
    store
        .record_receipt(&Receipt {
            receipt_id: "receipt-v2".to_owned(),
            attempt_id: second_intent.attempt_id.clone(),
            day: second_intent.day.clone(),
            destination: second_intent.destination.clone(),
            target_identity: second_intent.target_identity.clone(),
            version: 2,
            entry_digest: second_intent.entry_digest.clone(),
            selection_digest: second_intent.selection_digest.clone(),
            payload_digest: second_intent.payload_digest.clone(),
            returned_ids: vec!["local-v2".to_owned()],
            verified_at: Timestamp(200),
        })
        .unwrap();
    assert!(
        store
            .purge_expired(Timestamp(100 + 7 * Timestamp::SECONDS_PER_DAY))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .purge_expired(Timestamp(200 + 7 * Timestamp::SECONDS_PER_DAY))
            .unwrap(),
        vec![day("2026-10-28")]
    );
    let recovered = store.load(&day("2026-10-28")).unwrap().unwrap();
    assert!(recovered.entry().text.is_empty());
    assert!(
        recovered
            .snapshots()
            .iter()
            .all(|snapshot| snapshot.body.is_empty())
    );
    let receipts = store.receipts(&day("2026-10-28")).unwrap();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0].version, 1);
    assert_eq!(receipts[1].version, 2);
    assert!(
        store
            .purge_expired(Timestamp(200 + 7 * Timestamp::SECONDS_PER_DAY))
            .unwrap()
            .is_empty()
    );

    let abandoned = create_started(&store, "2026-10-29");
    let intent = OperationIntent::new(
        "unresolved",
        "plan-x",
        abandoned.day().clone(),
        &target("remote"),
        Digest::of_bytes(b"abandoned-entry"),
        Digest::of_bytes(b"abandoned-selection"),
        Timestamp(0),
    )
    .unwrap();
    store.record_intent(&intent).unwrap();
    assert!(
        store
            .purge_expired(Timestamp(30 * Timestamp::SECONDS_PER_DAY - 1))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .purge_expired(Timestamp(30 * Timestamp::SECONDS_PER_DAY))
            .unwrap(),
        vec![day("2026-10-29")]
    );
    assert_eq!(
        store
            .unresolved_intents(Some(abandoned.day()))
            .unwrap()
            .len(),
        1
    );
    let purged_abandoned = store.load(abandoned.day()).unwrap().unwrap();
    store
        .discard(
            purged_abandoned.day(),
            purged_abandoned.revision(),
            Timestamp(99),
        )
        .unwrap();
    assert_eq!(
        store
            .unresolved_intents(Some(abandoned.day()))
            .unwrap()
            .len(),
        1
    );

    let restart_day = day("2026-10-30");
    let before_discard = create_started(&store, restart_day.as_str());
    store
        .discard(&restart_day, before_discard.revision(), Timestamp(10))
        .unwrap();
    let discarded = store.load(&restart_day).unwrap().unwrap();
    assert_eq!(discarded.revision(), before_discard.revision() + 1);
    assert_eq!(
        discarded.entry().version,
        before_discard.entry().version + 1
    );
    let mut restarted = discarded.clone();
    restarted
        .discover(restarted.revision(), Vec::new(), Timestamp(20))
        .unwrap();
    store
        .compare_and_swap(
            discarded.revision(),
            Some(discarded.entry().version),
            &restarted,
        )
        .unwrap();
    assert!(
        store
            .purge_expired(Timestamp(20 + 30 * Timestamp::SECONDS_PER_DAY - 1))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .purge_expired(Timestamp(20 + 30 * Timestamp::SECONDS_PER_DAY))
            .unwrap(),
        vec![restart_day]
    );
}

struct FixedClock {
    now: Timestamp,
    day: DiaryDay,
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.now
    }

    fn local_day(&self) -> DiaryDay {
        self.day.clone()
    }
}

struct FakeDestination {
    store: SqliteStore,
    effect_applied: bool,
    inspection: InspectionOutcome,
}

impl Destination for FakeDestination {
    fn write(&mut self, intent: &OperationIntent) -> WriteOutcome {
        assert_eq!(
            self.store.reconciliation_gate(&intent.day).unwrap(),
            ReconciliationGate::Reconciling,
            "intent must be durable before the effect"
        );
        self.effect_applied = true;
        WriteOutcome::Verified {
            receipt_id: "crash-window-receipt".to_owned(),
            returned_ids: vec!["external-1".to_owned()],
        }
    }

    fn inspect(&mut self, _intent: &OperationIntent) -> InspectionOutcome {
        assert!(self.effect_applied);
        self.inspection.clone()
    }
}

fn create_started(store: &SqliteStore, raw_day: &str) -> Workflow {
    match store
        .start_or_resume(day(raw_day), Timestamp(0), &Preferences::default())
        .unwrap()
    {
        StartOutcome::Created(workflow) => workflow,
        _ => panic!("fixture day must be new"),
    }
}

fn make_confirmed(workflow: &mut Workflow) {
    let source = SourceRef::new("fake", "note-1").unwrap();
    workflow
        .discover(
            workflow.revision(),
            vec![SourceCandidate {
                source: source.clone(),
                source_note_name: "note-1.md".to_owned(),
                date_evidence: DateEvidence {
                    directory_day: Some(workflow.day().clone()),
                    header_day: Some(workflow.day().clone()),
                    warning: None,
                },
                availability: SourceAvailability::Resident,
            }],
            Timestamp(1),
        )
        .unwrap();
    let body = "A synthetic note.";
    workflow
        .confirm_sources(
            workflow.revision(),
            vec![SourceSnapshot {
                source,
                stamp: FreshnessStamp::new(
                    "synthetic-file",
                    body.len() as u64,
                    Timestamp(1),
                    Digest::of_bytes(body.as_bytes()),
                )
                .unwrap(),
                body: body.to_owned(),
            }],
            false,
            Timestamp(2),
        )
        .unwrap();
    workflow
        .begin_manual_composition(workflow.revision(), Timestamp(3))
        .unwrap();
    workflow
        .set_language(workflow.revision(), WritingLanguage::EnUs, Timestamp(4))
        .unwrap();
    workflow
        .save_entry(0, "An invented ordinary day.".to_owned(), Timestamp(5))
        .unwrap();
    workflow
        .confirm_entry(workflow.revision(), 1, Timestamp(6))
        .unwrap();
}

fn make_completed(workflow: &mut Workflow, plan_id: &str, now: Timestamp) {
    let plan = OutputPlan::new(
        plan_id,
        workflow
            .entry()
            .confirmed
            .as_ref()
            .unwrap()
            .entry_digest
            .clone(),
        workflow.selection_digest().unwrap().clone(),
        vec![target("local")],
        now,
    )
    .unwrap();
    workflow
        .preview_output(workflow.revision(), plan.clone(), now)
        .unwrap();
    workflow
        .approve(workflow.revision(), &plan.plan_digest, now)
        .unwrap();
    workflow
        .begin_publication(workflow.revision(), now)
        .unwrap();
    workflow
        .finish_publication(
            workflow.revision(),
            vec![DestinationProgress {
                destination: "local".to_owned(),
                status: DestinationStatus::Verified,
            }],
            now,
        )
        .unwrap();
}

fn target(name: &str) -> DestinationPlan {
    DestinationPlan::new(
        if name == "local" {
            DestinationKind::Local
        } else {
            DestinationKind::Remote
        },
        name,
        format!("{name}-target"),
        Digest::of_bytes(format!("{name}-state").as_bytes()),
        Digest::of_bytes(format!("{name}-payload").as_bytes()),
        Digest::of_bytes(format!("{name}-inspected").as_bytes()),
    )
    .unwrap()
}

fn schema_version(path: impl AsRef<Path>) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}

#[derive(Default)]
struct Recorder(std::sync::Mutex<Vec<MigrationEvent>>);

impl Recorder {
    fn events(&self) -> Vec<MigrationEvent> {
        self.0.lock().unwrap().clone()
    }

    /// The events with durations zeroed, so sequences compare exactly.
    fn shapes(&self) -> Vec<MigrationEvent> {
        self.events()
            .into_iter()
            .map(|event| match event {
                MigrationEvent::StepApplied { from, to, .. } => MigrationEvent::StepApplied {
                    from,
                    to,
                    elapsed_ms: 0,
                },
                MigrationEvent::Finished { from, to, .. } => MigrationEvent::Finished {
                    from,
                    to,
                    elapsed_ms: 0,
                },
                MigrationEvent::Failed {
                    from, to, reason, ..
                } => MigrationEvent::Failed {
                    from,
                    to,
                    elapsed_ms: 0,
                    reason,
                },
                other => other,
            })
            .collect()
    }
}

impl MigrationObserver for Recorder {
    fn observe(&self, event: MigrationEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[test]
fn amendment_seed_round_trips_and_legacy_rows_load_without_one() {
    let scratch = Scratch::new("amendment-seed");
    let store = SqliteStore::open(scratch.db()).unwrap();
    let mut workflow = create_started(&store, "2026-10-28");
    make_confirmed(&mut workflow);
    make_completed(&mut workflow, "plan-v1", Timestamp(100));
    store.compare_and_swap(0, Some(0), &workflow).unwrap();
    let loaded = store.load(&day("2026-10-28")).unwrap().unwrap();
    assert!(loaded.amendment().is_none());

    let seed = AmendmentSeed {
        file_digest: Digest::of_bytes(b"seed"),
        receipt_version: 3,
        hand_edited: true,
    };
    let mut amended = loaded.clone();
    amended
        .seed_amendment(
            amended.revision(),
            "Body.".to_owned(),
            seed.clone(),
            Timestamp(150),
        )
        .unwrap();
    store
        .compare_and_swap(loaded.revision(), Some(loaded.entry().version), &amended)
        .unwrap();
    let reloaded = store.load(&day("2026-10-28")).unwrap().unwrap();
    assert_eq!(reloaded.amendment(), Some(&seed));
}

#[test]
fn migration_list_is_contiguous_from_one() {
    for (index, migration) in MIGRATIONS.iter().enumerate() {
        assert_eq!(migration.version, index as i64 + 1);
        assert!(!migration.sql.trim().is_empty());
    }
    assert_eq!(CURRENT_SCHEMA, MIGRATIONS.len() as i64);
}

#[test]
fn migration_events_report_each_step_then_the_current_state() {
    let scratch = Scratch::new("migration-events");
    let recorder = Recorder::default();
    SqliteStore::open_observed(scratch.db(), &recorder).unwrap();
    let mut expected = vec![MigrationEvent::Started {
        from: 0,
        to: CURRENT_SCHEMA,
    }];
    for version in 1..=CURRENT_SCHEMA {
        expected.push(MigrationEvent::StepApplied {
            from: version - 1,
            to: version,
            elapsed_ms: 0,
        });
    }
    expected.push(MigrationEvent::Finished {
        from: 0,
        to: CURRENT_SCHEMA,
        elapsed_ms: 0,
    });
    assert_eq!(recorder.shapes(), expected);

    let reopened = Recorder::default();
    SqliteStore::open_observed(scratch.db(), &reopened).unwrap();
    assert_eq!(
        reopened.events(),
        [MigrationEvent::Current {
            version: CURRENT_SCHEMA
        }]
    );
}

#[test]
fn partial_upgrade_runs_only_the_pending_steps() {
    let scratch = Scratch::new("migration-partial");
    let connection = Connection::open(scratch.db()).unwrap();
    connection.execute_batch(MIGRATION_1).unwrap();
    connection.execute_batch(MIGRATION_2).unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
    drop(connection);
    let recorder = Recorder::default();
    SqliteStore::open_observed(scratch.db(), &recorder).unwrap();
    assert_eq!(
        recorder.shapes().first(),
        Some(&MigrationEvent::Started {
            from: 2,
            to: CURRENT_SCHEMA
        })
    );
    assert_eq!(recorder.events().len(), 2 + (CURRENT_SCHEMA - 2) as usize);
}

#[test]
fn a_newer_schema_is_refused_untouched_and_reported() {
    let scratch = Scratch::new("migration-newer");
    let connection = Connection::open(scratch.db()).unwrap();
    connection
        .pragma_update(None, "user_version", CURRENT_SCHEMA + 1)
        .unwrap();
    drop(connection);
    let recorder = Recorder::default();
    assert_eq!(
        SqliteStore::open_observed(scratch.db(), &recorder).err(),
        Some(StoreError::UnsupportedSchema(CURRENT_SCHEMA + 1))
    );
    assert_eq!(
        recorder.shapes(),
        [MigrationEvent::Failed {
            from: CURRENT_SCHEMA + 1,
            to: CURRENT_SCHEMA,
            elapsed_ms: 0,
            reason: "unsupported-schema",
        }]
    );
    assert_eq!(schema_version(scratch.db()), CURRENT_SCHEMA + 1);
}

#[test]
fn a_failing_step_rolls_back_and_keeps_earlier_steps() {
    let scratch = Scratch::new("migration-rollback");
    let mut connection = Connection::open(scratch.db()).unwrap();
    let migrations = [
        Migration {
            version: 1,
            sql: "CREATE TABLE first (id INTEGER PRIMARY KEY);",
        },
        Migration {
            version: 2,
            sql: "CREATE TABLE half_done (id INTEGER); THIS IS NOT SQL;",
        },
    ];
    let recorder = Recorder::default();
    assert!(run_migrations(&mut connection, &migrations, &recorder).is_err());
    assert_eq!(user_version(&connection).unwrap(), 1);
    let half_done: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'half_done')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!half_done, "a failed step must leave no partial schema");
    assert_eq!(
        recorder.shapes().last(),
        Some(&MigrationEvent::Failed {
            from: 1,
            to: 2,
            elapsed_ms: 0,
            reason: "step-failed",
        })
    );
}

#[test]
fn a_step_another_process_already_applied_is_skipped_not_rerun() {
    let scratch = Scratch::new("migration-raced");
    let mut connection = Connection::open(scratch.db()).unwrap();
    let migration = Migration {
        version: 1,
        sql: "CREATE TABLE once (id INTEGER PRIMARY KEY);",
    };
    apply_step(&mut connection, &migration).unwrap();
    // A second runner that read version 0 before the first committed
    // reaches the same step; it must see the committed version and stop.
    apply_step(&mut connection, &migration).unwrap();
    assert_eq!(user_version(&connection).unwrap(), 1);
}

#[test]
fn a_gap_in_the_steps_is_refused_before_any_change() {
    let scratch = Scratch::new("migration-gap");
    let mut connection = Connection::open(scratch.db()).unwrap();
    let migrations = [Migration {
        version: 2,
        sql: "CREATE TABLE skipped (id INTEGER);",
    }];
    let recorder = Recorder::default();
    assert!(run_migrations(&mut connection, &migrations, &recorder).is_err());
    assert_eq!(user_version(&connection).unwrap(), 0);
    assert!(matches!(
        recorder.shapes().last(),
        Some(MigrationEvent::Failed {
            reason: "migration-gap",
            ..
        })
    ));
}

fn day(value: &str) -> DiaryDay {
    DiaryDay::parse(value).unwrap()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        loop {
            let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "nd-workflow-{label}-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("scratch directory should be created: {error}"),
            }
        }
    }

    fn db(&self) -> PathBuf {
        self.0.join("private.sqlite3")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("failed to remove test scratch directory: {error}");
        }
    }
}
