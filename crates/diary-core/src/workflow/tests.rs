use super::*;
use crate::{
    DateEvidence, DestinationPlan, FreshnessStamp, SourceAccessError, SourceAvailability,
    SourceCheckpointError, SourceReadOutcome, SourceReader, SourceRef, WritingLanguage,
    revalidate_workflow_sources,
};

#[test]
fn state_table_contains_every_architecture_state_and_journey_enters_each() {
    let states: BTreeSet<_> = LEGAL_TRANSITIONS
        .iter()
        .flat_map(|(from, to)| [*from, *to])
        .collect();
    assert_eq!(states.len(), 10);

    let mut workflow = discovered_workflow();
    let snapshot = snapshot("selected", "body");
    workflow
        .confirm_sources(workflow.revision(), vec![snapshot], false, time(2))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::SourcesConfirmed);
    workflow
        .begin_manual_composition(workflow.revision(), time(3))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::Composing);
    workflow
        .set_language(workflow.revision(), WritingLanguage::EnUs, time(4))
        .unwrap();
    workflow
        .save_entry(0, "Invented day.".to_owned(), time(5))
        .unwrap();
    workflow
        .confirm_entry(workflow.revision(), 1, time(6))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::EntryConfirmed);
    let plan = plan_for(&workflow, "plan-1");
    workflow
        .preview_output(workflow.revision(), plan.clone(), time(7))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::OutputPlanned);
    workflow
        .approve(workflow.revision(), &plan.plan_digest, time(8))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::Approved);
    workflow
        .begin_publication(workflow.revision(), time(9))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::Publishing);
    workflow
        .request_cancellation(workflow.revision(), time(9))
        .unwrap();
    assert_eq!(
        workflow.job_status(),
        Some(JobStatus::CancellationRequested)
    );
    workflow
        .finish_publication(
            workflow.revision(),
            vec![DestinationProgress {
                destination: "local".to_owned(),
                status: DestinationStatus::Verified,
            }],
            time(10),
        )
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::Completed);
    workflow
        .seed_amendment(
            workflow.revision(),
            "Edited local text".to_owned(),
            seed(),
            time(11),
        )
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::Composing);

    let mut partial = journey_to_publishing();
    partial
        .finish_publication(
            partial.revision(),
            vec![DestinationProgress {
                destination: "local".to_owned(),
                status: DestinationStatus::Blocked,
            }],
            time(10),
        )
        .unwrap();
    assert_eq!(partial.state(), WorkflowState::Partial);
}

#[test]
fn refused_commands_and_navigation_leave_state_unchanged() {
    let mut workflow = Workflow::new(day(), time(0), &Preferences::default());
    let before = workflow.clone();
    assert_eq!(
        workflow.confirm_sources(0, Vec::new(), false, time(1)),
        Err(WorkflowError::WrongState)
    );
    workflow.navigate_back();
    assert_eq!(workflow, before);
    assert_eq!(
        workflow.discover(9, Vec::new(), time(1)),
        Err(WorkflowError::StaleRevision {
            expected: 9,
            actual: 0
        })
    );
    assert_eq!(workflow, before);
}

#[test]
fn selection_is_independent_of_date_evidence_and_empty_path_disables_assistant_actions() {
    let mut workflow = Workflow::new(day(), time(0), &Preferences::default());
    let outside = SourceCandidate {
        source: source("outside"),
        source_note_name: "outside.md".to_owned(),
        date_evidence: DateEvidence {
            directory_day: Some(DiaryDay::parse("2026-10-20").unwrap()),
            header_day: Some(DiaryDay::parse("2026-10-30").unwrap()),
            warning: Some("dates disagree".to_owned()),
        },
        availability: SourceAvailability::Resident,
    };
    workflow.discover(0, vec![outside], time(1)).unwrap();
    workflow
        .confirm_sources(1, vec![snapshot("outside", "included")], false, time(2))
        .unwrap();
    assert!(workflow.assistant_actions_available());

    let mut empty = discovered_workflow();
    assert_eq!(empty.language(), None);
    assert_eq!(empty.style(), WritingStyle::Concise);
    assert_eq!(empty.assistant_selection(), None);
    assert_eq!(empty.composition_path(), None);
    assert_eq!(
        empty.confirm_sources(1, Vec::new(), false, time(2)),
        Err(WorkflowError::EmptySelectionNeedsConfirmation)
    );
    empty.confirm_sources(1, Vec::new(), true, time(2)).unwrap();
    assert!(!empty.assistant_actions_available());
    assert_eq!(
        empty.memory_extraction_request(),
        Err(WorkflowError::AssistantActionsDisabled)
    );
    empty.begin_manual_composition(2, time(3)).unwrap();
    empty.save_entry(0, "빈 날".to_owned(), time(5)).unwrap();
    assert_eq!(
        empty.confirm_entry(empty.revision(), 1, time(6)),
        Err(WorkflowError::LanguageRequired)
    );
    empty
        .set_language(empty.revision(), WritingLanguage::KoKr, time(7))
        .unwrap();
    empty.confirm_entry(4, 1, time(8)).unwrap();
}

#[test]
fn explicit_empty_selection_can_confirm_directly_from_started() {
    let mut workflow = Workflow::new(day(), time(0), &Preferences::default());

    workflow
        .confirm_sources(0, Vec::new(), true, time(1))
        .unwrap();

    assert_eq!(workflow.state(), WorkflowState::SourcesConfirmed);
    assert!(workflow.candidates().is_empty());
    assert!(workflow.snapshots().is_empty());
    assert!(workflow.explicit_empty_selection());
    assert!(workflow.selection_digest().is_some());
    assert!(!workflow.assistant_actions_available());
}

#[test]
fn discovery_rejects_unsafe_source_note_names() {
    for source_note_name in [
        "",
        "/private/note.md",
        "../note.md",
        "day/../note.md",
        "day//note.md",
        "day/note\n.md",
    ] {
        let mut workflow = Workflow::new(day(), time(0), &Preferences::default());
        let candidate = SourceCandidate {
            source: source("unsafe"),
            source_note_name: source_note_name.to_owned(),
            date_evidence: DateEvidence {
                directory_day: Some(day()),
                header_day: None,
                warning: None,
            },
            availability: SourceAvailability::Resident,
        };

        assert_eq!(
            workflow.discover(0, vec![candidate], time(1)),
            Err(WorkflowError::InvalidSourceNoteName),
            "unexpectedly accepted {source_note_name:?}"
        );
    }
}

#[test]
fn reselection_voids_downstream_but_preserves_manual_text() {
    let mut workflow = confirmed_workflow();
    workflow
        .confirm_sources(workflow.revision(), Vec::new(), true, time(20))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::SourcesConfirmed);
    assert_eq!(workflow.entry().text, "Invented day.");
    assert!(workflow.entry().confirmed.is_none());
    assert!(workflow.output_plan().is_none());
    assert_eq!(
        workflow.invalidation_reason(),
        Some(&InvalidationReason::SelectionChanged)
    );
    assert!(!workflow.assistant_actions_available());
}

#[test]
fn reflection_markers_never_reach_draft_or_paragraph_requests() {
    let mut workflow = composing_workflow();
    let backend_id = crate::AssistantBackendId::parse("fake-one").unwrap();
    workflow
        .set_assistant_selection(
            workflow.revision(),
            Some(AssistantSelection::new(
                backend_id.clone(),
                CompositionPath::Hosted,
            )),
            time(5),
        )
        .unwrap();
    let source_digest = workflow.selection_digest().unwrap().clone();
    let provenance = ArtifactProvenance {
        backend_id,
        composition_path: CompositionPath::Hosted,
    };
    let extraction = MemoryExtractionResult::new(vec![
        crate::MemoryCandidate::new("remembered", vec![1], 1).unwrap(),
    ])
    .unwrap();
    workflow
        .commit_memory_extraction(
            "memory-set-1".to_owned(),
            provenance.clone(),
            &source_digest,
            &extraction,
            time(6),
        )
        .unwrap();
    workflow
        .replace_reviewed_memory(
            workflow.revision(),
            "reviewed-1".to_owned(),
            "memory-set-1",
            "human reviewed".to_owned(),
            time(7),
        )
        .unwrap();

    let (_, binding) = workflow.draft_entry_request(false).unwrap();
    let draft =
        DraftEntryResult::new("A quiet walk.", vec![ReflectionRange { start: 2, end: 7 }]).unwrap();
    workflow
        .commit_draft_proposal("draft-1".to_owned(), provenance, binding, &draft, time(8))
        .unwrap();
    assert!(
        workflow.draft_proposals()[0]
            .text
            .contains(INFERRED_REFLECTION_OPEN)
    );

    let (request, _) = workflow.draft_entry_request(true).unwrap();
    assert_eq!(request.prior_draft(), Some("A quiet walk."));

    // The populated buffer is the marked text; a paragraph review over it
    // sends plain text but keeps buffer coordinates in its binding.
    let end = workflow.entry().text.len();
    assert!(workflow.entry().text.contains(INFERRED_REFLECTION_CLOSE));
    let range = ReflectionRange { start: 0, end };
    let (paragraph, binding) = workflow.paragraph_review_request(range).unwrap();
    assert_eq!(paragraph.paragraph(), "A quiet walk.");
    assert_eq!(binding.range, range);
    assert_eq!(
        workflow.paragraph_review_request(ReflectionRange { start: 3, end: 3 }),
        Err(WorkflowError::AssistantParagraphRangeInvalid)
    );
}

#[test]
fn assistant_artifacts_are_bound_guarded_and_paragraph_resolution_is_once_only() {
    let mut workflow = composing_workflow();
    let backend_id = crate::AssistantBackendId::parse("fake-one").unwrap();
    workflow
        .set_assistant_selection(
            workflow.revision(),
            Some(AssistantSelection::new(
                backend_id.clone(),
                CompositionPath::Hosted,
            )),
            time(5),
        )
        .unwrap();
    let source_digest = workflow.selection_digest().unwrap().clone();
    let extraction = MemoryExtractionResult::new(vec![
        crate::MemoryCandidate::new("remembered", vec![1], 1).unwrap(),
    ])
    .unwrap();
    let provenance = ArtifactProvenance {
        backend_id: backend_id.clone(),
        composition_path: CompositionPath::Hosted,
    };
    workflow
        .commit_memory_extraction(
            "memory-set-1".to_owned(),
            provenance.clone(),
            &source_digest,
            &extraction,
            time(6),
        )
        .unwrap();
    assert_eq!(
        workflow.memory_candidate_sets()[0].candidates[0].source_snapshot_digests,
        [workflow.snapshots()[0].stamp.content_digest.clone()]
    );
    workflow
        .replace_reviewed_memory(
            workflow.revision(),
            "reviewed-1".to_owned(),
            "memory-set-1",
            "human reviewed".to_owned(),
            time(7),
        )
        .unwrap();

    let (_, stale_binding) = workflow.draft_entry_request(false).unwrap();
    workflow
        .save_entry(0, "newer manual text".to_owned(), time(8))
        .unwrap();
    let draft = DraftEntryResult::new("late draft", vec![]).unwrap();
    assert!(
        !workflow
            .commit_draft_proposal(
                "draft-1".to_owned(),
                provenance.clone(),
                stale_binding,
                &draft,
                time(9),
            )
            .unwrap()
    );
    assert_eq!(workflow.entry().text, "newer manual text");
    assert!(workflow.draft_proposals()[0].stale);

    let (_, current_binding) = workflow.draft_entry_request(false).unwrap();
    let draft = DraftEntryResult::new("fresh paragraph", vec![]).unwrap();
    assert!(
        workflow
            .commit_draft_proposal(
                "draft-2".to_owned(),
                provenance.clone(),
                current_binding,
                &draft,
                time(10),
            )
            .unwrap()
    );
    assert_eq!(workflow.entry().text, "fresh paragraph");

    let range = ReflectionRange { start: 0, end: 5 };
    let (_, paragraph_binding) = workflow.paragraph_review_request(range).unwrap();
    let suggestion = ParagraphReviewResult::new("new", vec![], None).unwrap();
    workflow
        .commit_paragraph_suggestion(
            "suggestion-1".to_owned(),
            provenance,
            paragraph_binding,
            &suggestion,
            time(11),
        )
        .unwrap();
    workflow
        .resolve_paragraph_suggestion(
            workflow.revision(),
            "suggestion-1",
            SuggestionResolution::Accepted,
            time(12),
        )
        .unwrap();
    assert_eq!(workflow.entry().text, "new paragraph");
    assert_eq!(
        workflow.resolve_paragraph_suggestion(
            workflow.revision(),
            "suggestion-1",
            SuggestionResolution::Rejected,
            time(13),
        ),
        Err(WorkflowError::AssistantSuggestionAlreadyResolved)
    );
}

#[test]
fn structured_identities_and_plan_bindings_reject_ambiguity() {
    let left = SourceRef::new("a:b", "c").unwrap();
    let right = SourceRef::new("a", "b:c").unwrap();
    assert_ne!(left, right);
    assert_ne!(left.stable_key(), right.stable_key());

    let duplicate = OutputPlan::new(
        "duplicate-target",
        Digest::of_bytes(b"entry"),
        Digest::of_bytes(b"selection"),
        vec![
            target_kind("same", DestinationKind::Local),
            target_kind("same", DestinationKind::Remote),
        ],
        time(1),
    );
    assert!(duplicate.is_err());

    let workflow = confirmed_workflow();
    let mut corrupt = workflow.persisted();
    corrupt.selection_digest = Some(Digest::of_bytes(b"wrong selection"));
    assert_eq!(
        Workflow::restore(corrupt),
        Err(WorkflowError::InvalidPersistedState)
    );
}

#[test]
fn freshness_invalidation_preserves_manual_text_but_voids_downstream() {
    for outcome in [
        FreshnessOutcome::Changed {
            source: source("selected"),
        },
        FreshnessOutcome::Unverifiable {
            source: source("selected"),
        },
    ] {
        let mut workflow = confirmed_workflow();
        workflow
            .apply_freshness(
                workflow.revision(),
                FreshnessCheckpoint::BeforePreview,
                outcome,
                time(20),
            )
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::Discovered);
        assert_eq!(workflow.entry().text, "Invented day.");
        assert!(workflow.entry().confirmed.is_none());
        assert!(workflow.output_plan().is_none());
        assert!(workflow.approval().is_none());
    }
}

#[test]
fn every_freshness_checkpoint_accepts_fresh_without_mutation() {
    for checkpoint in [
        FreshnessCheckpoint::PostRead,
        FreshnessCheckpoint::BeforeExtraction,
        FreshnessCheckpoint::BeforeDrafting,
        FreshnessCheckpoint::BeforeEntryConfirmation,
        FreshnessCheckpoint::BeforePreview,
        FreshnessCheckpoint::BeforeFirstWrite,
    ] {
        let mut workflow = composing_workflow();
        let before = workflow.clone();
        workflow
            .apply_freshness(
                workflow.revision(),
                checkpoint,
                FreshnessOutcome::Fresh,
                time(20),
            )
            .unwrap();
        assert_eq!(workflow, before);
    }
}

#[test]
fn source_reader_checkpoint_invalidates_then_allows_reread_and_reconfirmation() {
    let mut workflow = confirmed_workflow();
    let reader = FakeSourceReader {
        outcome: Ok(FreshnessOutcome::Changed {
            source: source("selected"),
        }),
    };
    revalidate_workflow_sources(
        &mut workflow,
        &reader,
        FreshnessCheckpoint::BeforePreview,
        time(30),
    )
    .unwrap();
    assert_eq!(workflow.state(), WorkflowState::Discovered);
    assert_eq!(workflow.entry().text, "Invented day.");
    assert!(workflow.entry().confirmed.is_none());

    workflow
        .confirm_sources(
            workflow.revision(),
            vec![snapshot("selected", "fresh body")],
            false,
            time(31),
        )
        .unwrap();
    workflow
        .confirm_entry(workflow.revision(), workflow.entry().version, time(32))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::EntryConfirmed);
}

struct FakeSourceReader {
    outcome: Result<FreshnessOutcome, SourceAccessError>,
}

impl SourceReader for FakeSourceReader {
    fn read_selected(&self, _source: &SourceRef) -> Result<SourceReadOutcome, SourceAccessError> {
        unreachable!()
    }

    fn revalidate(
        &self,
        _snapshot: &SourceSnapshot,
    ) -> Result<FreshnessOutcome, SourceAccessError> {
        self.outcome.clone()
    }

    fn request_download(&self, _source: &SourceRef) -> Result<(), SourceAccessError> {
        unreachable!()
    }
}

#[test]
fn source_reader_access_failure_invalidates_before_returning_the_error() {
    let mut workflow = confirmed_workflow();
    let reader = FakeSourceReader {
        outcome: Err(SourceAccessError::TimedOut),
    };
    assert_eq!(
        revalidate_workflow_sources(
            &mut workflow,
            &reader,
            FreshnessCheckpoint::BeforePreview,
            time(30),
        ),
        Err(SourceCheckpointError::Access(SourceAccessError::TimedOut))
    );
    assert_eq!(workflow.state(), WorkflowState::Discovered);
    assert_eq!(workflow.entry().text, "Invented day.");
    assert!(workflow.entry().confirmed.is_none());
    assert!(matches!(
        workflow.invalidation_reason(),
        Some(InvalidationReason::FreshnessUnverifiable(failed_source))
            if failed_source == &source("selected")
    ));
}

#[test]
fn entry_versions_are_independent_and_confirmation_checks_both_versions() {
    let mut workflow = composing_workflow();
    let workflow_revision = workflow.revision();
    workflow.save_entry(0, "first".to_owned(), time(5)).unwrap();
    assert_eq!(workflow.revision(), workflow_revision);
    assert_eq!(
        workflow.save_entry(0, "racing".to_owned(), time(6)),
        Err(WorkflowError::StaleBuffer {
            expected: 0,
            actual: 1
        })
    );
    assert_eq!(
        workflow.confirm_entry(workflow_revision, 0, time(7)),
        Err(WorkflowError::StaleBuffer {
            expected: 0,
            actual: 1
        })
    );
    workflow
        .confirm_entry(workflow_revision, 1, time(8))
        .unwrap();
    let confirmed_revision = workflow.revision();
    workflow
        .save_entry(1, "second".to_owned(), time(9))
        .unwrap();
    assert_eq!(workflow.revision(), confirmed_revision + 1);
    assert_eq!(workflow.state(), WorkflowState::Composing);
    assert!(workflow.entry().confirmed.is_none());
}

#[test]
fn blank_and_inferred_reflection_text_cannot_be_confirmed() {
    let mut workflow = composing_workflow();
    workflow.save_entry(0, "  \n".to_owned(), time(5)).unwrap();
    assert_eq!(
        workflow.confirm_entry(workflow.revision(), 1, time(6)),
        Err(WorkflowError::BlankEntry)
    );
    workflow
        .save_entry(
            1,
            format!("Review {INFERRED_REFLECTION_OPEN}maybe"),
            time(7),
        )
        .unwrap();
    assert_eq!(
        workflow.confirm_entry(workflow.revision(), 2, time(8)),
        Err(WorkflowError::UnresolvedInferenceMarker)
    );
}

#[test]
fn language_and_style_changes_stale_assistant_material_without_losing_text() {
    let mut workflow = confirmed_workflow();
    workflow
        .set_style(workflow.revision(), WritingStyle::Reflective, time(20))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::Composing);
    assert_eq!(workflow.entry().text, "Invented day.");
    assert!(workflow.entry().confirmed.is_none());
    workflow
        .set_language(workflow.revision(), WritingLanguage::KoKr, time(21))
        .unwrap();
    assert_eq!(workflow.entry().text, "Invented day.");
    workflow
        .set_assistant_selection(
            workflow.revision(),
            Some(AssistantSelection::new(
                crate::AssistantBackendId::parse("hosted-default").unwrap(),
                CompositionPath::Hosted,
            )),
            time(22),
        )
        .unwrap();
    assert_eq!(
        workflow
            .assistant_selection()
            .map(|selection| selection.backend_id.as_str()),
        Some("hosted-default")
    );
    assert_eq!(workflow.composition_path(), Some(CompositionPath::Hosted));
    assert_eq!(workflow.entry().text, "Invented day.");
}

#[test]
fn plan_and_approval_are_bound_to_exact_inputs_and_retry_policy() {
    let mut workflow = confirmed_workflow();
    let wrong = OutputPlan::new(
        "wrong",
        Digest::of_bytes(b"different entry"),
        workflow.selection_digest().unwrap().clone(),
        vec![target("local")],
        time(20),
    )
    .unwrap();
    assert_eq!(
        workflow.preview_output(workflow.revision(), wrong, time(20)),
        Err(WorkflowError::PlanBindingMismatch)
    );

    let plan = plan_for(&workflow, "plan-1");
    workflow
        .preview_output(workflow.revision(), plan.clone(), time(21))
        .unwrap();
    let other_digest = Digest::of_bytes(b"other plan");
    assert_eq!(
        workflow.approve(workflow.revision(), &other_digest, time(22)),
        Err(WorkflowError::PlanBindingMismatch)
    );
    workflow
        .approve(workflow.revision(), &plan.plan_digest, time(23))
        .unwrap();
    let mut corrupt_approval = workflow.persisted();
    corrupt_approval.approval.as_mut().unwrap().destination_only = Some("local".to_owned());
    assert_eq!(
        Workflow::restore(corrupt_approval),
        Err(WorkflowError::InvalidPersistedState)
    );
    assert!(!workflow.pending_retry_reuses_approval("local", &plan.plan_digest));
    assert!(!workflow.pending_retry_reuses_approval("local", &other_digest));

    let replacement = plan_for(&workflow, "plan-2");
    let replacement_digest = replacement.plan_digest.clone();
    workflow
        .preview_output(workflow.revision(), replacement, time(24))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::OutputPlanned);
    assert!(workflow.approval().is_none());
    workflow
        .approve(workflow.revision(), &replacement_digest, time(25))
        .unwrap();
    workflow
        .save_entry(1, "Changed after approval.".to_owned(), time(26))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::Composing);
    assert!(workflow.output_plan().is_none());
    assert!(workflow.approval().is_none());
}

#[test]
fn local_first_and_blocked_remote_retry_require_exact_new_approval() {
    let mut workflow = confirmed_workflow();
    let plan = OutputPlan::new(
        "two-target-plan",
        workflow
            .entry()
            .confirmed
            .as_ref()
            .unwrap()
            .entry_digest
            .clone(),
        workflow.selection_digest().unwrap().clone(),
        vec![
            target_kind("remote", DestinationKind::Remote),
            target_kind("local", DestinationKind::Local),
        ],
        time(20),
    )
    .unwrap();
    workflow
        .preview_output(workflow.revision(), plan.clone(), time(21))
        .unwrap();
    workflow
        .approve(workflow.revision(), &plan.plan_digest, time(22))
        .unwrap();
    workflow
        .begin_publication(workflow.revision(), time(23))
        .unwrap();
    assert_eq!(
        workflow.finish_publication(
            workflow.revision(),
            vec![
                DestinationProgress {
                    destination: "local".to_owned(),
                    status: DestinationStatus::Pending,
                },
                DestinationProgress {
                    destination: "remote".to_owned(),
                    status: DestinationStatus::Verified,
                },
            ],
            time(24),
        ),
        Err(WorkflowError::LocalMustBeVerified)
    );
    workflow
        .finish_publication(
            workflow.revision(),
            vec![
                DestinationProgress {
                    destination: "local".to_owned(),
                    status: DestinationStatus::Verified,
                },
                DestinationProgress {
                    destination: "remote".to_owned(),
                    status: DestinationStatus::Blocked,
                },
            ],
            time(25),
        )
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::Completed);
    let retry_plan = workflow
        .preview_blocked_retry(
            workflow.revision(),
            "remote",
            target_kind("remote", DestinationKind::Remote),
            "remote-retry-plan",
            time(26),
        )
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::OutputPlanned);
    assert!(workflow.approval().is_none());
    assert_eq!(
        workflow.approve_blocked_retry(
            workflow.revision(),
            "remote",
            &Digest::of_bytes(b"changed plan"),
            time(27),
        ),
        Err(WorkflowError::PlanBindingMismatch)
    );
    assert_eq!(
        workflow.approve(workflow.revision(), &retry_plan.plan_digest, time(28),),
        Err(WorkflowError::RetryNeedsPreview)
    );
    workflow
        .approve_blocked_retry(
            workflow.revision(),
            "remote",
            &retry_plan.plan_digest,
            time(29),
        )
        .unwrap();
    assert_eq!(
        workflow.approval().unwrap().destination_only.as_deref(),
        Some("remote")
    );
}

#[test]
fn pending_remote_reuses_approval_only_after_local_verifies_and_digest_matches() {
    let (mut pending, plan) = two_target_publishing();
    pending
        .finish_publication(
            pending.revision(),
            vec![
                DestinationProgress {
                    destination: "local".to_owned(),
                    status: DestinationStatus::Verified,
                },
                DestinationProgress {
                    destination: "remote".to_owned(),
                    status: DestinationStatus::Pending,
                },
            ],
            time(30),
        )
        .unwrap();
    assert!(pending.pending_retry_reuses_approval("remote", &plan.plan_digest));
    assert!(
        !pending.pending_retry_reuses_approval(
            "remote",
            &Digest::of_bytes(b"changed destination state")
        )
    );
    assert_eq!(
        pending.begin_pending_retry(
            pending.revision(),
            "remote",
            &Digest::of_bytes(b"changed destination state"),
            time(31),
        ),
        Err(WorkflowError::RetryNeedsPreview)
    );
    pending
        .begin_pending_retry(pending.revision(), "remote", &plan.plan_digest, time(32))
        .unwrap();
    assert_eq!(pending.state(), WorkflowState::Publishing);
    assert_eq!(
        pending.approval().unwrap().destination_only.as_deref(),
        Some("remote")
    );
    assert_eq!(
        pending.finish_publication(
            pending.revision(),
            vec![
                DestinationProgress {
                    destination: "local".to_owned(),
                    status: DestinationStatus::Pending,
                },
                DestinationProgress {
                    destination: "remote".to_owned(),
                    status: DestinationStatus::Verified,
                },
            ],
            time(33),
        ),
        Err(WorkflowError::VerifiedDestinationRegressed)
    );
    pending
        .finish_publication(
            pending.revision(),
            vec![
                DestinationProgress {
                    destination: "local".to_owned(),
                    status: DestinationStatus::Verified,
                },
                DestinationProgress {
                    destination: "remote".to_owned(),
                    status: DestinationStatus::Verified,
                },
            ],
            time(34),
        )
        .unwrap();
    assert_eq!(pending.state(), WorkflowState::Completed);

    let (mut conflicted, conflicted_plan) = two_target_publishing();
    conflicted
        .finish_publication(
            conflicted.revision(),
            vec![
                DestinationProgress {
                    destination: "local".to_owned(),
                    status: DestinationStatus::Changed,
                },
                DestinationProgress {
                    destination: "remote".to_owned(),
                    status: DestinationStatus::Pending,
                },
            ],
            time(30),
        )
        .unwrap();
    assert!(!conflicted.pending_retry_reuses_approval("remote", &conflicted_plan.plan_digest));
}

fn progress_of(items: &[(&str, DestinationStatus)]) -> Vec<DestinationProgress> {
    items
        .iter()
        .map(|(destination, status)| DestinationProgress {
            destination: (*destination).to_owned(),
            status: *status,
        })
        .collect()
}

fn mirror_plan(workflow: &Workflow) -> OutputPlan {
    OutputPlan::new(
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
            target_kind("local", DestinationKind::Local),
            target_kind("icloud", DestinationKind::Mirror),
            target_kind("notion", DestinationKind::Remote),
        ],
        time(20),
    )
    .unwrap()
}

fn mirror_publishing(
    local: DestinationStatus,
    icloud: DestinationStatus,
    notion: DestinationStatus,
) -> Workflow {
    let mut workflow = confirmed_workflow();
    let plan = mirror_plan(&workflow);
    workflow
        .preview_output(workflow.revision(), plan.clone(), time(21))
        .unwrap();
    workflow
        .approve(workflow.revision(), &plan.plan_digest, time(22))
        .unwrap();
    workflow
        .begin_publication(workflow.revision(), time(23))
        .unwrap();
    workflow
        .finish_publication(
            workflow.revision(),
            progress_of(&[("local", local), ("icloud", icloud), ("notion", notion)]),
            time(24),
        )
        .unwrap();
    workflow
}

#[test]
fn destination_kind_mirror_round_trips() {
    assert_eq!(DestinationKind::Mirror.as_str(), "mirror");
    assert_eq!(
        DestinationKind::parse(DestinationKind::Mirror.as_str()),
        Ok(DestinationKind::Mirror)
    );
}

#[test]
fn mirror_blocked_does_not_block_notion_approval() {
    let mut workflow = confirmed_workflow();
    let plan = mirror_plan(&workflow);
    workflow
        .preview_output_with_progress(
            workflow.revision(),
            plan.clone(),
            progress_of(&[
                ("local", DestinationStatus::Pending),
                ("icloud", DestinationStatus::Blocked),
                ("notion", DestinationStatus::Blocked),
            ]),
            time(21),
        )
        .unwrap();
    workflow
        .approve(workflow.revision(), &plan.plan_digest, time(22))
        .unwrap();
    workflow
        .begin_publication(workflow.revision(), time(23))
        .unwrap();
    workflow
        .finish_publication(
            workflow.revision(),
            progress_of(&[
                ("local", DestinationStatus::Verified),
                ("icloud", DestinationStatus::Blocked),
                ("notion", DestinationStatus::Blocked),
            ]),
            time(24),
        )
        .unwrap();
    let replanned = workflow
        .preview_blocked_retry(
            workflow.revision(),
            "notion",
            target_kind("notion", DestinationKind::Remote),
            "mirror-retry",
            time(25),
        )
        .unwrap();
    workflow
        .approve_blocked_retry(
            workflow.revision(),
            "notion",
            &replanned.plan_digest,
            time(26),
        )
        .unwrap();
}

#[test]
fn mirror_conflict_preview_still_allows_approval() {
    let mut workflow = confirmed_workflow();
    let plan = mirror_plan(&workflow);
    workflow
        .preview_output_with_progress(
            workflow.revision(),
            plan.clone(),
            progress_of(&[
                ("local", DestinationStatus::Pending),
                ("icloud", DestinationStatus::Changed),
                ("notion", DestinationStatus::Blocked),
            ]),
            time(21),
        )
        .unwrap();
    workflow
        .approve(workflow.revision(), &plan.plan_digest, time(22))
        .unwrap();
}

#[test]
fn mirror_conflict_leaves_local_verified_and_day_completed() {
    let workflow = mirror_publishing(
        DestinationStatus::Verified,
        DestinationStatus::Changed,
        DestinationStatus::Verified,
    );
    assert_eq!(workflow.state(), WorkflowState::Completed);
    assert!(
        workflow.destination_progress().iter().any(|item| {
            item.destination == "local" && item.status == DestinationStatus::Verified
        })
    );
}

#[test]
fn finish_mirror_retry_completes_when_all_verified() {
    let mut workflow = mirror_publishing(
        DestinationStatus::Verified,
        DestinationStatus::Blocked,
        DestinationStatus::Verified,
    );
    workflow
        .finish_mirror_retry(workflow.revision(), DestinationStatus::Verified, time(30))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::Completed);
    assert_eq!(workflow.all_verified_at(), Some(time(24)));

    let mut still_partial = mirror_publishing(
        DestinationStatus::Verified,
        DestinationStatus::Blocked,
        DestinationStatus::Blocked,
    );
    still_partial
        .finish_mirror_retry(
            still_partial.revision(),
            DestinationStatus::Verified,
            time(30),
        )
        .unwrap();
    assert_eq!(still_partial.state(), WorkflowState::Completed);
}

#[test]
fn finish_mirror_retry_rejects_non_partial() {
    let mut workflow = journey_to_publishing();
    assert_eq!(
        workflow.finish_mirror_retry(workflow.revision(), DestinationStatus::Verified, time(30)),
        Err(WorkflowError::WrongState)
    );
    let mut no_mirror = mirror_publishing(
        DestinationStatus::Verified,
        DestinationStatus::Blocked,
        DestinationStatus::Blocked,
    );
    no_mirror
        .destination_progress
        .retain(|item| item.destination != "icloud");
    assert_eq!(
        no_mirror.finish_mirror_retry(no_mirror.revision(), DestinationStatus::Verified, time(30)),
        Err(WorkflowError::UnknownDestination)
    );
}

#[test]
fn finish_mirror_retry_cannot_regress_verified() {
    let mut workflow = mirror_publishing(
        DestinationStatus::Verified,
        DestinationStatus::Verified,
        DestinationStatus::Blocked,
    );
    assert_eq!(
        workflow.finish_mirror_retry(workflow.revision(), DestinationStatus::Blocked, time(30)),
        Err(WorkflowError::VerifiedDestinationRegressed)
    );
}

fn seed() -> AmendmentSeed {
    AmendmentSeed {
        file_digest: Digest::of_bytes(b"seed file"),
        receipt_version: 1,
        hand_edited: false,
    }
}

fn completed_workflow() -> Workflow {
    let mut workflow = journey_to_publishing();
    workflow
        .finish_publication(
            workflow.revision(),
            vec![DestinationProgress {
                destination: "local".to_owned(),
                status: DestinationStatus::Verified,
            }],
            time(10),
        )
        .unwrap();
    workflow
}

#[test]
fn amendment_keeps_original_sources_binds_seed_and_restarts_retention_on_completion() {
    let mut workflow = completed_workflow();
    let selection = workflow.selection_digest().cloned();
    let snapshots = workflow.snapshots().to_vec();
    workflow
        .seed_amendment(workflow.revision(), "Amended.".to_owned(), seed(), time(11))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::Composing);
    assert_eq!(workflow.amendment(), Some(&seed()));
    assert_eq!(workflow.selection_digest().cloned(), selection);
    assert_eq!(workflow.snapshots(), snapshots.as_slice());
    assert!(workflow.output_plan().is_none());

    // The amendment survives a persistence round trip.
    let restored = Workflow::restore(workflow.persisted()).unwrap();
    assert_eq!(restored.amendment(), Some(&seed()));

    workflow
        .confirm_entry(workflow.revision(), workflow.entry().version, time(12))
        .unwrap();
    let plan = plan_for(&workflow, "amend-plan");
    workflow
        .preview_output(workflow.revision(), plan.clone(), time(13))
        .unwrap();
    workflow
        .approve(workflow.revision(), &plan.plan_digest, time(14))
        .unwrap();
    workflow
        .begin_publication(workflow.revision(), time(15))
        .unwrap();
    workflow
        .finish_publication(
            workflow.revision(),
            vec![DestinationProgress {
                destination: "local".to_owned(),
                status: DestinationStatus::Verified,
            }],
            time(16),
        )
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::Completed);
    assert!(workflow.amendment().is_none());
    assert_eq!(workflow.persisted().all_verified_at, Some(time(16)));
}

#[test]
fn amendment_needs_a_completed_day_and_abandon_returns_to_completed() {
    let mut composing = composing_workflow();
    assert_eq!(
        composing.seed_amendment(composing.revision(), "x".into(), seed(), time(11)),
        Err(WorkflowError::WrongState)
    );
    let mut workflow = completed_workflow();
    assert_eq!(
        workflow.abandon_amendment(workflow.revision(), time(11)),
        Err(WorkflowError::WrongState),
        "nothing to abandon without an amendment"
    );
    workflow
        .seed_amendment(workflow.revision(), "Amended.".to_owned(), seed(), time(11))
        .unwrap();
    let stale = workflow.revision() - 1;
    assert!(workflow.abandon_amendment(stale, time(12)).is_err());
    workflow
        .abandon_amendment(workflow.revision(), time(12))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::Completed);
    assert!(workflow.amendment().is_none());
    assert!(workflow.entry().text.is_empty());
    Workflow::restore(workflow.persisted()).unwrap();
    // The day can be amended again after abandoning.
    workflow
        .seed_amendment(workflow.revision(), "Again.".to_owned(), seed(), time(13))
        .unwrap();
}

#[test]
fn rebinding_keeps_text_and_requires_reconfirmation() {
    let mut workflow = completed_workflow();
    workflow
        .seed_amendment(workflow.revision(), "Mine.".to_owned(), seed(), time(11))
        .unwrap();
    workflow
        .confirm_entry(workflow.revision(), workflow.entry().version, time(12))
        .unwrap();
    let newer = AmendmentSeed {
        file_digest: Digest::of_bytes(b"newer file"),
        receipt_version: 1,
        hand_edited: true,
    };
    workflow
        .rebind_amendment(workflow.revision(), newer.clone(), time(13))
        .unwrap();
    assert_eq!(workflow.state(), WorkflowState::Composing);
    assert_eq!(workflow.amendment(), Some(&newer));
    assert_eq!(workflow.entry().text, "Mine.");
    assert!(workflow.entry().confirmed.is_none());
    let mut completed = completed_workflow();
    assert!(
        completed
            .rebind_amendment(completed.revision(), newer, time(14))
            .is_err()
    );
}

fn two_target_publishing() -> (Workflow, OutputPlan) {
    let mut workflow = confirmed_workflow();
    let plan = OutputPlan::new(
        "two-target-retry",
        workflow
            .entry()
            .confirmed
            .as_ref()
            .unwrap()
            .entry_digest
            .clone(),
        workflow.selection_digest().unwrap().clone(),
        vec![
            target_kind("local", DestinationKind::Local),
            target_kind("remote", DestinationKind::Remote),
        ],
        time(20),
    )
    .unwrap();
    workflow
        .preview_output(workflow.revision(), plan.clone(), time(21))
        .unwrap();
    workflow
        .approve(workflow.revision(), &plan.plan_digest, time(22))
        .unwrap();
    workflow
        .begin_publication(workflow.revision(), time(23))
        .unwrap();
    (workflow, plan)
}

fn journey_to_publishing() -> Workflow {
    let mut workflow = confirmed_workflow();
    let plan = plan_for(&workflow, "plan-1");
    workflow
        .preview_output(workflow.revision(), plan.clone(), time(7))
        .unwrap();
    workflow
        .approve(workflow.revision(), &plan.plan_digest, time(8))
        .unwrap();
    workflow
        .begin_publication(workflow.revision(), time(9))
        .unwrap();
    workflow
}

fn confirmed_workflow() -> Workflow {
    let mut workflow = composing_workflow();
    workflow
        .save_entry(0, "Invented day.".to_owned(), time(5))
        .unwrap();
    workflow
        .confirm_entry(workflow.revision(), 1, time(6))
        .unwrap();
    workflow
}

fn composing_workflow() -> Workflow {
    let mut workflow = discovered_workflow();
    workflow
        .confirm_sources(
            1,
            vec![snapshot("selected", "invented note")],
            false,
            time(2),
        )
        .unwrap();
    workflow.begin_manual_composition(2, time(3)).unwrap();
    workflow
        .set_language(3, WritingLanguage::EnUs, time(4))
        .unwrap();
    workflow
}

fn discovered_workflow() -> Workflow {
    let mut workflow = Workflow::new(day(), time(0), &Preferences::default());
    workflow
        .discover(
            0,
            vec![SourceCandidate {
                source: source("selected"),
                source_note_name: "selected.md".to_owned(),
                date_evidence: DateEvidence {
                    directory_day: Some(day()),
                    header_day: Some(day()),
                    warning: None,
                },
                availability: SourceAvailability::Resident,
            }],
            time(1),
        )
        .unwrap();
    workflow
}

#[test]
fn source_refinement_is_digest_bound_and_reject_leaves_source_state_unchanged() {
    let mut workflow = discovered_workflow();
    let source = source("selected");
    let digest = Digest::of_bytes(b"original");
    workflow
        .record_source_refine_suggestion(
            &source,
            "original".to_owned(),
            "corrected".to_owned(),
            digest.clone(),
            ArtifactProvenance {
                backend_id: crate::AssistantBackendId::parse("codex.test").unwrap(),
                composition_path: CompositionPath::Hosted,
            },
            time(2),
        )
        .unwrap();
    assert_eq!(
        workflow
            .take_source_refine_suggestion(workflow.revision(), &source, &digest, false, time(3),)
            .unwrap(),
        None
    );
    assert!(workflow.source_refine_suggestion(&source).is_none());
    assert_eq!(workflow.candidates().len(), 1);
}

#[test]
fn deleting_a_confirmed_source_removes_candidate_and_invalidates_selection() {
    let mut workflow = composing_workflow();
    let source = source("selected");
    workflow
        .source_deleted(workflow.revision(), &source, time(5))
        .unwrap();
    assert!(workflow.candidates().is_empty());
    assert!(workflow.snapshots().is_empty());
    assert_eq!(workflow.state(), WorkflowState::Discovered);
}

fn plan_for(workflow: &Workflow, id: &str) -> OutputPlan {
    OutputPlan::new(
        id,
        workflow
            .entry()
            .confirmed
            .as_ref()
            .unwrap()
            .entry_digest
            .clone(),
        workflow.selection_digest().unwrap().clone(),
        vec![target("local")],
        time(7),
    )
    .unwrap()
}

fn target(name: &str) -> DestinationPlan {
    target_kind(name, DestinationKind::Local)
}

fn target_kind(name: &str, kind: DestinationKind) -> DestinationPlan {
    DestinationPlan::new(
        kind,
        name,
        format!("{name}-target"),
        Digest::of_bytes(format!("{name}-state").as_bytes()),
        Digest::of_bytes(format!("{name}-payload").as_bytes()),
        Digest::of_bytes(format!("{name}-inspected").as_bytes()),
    )
    .unwrap()
}

fn snapshot(id: &str, body: &str) -> SourceSnapshot {
    SourceSnapshot {
        source: source(id),
        stamp: FreshnessStamp::new(
            format!("file-{id}"),
            body.len() as u64,
            time(1),
            Digest::of_bytes(body.as_bytes()),
        )
        .unwrap(),
        body: body.to_owned(),
    }
}

fn source(id: &str) -> SourceRef {
    SourceRef::new("fake", id).unwrap()
}

fn day() -> DiaryDay {
    DiaryDay::parse("2026-10-28").unwrap()
}

const fn time(value: i64) -> Timestamp {
    Timestamp(value)
}
