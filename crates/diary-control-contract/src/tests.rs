use super::*;

#[test]
fn every_job_status_is_exactly_one_of_active_or_terminal() {
    use job_status::*;
    for status in [
        QUEUED,
        RUNNING,
        CANCELLATION_REQUESTED,
        COMPLETED,
        FAILED,
        CANCELLED,
    ] {
        assert_ne!(is_active(status), is_terminal(status), "{status}");
    }
    assert!(!is_terminal("timed-out"));
    assert!(!is_active("timed-out"));
}

#[test]
fn ids_and_strong_etags_have_frozen_grammar() {
    assert_eq!(workflow_id("2026-10-29").as_deref(), Some("wf_20261029"));
    assert_eq!(workflow_day("wf_20261029").as_deref(), Some("2026-10-29"));
    assert_eq!(workflow_etag(7), "\"wf-7\"");
    assert_eq!(buffer_etag(8), "\"buf-8\"");
    assert_eq!(parse_workflow_etag("\"wf-7\""), Some(7));
    assert_eq!(parse_buffer_etag("W/\"buf-8\""), None);
}

#[test]
fn request_dtos_reject_unknown_fields() {
    let error =
        serde_json::from_str::<SaveEntryRequest>(r#"{"text":"synthetic","unexpected":true}"#)
            .unwrap_err();
    assert!(error.to_string().contains("unknown field"));
    let settings: WritingSettingsRequest = serde_json::from_str(
        r#"{"language":"en-US","style":"concise","assistant_backend_id":null}"#,
    )
    .unwrap();
    assert_eq!(settings.assistant_backend_id, None);
    let legacy = serde_json::from_str::<WritingSettingsRequest>(
        r#"{"language":"en-US","style":"concise","composition_path":"manual"}"#,
    )
    .unwrap_err();
    assert!(legacy.to_string().contains("unknown field"));
    let legacy = serde_json::from_str::<PreferencesDto>(
        r#"{"last_language":"en-US","last_composition_path":"manual"}"#,
    )
    .unwrap_err();
    assert!(legacy.to_string().contains("unknown field"));
}

#[test]
fn assistant_backend_descriptor_has_collection_shaped_action_contract() {
    let descriptor = AssistantBackendDto {
        id: "synthetic-hosted".to_owned(),
        display_label: "Synthetic hosted".to_owned(),
        composition_path: "hosted".to_owned(),
        available: true,
        reason_code: None,
        supported_languages: vec!["en-US".to_owned(), "ko-KR".to_owned()],
        supported_actions: vec!["extract-memories".to_owned(), "draft-entry".to_owned()],
    };
    assert_eq!(
        serde_json::to_value(descriptor).unwrap(),
        serde_json::json!({
            "id": "synthetic-hosted",
            "display_label": "Synthetic hosted",
            "composition_path": "hosted",
            "available": true,
            "reason_code": null,
            "supported_languages": ["en-US", "ko-KR"],
            "supported_actions": ["extract-memories", "draft-entry"]
        })
    );
}

#[test]
fn workflow_dto_rejects_partial_or_malformed_assistant_selection() {
    let base = serde_json::json!({
        "id": "wf_20261029",
        "diary_day": "2026-10-29",
        "state": "composing",
        "revision": 2,
        "buffer_version": 0,
        "language": "en-US",
        "style": "concise",
        "assistant_backend_id": null,
        "composition_path": null,
        "selection_digest": null,
        "explicit_empty_selection": true,
        "invalidation_code": null,
        "entry_text": "",
        "entry_confirmed": false,
        "candidates": []
    });
    assert!(serde_json::from_value::<WorkflowDto>(base.clone()).is_ok());

    for (backend_id, path) in [
        (serde_json::json!("hosted-default"), serde_json::Value::Null),
        (serde_json::Value::Null, serde_json::json!("hosted")),
        (
            serde_json::json!("Hosted Default"),
            serde_json::json!("hosted"),
        ),
        (
            serde_json::json!("hosted-default"),
            serde_json::json!("manual"),
        ),
    ] {
        let mut invalid = base.clone();
        invalid["assistant_backend_id"] = backend_id;
        invalid["composition_path"] = path;
        assert!(serde_json::from_value::<WorkflowDto>(invalid).is_err());
    }

    let mut valid = base;
    valid["assistant_backend_id"] = serde_json::json!("hosted-default");
    valid["composition_path"] = serde_json::json!("hosted");
    assert!(serde_json::from_value::<WorkflowDto>(valid).is_ok());
}

#[test]
fn route_table_contains_local_publication_and_remotes_and_excludes_future_surfaces() {
    assert!(ROUTES.iter().all(|route| !route.path.contains("mcp")));
    for suffix in [
        "/amendments",
        "/amendments/previews",
        "/amendments/approvals",
        "/amendments/publications",
        "/amendments/rereads",
        "/amendments/abandonments",
    ] {
        assert!(
            ROUTES.iter().any(|route| route.path.ends_with(suffix)),
            "{suffix}"
        );
    }
    assert_eq!(
        CONTRACT_VERSION, 9,
        "the remote publication stages bumped the contract"
    );
    assert!(
        ROUTES
            .iter()
            .any(|route| route.path.ends_with("/local-publications"))
    );
    // Remotes are one family; the Notion-only and combined readiness routes are gone.
    for (method, suffix) in [
        ("GET", "/remotes"),
        ("POST", "/remotes/{name}/readiness"),
        ("POST", "/remotes/{name}/previews"),
        ("POST", "/remotes/{name}/pushes"),
    ] {
        assert!(
            ROUTES
                .iter()
                .any(|route| route.method == method && route.path.ends_with(suffix)),
            "{method} {suffix}"
        );
    }
    for gone in [
        "/notion-previews",
        "/notion-approvals",
        "/notion-publications",
    ] {
        assert!(
            !ROUTES.iter().any(|route| route.path.ends_with(gone)),
            "{gone}"
        );
    }
    assert!(
        !ROUTES
            .iter()
            .any(|route| route.path == "/v1/workflows/{id}/readiness")
    );
    for path in [
        "/v1/assistant-backends",
        "/v1/workflows/{id}/memory-extractions",
        "/v1/workflows/{id}/draft-result",
        "/v1/workflows/{id}/paragraph-suggestions/{suggestion_id}/resolution",
    ] {
        assert!(ROUTES.iter().any(|route| route.path == path));
    }
    assert_eq!(
        ROUTES.iter().filter(|route| !route.authenticated).count(),
        1
    );
}

#[test]
fn m8_request_dtos_reject_unknown_fields() {
    assert!(serde_json::from_str::<StartMemoryExtractionRequest>("{}").is_ok());
    assert!(
        serde_json::from_str::<StartMemoryExtractionRequest>(r#"{"unexpected":true}"#).is_err()
    );
    let request: ResolveParagraphSuggestionRequest = serde_json::from_str(
            r#"{"resolution":"accept","buffer_etag":"\"buf-3\"","range_start":0,"range_end":4,"selected_text_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
        )
        .unwrap();
    assert_eq!(request.resolution, "accept");
}
