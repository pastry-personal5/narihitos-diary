use std::cell::RefCell;

use diary_control_contract::{CapabilitiesDto, PreferencesDto, SetupCheckDto};

use super::{
    ApiClient, Attachment, ClientOutcome, RawRequest, RawResponse, Transport, TransportError,
};

struct SeenRequest {
    method: &'static str,
    path: String,
    authenticated: bool,
    if_match: Option<String>,
    body: Option<Vec<u8>>,
    preparation_handle: Option<String>,
}

struct FakeTransport {
    responses: RefCell<Vec<(&'static str, &'static str, RawResponse)>>,
    seen: RefCell<Vec<SeenRequest>>,
}

impl FakeTransport {
    fn new() -> Self {
        Self {
            responses: RefCell::new(Vec::new()),
            seen: RefCell::new(Vec::new()),
        }
    }

    fn push(&self, method: &'static str, path: &'static str, response: RawResponse) {
        self.responses.borrow_mut().push((method, path, response));
    }
}

impl Transport for FakeTransport {
    fn execute(
        &self,
        _attachment: &Attachment,
        request: &RawRequest,
    ) -> Result<RawResponse, TransportError> {
        self.seen.borrow_mut().push(SeenRequest {
            method: request.method,
            path: request.path.clone(),
            authenticated: request.authenticated,
            if_match: request.if_match.clone(),
            body: request.body.clone(),
            preparation_handle: request.preparation_handle.clone(),
        });
        let mut responses = self.responses.borrow_mut();
        let index = responses
            .iter()
            .position(|(method, path, _)| *method == request.method && *path == request.path)
            .expect("unexpected request in fake transport");
        Ok(responses.remove(index).2)
    }
}

fn attachment() -> Attachment {
    Attachment::new(
        "instance_synthetic".to_owned(),
        "127.0.0.1".to_owned(),
        4000,
        "token_synthetic".to_owned(),
        2,
    )
}

#[test]
fn preferences_decodes_a_successful_response() {
    let transport = FakeTransport::new();
    transport.push(
        "GET",
        "/v1/preferences",
        RawResponse {
            status: 200,
            body: serde_json::to_vec(&PreferencesDto {
                last_language: Some("en-US".to_owned()),
                last_assistant_backend_id: None,
            })
            .unwrap(),
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    match client.preferences(&attachment()) {
        ClientOutcome::Ok(prefs) => assert_eq!(prefs.last_language.as_deref(), Some("en-US")),
        _ => panic!("expected Ok outcome"),
    }
}

#[test]
fn preferences_request_is_authenticated_with_no_body() {
    let transport = FakeTransport::new();
    transport.push(
        "GET",
        "/v1/preferences",
        RawResponse {
            status: 200,
            body: serde_json::to_vec(&PreferencesDto {
                last_language: None,
                last_assistant_backend_id: None,
            })
            .unwrap(),
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    let _ = client.preferences(&attachment());
    let seen = client.transport.seen.borrow();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].path, "/v1/preferences");
    assert!(
        seen[0].authenticated,
        "preferences must be requested authenticated"
    );
    assert_eq!(seen[0].if_match, None);
}

#[test]
fn capabilities_and_setup_check_use_their_typed_authenticated_routes() {
    let transport = FakeTransport::new();
    transport.push(
        "GET",
        "/v1/capabilities",
        RawResponse {
            status: 200,
            body: serde_json::to_vec(&CapabilitiesDto {
                capabilities: vec!["read".to_owned()],
                manual_workflow_available: true,
                assistant_backends: Vec::new(),
                disabled_routes: Vec::new(),
            })
            .unwrap(),
            ..RawResponse::default()
        },
    );
    transport.push(
        "GET",
        "/v1/setup-check",
        RawResponse {
            status: 200,
            body: serde_json::to_vec(&SetupCheckDto {
                config_safe: true,
                database_ready: true,
                apple_helper_available: false,
                notion_configured: false,
                notion_reason_code: Some("notion-not-configured".to_owned()),
                manual_workflow_available: true,
            })
            .unwrap(),
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    assert!(matches!(
        client.capabilities(&attachment()),
        ClientOutcome::Ok(_)
    ));
    assert!(matches!(
        client.setup_check(&attachment()),
        ClientOutcome::Ok(_)
    ));
    let seen = client.transport.seen.borrow();
    assert_eq!(seen[0].path, "/v1/capabilities");
    assert_eq!(seen[1].path, "/v1/setup-check");
    assert!(seen.iter().all(|request| request.authenticated));
}

#[test]
fn saving_preferences_sends_the_complete_typed_document_without_an_etag() {
    let preferences = PreferencesDto {
        last_language: Some("ko-KR".to_owned()),
        last_assistant_backend_id: None,
    };
    let transport = FakeTransport::new();
    transport.push(
        "PUT",
        "/v1/preferences",
        RawResponse {
            status: 200,
            body: serde_json::to_vec(&preferences).unwrap(),
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    match client.save_preferences(&attachment(), &preferences) {
        ClientOutcome::Ok(saved) => assert_eq!(saved, preferences),
        _ => panic!("expected saved preferences"),
    }
    let seen = client.transport.seen.borrow();
    assert_eq!(seen[0].method, "PUT");
    assert_eq!(seen[0].path, "/v1/preferences");
    assert_eq!(seen[0].if_match, None);
    assert_eq!(
        serde_json::from_slice::<PreferencesDto>(seen[0].body.as_ref().unwrap()).unwrap(),
        preferences
    );
}

#[test]
fn workflow_pagination_percent_encodes_the_server_cursor() {
    let transport = FakeTransport::new();
    transport.push(
        "GET",
        "/v1/workflows?cursor=page%2F2",
        RawResponse {
            status: 200,
            body: br#"{"workflows":[],"next_cursor":null}"#.to_vec(),
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    assert!(matches!(
        client.list_workflows(&attachment(), Some("page/2")),
        ClientOutcome::Ok(_)
    ));
}

#[test]
fn a_typed_problem_response_never_becomes_ok() {
    let transport = FakeTransport::new();
    transport.push(
        "DELETE",
        "/v1/workflows/wf_20261029",
        RawResponse {
            status: 412,
            body: serde_json::json!({
                "type": "https://nd.local/problems/precondition-failed",
                "title": "Precondition failed",
                "status": 412,
                "code": "precondition-failed",
                "instance": null
            })
            .to_string()
            .into_bytes(),
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    match client.discard_workflow(&attachment(), "wf_20261029", 3) {
        ClientOutcome::Problem(problem) => {
            assert_eq!(problem.status, 412);
            assert_eq!(problem.code, "precondition-failed");
        }
        _ => panic!("expected a typed problem outcome"),
    }
}

#[test]
fn discard_sends_the_workflow_etag_as_if_match() {
    let transport = FakeTransport::new();
    transport.push(
        "DELETE",
        "/v1/workflows/wf_20261029",
        RawResponse {
            status: 204,
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    let outcome = client.discard_workflow(&attachment(), "wf_20261029", 5);
    assert!(matches!(outcome, ClientOutcome::Ok(())));
    let seen = client.transport.seen.borrow();
    assert_eq!(seen[0].if_match.as_deref(), Some("\"wf-5\""));
}

#[test]
fn a_malformed_body_never_panics_and_never_claims_ok() {
    let transport = FakeTransport::new();
    transport.push(
        "GET",
        "/v1/preferences",
        RawResponse {
            status: 200,
            body: b"not json".to_vec(),
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    match client.preferences(&attachment()) {
        ClientOutcome::Transport(TransportError::MalformedResponse) => {}
        other => panic!(
            "expected a malformed-response transport error, got something else: {}",
            matches!(other, ClientOutcome::Ok(_))
        ),
    }
}

#[test]
fn source_detail_with_handle_sends_and_retains_the_job_result_handle() {
    let transport = FakeTransport::new();
    transport.push(
        "GET",
        "/v1/workflows/wf_20261029/sources/src_1",
        RawResponse {
            status: 200,
            body: serde_json::json!({
                "id": "src_1",
                "source_note_name": "2026-10-28 note",
                "body": "synthetic body",
                "content_digest": "digest",
                "directory_day": "2026-10-28",
                "header_day": "2026-10-28",
                "warning_code": null
            })
            .to_string()
            .into_bytes(),
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    match client.source_detail_with_handle(&attachment(), "wf_20261029", "src_1", "handle_abc") {
        ClientOutcome::Ok(response) => {
            assert_eq!(response.preparation_handle.as_deref(), Some("handle_abc"));
            assert_eq!(response.detail.id, "src_1");
        }
        other => panic!(
            "expected Ok with a preparation handle, got Problem/Transport: {}",
            matches!(other, ClientOutcome::Ok(_))
        ),
    }
    assert_eq!(
        client.transport.seen.borrow()[0]
            .preparation_handle
            .as_deref(),
        Some("handle_abc")
    );
}

#[test]
fn assistant_selection_sends_the_revision_etag_and_a_null_backend_to_deselect() {
    let transport = FakeTransport::new();
    transport.push(
        "PUT",
        "/v1/workflows/wf_20261029/assistant-selection",
        RawResponse {
            status: 500,
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    let _ = client.set_assistant_selection(&attachment(), "wf_20261029", 7, None);
    let seen = client.transport.seen.borrow();
    assert_eq!(seen[0].method, "PUT");
    assert_eq!(seen[0].if_match.as_deref(), Some("\"wf-7\""));
    let body: serde_json::Value =
        serde_json::from_slice(seen[0].body.as_deref().expect("body")).unwrap();
    assert_eq!(body, serde_json::json!({ "backend_id": null }));
}

#[test]
fn start_and_writing_settings_carry_session_choices() {
    let transport = FakeTransport::new();
    transport.push(
        "POST",
        "/v1/workflows",
        RawResponse {
            status: 500,
            ..RawResponse::default()
        },
    );
    transport.push(
        "PUT",
        "/v1/workflows/wf_20261029/writing-settings",
        RawResponse {
            status: 500,
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    let _ = client.start_workflow(
        &attachment(),
        Some("2026-10-29"),
        Some("ko-KR"),
        Some("reflective"),
    );
    let _ = client.set_writing_settings(
        &attachment(),
        "wf_20261029",
        4,
        "ko-KR",
        "reflective",
        Some("synthetic-hosted"),
    );
    let seen = client.transport.seen.borrow();
    let start: serde_json::Value =
        serde_json::from_slice(seen[0].body.as_deref().unwrap()).unwrap();
    assert_eq!(start["language"], "ko-KR");
    assert_eq!(start["style"], "reflective");
    let settings: serde_json::Value =
        serde_json::from_slice(seen[1].body.as_deref().unwrap()).unwrap();
    assert_eq!(settings["assistant_backend_id"], "synthetic-hosted");
    assert_eq!(seen[1].if_match.as_deref(), Some("\"wf-4\""));
}

#[test]
fn a_stale_assistant_selection_is_a_typed_problem_never_ok() {
    let transport = FakeTransport::new();
    transport.push(
        "PUT",
        "/v1/workflows/wf_20261029/assistant-selection",
        RawResponse {
            status: 412,
            body: serde_json::json!({
                "type": "https://nd.local/problems/precondition-failed",
                "title": "Precondition failed",
                "status": 412,
                "code": "precondition-failed",
                "instance": null
            })
            .to_string()
            .into_bytes(),
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    match client.set_assistant_selection(
        &attachment(),
        "wf_20261029",
        3,
        Some("codex-default".to_owned()),
    ) {
        ClientOutcome::Problem(problem) => assert_eq!(problem.status, 412),
        _ => panic!("expected a typed problem outcome"),
    }
}
