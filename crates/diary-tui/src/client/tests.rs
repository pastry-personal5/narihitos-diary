use std::cell::RefCell;

use diary_control_contract::PreferencesDto;

use super::{
    ApiClient, Attachment, ClientOutcome, RawRequest, RawResponse, Transport, TransportError,
};

struct SeenRequest {
    method: &'static str,
    path: String,
    authenticated: bool,
    if_match: Option<String>,
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
fn source_detail_with_handle_carries_the_preparation_handle_header_through() {
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
            preparation_handle: Some("handle_abc".to_owned()),
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    match client.source_detail_with_handle(&attachment(), "wf_20261029", "src_1") {
        ClientOutcome::Ok(response) => {
            assert_eq!(response.preparation_handle.as_deref(), Some("handle_abc"));
            assert_eq!(response.detail.id, "src_1");
        }
        other => panic!(
            "expected Ok with a preparation handle, got Problem/Transport: {}",
            matches!(other, ClientOutcome::Ok(_))
        ),
    }
}
