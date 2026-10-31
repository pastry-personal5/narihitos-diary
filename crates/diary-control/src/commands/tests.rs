use std::collections::BTreeSet;

use super::*;

#[test]
fn every_workflow_error_has_its_own_diagnostic_token() {
    let errors = [
        WorkflowError::StaleRevision {
            expected: 1,
            actual: 2,
        },
        WorkflowError::StaleBuffer {
            expected: 1,
            actual: 2,
        },
        WorkflowError::WrongState,
        WorkflowError::EmptySelectionNeedsConfirmation,
        WorkflowError::UnknownSource,
        WorkflowError::DuplicateSource,
        WorkflowError::InvalidSourceNoteName,
        WorkflowError::NeedsDownload,
        WorkflowError::PermissionDeniedSource,
        WorkflowError::UnreadableSource,
        WorkflowError::AssistantActionsDisabled,
        WorkflowError::AssistantRequestInvalid,
        WorkflowError::AssistantDraftRequestInvalid,
        WorkflowError::AssistantDraftResultInvalid,
        WorkflowError::AssistantParagraphRangeInvalid,
        WorkflowError::AssistantParagraphRequestInvalid,
        WorkflowError::AssistantParagraphResultInvalid,
        WorkflowError::AssistantBackendRequired,
        WorkflowError::ReviewedMemoryRequired,
        WorkflowError::AssistantArtifactNotFound,
        WorkflowError::AssistantBindingStale,
        WorkflowError::AssistantSuggestionAlreadyResolved,
        WorkflowError::LanguageRequired,
        WorkflowError::BlankEntry,
        WorkflowError::UnresolvedInferenceMarker,
        WorkflowError::EntryNotConfirmed,
        WorkflowError::MissingPlan,
        WorkflowError::PlanBindingMismatch,
        WorkflowError::UnknownDestination,
        WorkflowError::LocalMustBeVerified,
        WorkflowError::RetryNeedsPreview,
        WorkflowError::MissingApproval,
        WorkflowError::VerifiedDestinationRegressed,
        WorkflowError::DestinationOutsideExecutionScope,
        WorkflowError::InvalidPersistedState,
    ];
    let tokens: BTreeSet<_> = errors.iter().map(workflow_diagnostic_code).collect();
    assert_eq!(tokens.len(), errors.len());
    assert!(!tokens.contains("workflow-conflict"));
    assert!(tokens.iter().all(|token| {
        token.len() <= 64
            && token
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
    }));
}

#[test]
fn wire_code_is_unchanged_while_the_log_code_names_the_check() {
    let error = CommandError::Workflow(WorkflowError::ReviewedMemoryRequired);
    assert_eq!(error.code(), "workflow-conflict");
    assert_eq!(error.diagnostic_code(), "reviewed-memory-required");
    assert_eq!(
        CommandError::NotFound.diagnostic_code(),
        CommandError::NotFound.code()
    );
}

#[test]
fn notion_refusals_keep_their_wire_codes_but_name_the_failed_check() {
    let changed = CommandError::NotionBindingChanged("notion-state-changed");
    assert_eq!(changed.code(), "precondition-failed");
    assert_eq!(changed.diagnostic_code(), "notion-state-changed");
    let unavailable = notion_preflight_error("notion-permission-denied");
    assert_eq!(unavailable.code(), "notion-unavailable");
    assert_eq!(unavailable.diagnostic_code(), "notion-permission-denied");
    // Free text from a connector never reaches the log.
    assert_eq!(
        notion_preflight_error("page 1234 exploded").diagnostic_code(),
        "notion-unavailable"
    );
}

#[test]
fn notion_binding_refusal_names_the_first_departure() {
    let digest = |text: &str| Digest::of_bytes(text.as_bytes());
    let target = DestinationPlan::new(
        DestinationKind::Remote,
        "notion",
        "root",
        digest("payload"),
        digest("payload"),
        digest("state"),
    )
    .unwrap();
    let preflight =
        |status, identity: &str, payload: &str, state: &str| diary_core::NotionPreflight {
            target_identity: identity.to_owned(),
            root_label: String::new(),
            year_title: String::new(),
            month_title: String::new(),
            year_exists: true,
            month_exists: true,
            placement: "end".to_owned(),
            renderer_version: 1,
            api_version: String::new(),
            payload_digest: digest(payload),
            observed_state_digest: digest(state),
            blocks: Vec::new(),
            status,
            reason: None,
        };
    let ready = NotionPreflightStatus::Ready;
    assert_eq!(
        notion_binding_refusal(&preflight(ready, "root", "payload", "state"), &target, true),
        None
    );
    assert_eq!(
        notion_binding_refusal(
            &preflight(ready, "other", "payload", "state"),
            &target,
            true
        ),
        Some("notion-target-changed")
    );
    assert_eq!(
        notion_binding_refusal(
            &preflight(ready, "other", "payload", "state"),
            &target,
            false
        ),
        None
    );
    assert_eq!(
        notion_binding_refusal(&preflight(ready, "root", "edited", "state"), &target, true),
        Some("notion-payload-changed")
    );
    assert_eq!(
        notion_binding_refusal(&preflight(ready, "root", "payload", "moved"), &target, true),
        Some("notion-state-changed")
    );
    assert_eq!(
        notion_binding_refusal(
            &preflight(NotionPreflightStatus::Existing, "root", "payload", "state"),
            &target,
            true
        ),
        Some("notion-preflight-existing")
    );
    assert_eq!(
        notion_binding_refusal(
            &preflight(NotionPreflightStatus::Blocked, "root", "payload", "state"),
            &target,
            true
        ),
        Some("notion-preflight-blocked")
    );
}

#[test]
fn store_failures_get_distinct_diagnostic_tokens_but_one_wire_code() {
    let errors = [
        (StoreError::DuplicateDay, "state-duplicate-day"),
        (StoreError::UnsupportedSchema(9), "state-unsupported-schema"),
        (StoreError::Corrupt(String::new()), "state-corrupt"),
        (
            StoreError::InvalidRecord(String::new()),
            "state-invalid-record",
        ),
        (StoreError::Unavailable(String::new()), "state-unavailable"),
    ];
    for (error, token) in errors {
        let error = CommandError::Store(error);
        assert_eq!(error.code(), "state-unavailable");
        assert_eq!(error.diagnostic_code(), token);
    }
}
