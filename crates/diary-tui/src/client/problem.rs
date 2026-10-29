//! Typed-problem decoding. Only fixed, content-free views reach the UI.

use diary_control_contract::ProblemDto;

use super::{RawResponse, TransportError};

/// The result of one typed client call: a decoded success value, a typed
/// server problem, or a transport-level failure. There is no "arbitrary
/// string" variant.
pub enum ClientOutcome<T> {
    Ok(T),
    Problem(ProblemView),
    Transport(TransportError),
}

/// A safe, rendered view of an RFC 9457 problem response. Carries only the
/// server's stable `code`, not free-form title/detail text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProblemView {
    pub status: u16,
    pub code: String,
}

impl ProblemView {
    /// A fixed-shape, content-free label safe for the Status Line or Work
    /// Status: the server's stable `code` token, never title/detail text.
    #[must_use]
    pub fn safe_label(&self) -> String {
        format!("Request rejected: {}", self.code)
    }
}

impl TransportError {
    /// A fixed, content-free label for the Status Line or Work Status —
    /// never the underlying library/transport error string.
    #[must_use]
    pub const fn safe_label(self) -> &'static str {
        match self {
            Self::ConnectFailed => "Connection failed",
            Self::Timeout => "Request timed out",
            Self::OversizedResponse => "Response too large",
            Self::MalformedResponse => "Malformed response",
            Self::Io => "Connection error",
        }
    }
}

pub(super) fn parse(response: &RawResponse) -> ProblemView {
    match serde_json::from_slice::<ProblemDto>(&response.body) {
        Ok(problem) => ProblemView {
            status: response.status,
            code: problem.code,
        },
        Err(_) => ProblemView {
            status: response.status,
            code: "malformed-problem".to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_problem_label_carries_only_the_stable_code_not_title_or_detail() {
        let problem = ProblemView {
            status: 412,
            code: "precondition-failed".to_owned(),
        };
        assert_eq!(
            problem.safe_label(),
            "Request rejected: precondition-failed"
        );
    }

    #[test]
    fn every_transport_error_has_a_fixed_fully_static_label() {
        for error in [
            TransportError::ConnectFailed,
            TransportError::Timeout,
            TransportError::OversizedResponse,
            TransportError::MalformedResponse,
            TransportError::Io,
        ] {
            assert!(!error.safe_label().is_empty());
        }
    }
}
