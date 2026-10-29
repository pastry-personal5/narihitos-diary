//! Pure attachment-state decision: compares the validated runtime
//! advertisement against unauthenticated health before the bearer token is
//! trusted for any further request. See the architecture doc's "Attachment
//! and HTTP safety" section.

use diary_control_contract::CONTRACT_VERSION;

use super::{ApiClient, Attachment, ClientOutcome, Transport};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttachmentState {
    /// No runtime file, or the transport could not reach the advertised
    /// port at all.
    NotRunning,
    /// Health responded but with a different instance ID than the runtime
    /// file advertised (a new server instance since the file was read).
    InstanceMismatch,
    /// Health's API or contract version does not match what this client
    /// supports.
    VersionMismatch,
    /// Health responded but could not be parsed, or returned an unexpected
    /// status — distinct from "not running" because something is listening.
    UnsafeAttachment,
    /// Instance and contract version match; the bearer token may now be
    /// attached to further requests.
    Connected,
}

/// Performs the unauthenticated health check and decides whether the
/// bearer token in `attachment` may be trusted. Never sends the token
/// itself (the health route is unauthenticated), and never creates a
/// workflow.
pub fn verify_health<T: Transport>(
    client: &ApiClient<T>,
    attachment: &Attachment,
) -> AttachmentState {
    match client.health(attachment) {
        ClientOutcome::Ok(health) => {
            if health.api_version != diary_control_contract::API_VERSION {
                AttachmentState::UnsafeAttachment
            } else if health.instance_id != attachment.instance_id() {
                AttachmentState::InstanceMismatch
            } else if health.contract_version != CONTRACT_VERSION
                || health.contract_version != attachment.contract_version()
            {
                AttachmentState::VersionMismatch
            } else {
                AttachmentState::Connected
            }
        }
        ClientOutcome::Problem(_) => AttachmentState::UnsafeAttachment,
        // Something responded but not sensibly: distinct from "nothing is
        // listening" so the UI never tells a person to run `nd serve` when
        // a process already is.
        ClientOutcome::Transport(
            super::TransportError::MalformedResponse | super::TransportError::OversizedResponse,
        ) => AttachmentState::UnsafeAttachment,
        ClientOutcome::Transport(_) => AttachmentState::NotRunning,
    }
}

#[cfg(test)]
mod tests {
    use diary_control_contract::HealthDto;

    use super::*;
    use crate::client::{RawRequest, RawResponse, TransportError};
    use std::cell::RefCell;

    struct FixedTransport {
        response: RefCell<Option<Result<RawResponse, TransportError>>>,
        last_authenticated: RefCell<Option<bool>>,
    }

    impl Transport for FixedTransport {
        fn execute(
            &self,
            _attachment: &Attachment,
            request: &RawRequest,
        ) -> Result<RawResponse, TransportError> {
            *self.last_authenticated.borrow_mut() = Some(request.authenticated);
            self.response
                .borrow_mut()
                .take()
                .expect("test must configure exactly one response")
        }
    }

    fn attachment(instance_id: &str, contract_version: u32) -> Attachment {
        Attachment::new(
            instance_id.to_owned(),
            "127.0.0.1".to_owned(),
            4000,
            "token_synthetic".to_owned(),
            contract_version,
        )
    }

    fn health_response(instance_id: &str, contract_version: u32) -> RawResponse {
        RawResponse {
            status: 200,
            body: serde_json::to_vec(&HealthDto {
                api_version: diary_control_contract::API_VERSION.to_owned(),
                contract_version,
                instance_id: instance_id.to_owned(),
                build_version: "synthetic".to_owned(),
                ready: true,
            })
            .unwrap(),
            ..RawResponse::default()
        }
    }

    #[test]
    fn matching_instance_and_contract_version_connects_without_sending_the_token() {
        let transport = FixedTransport {
            response: RefCell::new(Some(Ok(health_response("instance_a", CONTRACT_VERSION)))),
            last_authenticated: RefCell::new(None),
        };
        let client = ApiClient::new(transport);
        let state = verify_health(&client, &attachment("instance_a", CONTRACT_VERSION));
        assert_eq!(state, AttachmentState::Connected);
        assert_eq!(
            *client.transport.last_authenticated.borrow(),
            Some(false),
            "health must never be requested authenticated"
        );
    }

    #[test]
    fn a_different_instance_id_is_a_mismatch_not_a_connection() {
        let transport = FixedTransport {
            response: RefCell::new(Some(Ok(health_response("instance_b", CONTRACT_VERSION)))),
            last_authenticated: RefCell::new(None),
        };
        let client = ApiClient::new(transport);
        let state = verify_health(&client, &attachment("instance_a", CONTRACT_VERSION));
        assert_eq!(state, AttachmentState::InstanceMismatch);
    }

    #[test]
    fn an_incompatible_contract_version_is_a_mismatch() {
        let transport = FixedTransport {
            response: RefCell::new(Some(Ok(health_response(
                "instance_a",
                CONTRACT_VERSION + 1,
            )))),
            last_authenticated: RefCell::new(None),
        };
        let client = ApiClient::new(transport);
        let state = verify_health(&client, &attachment("instance_a", CONTRACT_VERSION + 1));
        assert_eq!(state, AttachmentState::VersionMismatch);
    }

    #[test]
    fn a_transport_failure_means_not_running() {
        let transport = FixedTransport {
            response: RefCell::new(Some(Err(TransportError::ConnectFailed))),
            last_authenticated: RefCell::new(None),
        };
        let client = ApiClient::new(transport);
        let state = verify_health(&client, &attachment("instance_a", CONTRACT_VERSION));
        assert_eq!(state, AttachmentState::NotRunning);
    }

    #[test]
    fn an_unparseable_health_body_is_unsafe_not_not_running() {
        let transport = FixedTransport {
            response: RefCell::new(Some(Ok(RawResponse {
                status: 200,
                body: b"not json".to_vec(),
                ..RawResponse::default()
            }))),
            last_authenticated: RefCell::new(None),
        };
        let client = ApiClient::new(transport);
        let state = verify_health(&client, &attachment("instance_a", CONTRACT_VERSION));
        assert_eq!(state, AttachmentState::UnsafeAttachment);
    }
}
