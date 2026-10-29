//! Bounded HTTP transport boundary.
//!
//! `Transport` is the only thing that touches the network. Production code
//! uses [`UreqTransport`]; tests use a fake that implements the same trait so
//! `ApiClient` logic (routing, headers, body caps, conflict handling) is
//! testable without a real server.

use std::time::Duration;

use ureq::Agent;
use ureq::config::Config;

/// Validated native-client attachment data. Intentionally has no `Debug`
/// implementation because it carries the bearer token.
#[derive(Clone)]
pub struct Attachment {
    instance_id: String,
    host: String,
    port: u16,
    control_token: String,
    contract_version: u32,
}

impl Attachment {
    #[must_use]
    pub fn new(
        instance_id: String,
        host: String,
        port: u16,
        control_token: String,
        contract_version: u32,
    ) -> Self {
        Self {
            instance_id,
            host,
            port,
            control_token,
            contract_version,
        }
    }

    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    #[must_use]
    pub fn control_token(&self) -> &str {
        &self.control_token
    }

    #[must_use]
    pub const fn contract_version(&self) -> u32 {
        self.contract_version
    }

    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

/// A bounded, purpose-built request. There is no way to pass an absolute or
/// externally supplied URL: callers only choose a path and body.
pub struct RawRequest {
    pub method: &'static str,
    pub path: String,
    pub authenticated: bool,
    pub body: Option<Vec<u8>>,
    pub if_match: Option<String>,
    pub max_response_bytes: usize,
}

#[derive(Clone, Debug, Default)]
pub struct RawResponse {
    pub status: u16,
    pub etag: Option<String>,
    pub buffer_etag: Option<String>,
    pub preparation_handle: Option<String>,
    pub body: Vec<u8>,
}

/// Fixed, content-free transport failure codes. Never carries an arbitrary
/// transport/library error string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportError {
    ConnectFailed,
    Timeout,
    OversizedResponse,
    MalformedResponse,
    Io,
}

pub trait Transport {
    fn execute(
        &self,
        attachment: &Attachment,
        request: &RawRequest,
    ) -> Result<RawResponse, TransportError>;
}

/// Production transport: one blocking `ureq` agent bound to the validated
/// loopback base URL, with proxy discovery, redirects, TLS, cookies, and
/// compression disabled.
pub struct UreqTransport {
    agent: Agent,
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const HEADER_TIMEOUT: Duration = Duration::from_secs(5);
const BODY_TIMEOUT: Duration = Duration::from_secs(30);

impl UreqTransport {
    #[must_use]
    pub fn new() -> Self {
        let config: Config = Agent::config_builder()
            .proxy(None)
            .https_only(false)
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_send_request(Some(SEND_TIMEOUT))
            .timeout_recv_response(Some(HEADER_TIMEOUT))
            .timeout_recv_body(Some(BODY_TIMEOUT))
            .build();
        Self {
            agent: Agent::new_with_config(config),
        }
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for UreqTransport {
    fn execute(
        &self,
        attachment: &Attachment,
        request: &RawRequest,
    ) -> Result<RawResponse, TransportError> {
        let url = format!("{}{}", attachment.base_url(), request.path);
        let auth = request
            .authenticated
            .then(|| format!("Bearer {}", attachment.control_token()));
        let send_result = match request.method {
            "GET" => {
                with_common_headers(self.agent.get(&url), auth.as_deref(), &request.if_match).call()
            }
            "DELETE" => {
                with_common_headers(self.agent.delete(&url), auth.as_deref(), &request.if_match)
                    .call()
            }
            "POST" | "PUT" => {
                let builder = if request.method == "POST" {
                    self.agent.post(&url)
                } else {
                    self.agent.put(&url)
                };
                let builder = with_common_headers(builder, auth.as_deref(), &request.if_match)
                    .header("Content-Type", "application/json");
                match &request.body {
                    Some(bytes) => builder.send(bytes.as_slice()),
                    None => builder.send_empty(),
                }
            }
            _ => return Err(TransportError::MalformedResponse),
        };
        let mut response = send_result.map_err(classify_send_error)?;

        if let Some(length) = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok())
            && length > request.max_response_bytes
        {
            return Err(TransportError::OversizedResponse);
        }

        let etag = header_value(&response, "etag");
        let buffer_etag = header_value(&response, "x-entry-buffer-etag");
        let preparation_handle = header_value(&response, "x-nd-preparation-handle");
        let status = response.status().as_u16();

        let body = response
            .body_mut()
            .with_config()
            .limit(request.max_response_bytes as u64 + 1)
            .read_to_vec()
            .map_err(|_| TransportError::Timeout)?;
        if body.len() > request.max_response_bytes {
            return Err(TransportError::OversizedResponse);
        }

        Ok(RawResponse {
            status,
            etag,
            buffer_etag,
            preparation_handle,
            body,
        })
    }
}

fn with_common_headers<State>(
    mut builder: ureq::RequestBuilder<State>,
    auth: Option<&str>,
    if_match: &Option<String>,
) -> ureq::RequestBuilder<State> {
    if let Some(auth) = auth {
        builder = builder.header("Authorization", auth);
    }
    if let Some(if_match) = if_match {
        builder = builder.header("If-Match", if_match);
    }
    builder
}

fn header_value(response: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn classify_send_error(error: ureq::Error) -> TransportError {
    match error {
        ureq::Error::Timeout(_) => TransportError::Timeout,
        ureq::Error::ConnectionFailed | ureq::Error::Io(_) => TransportError::ConnectFailed,
        _ => TransportError::MalformedResponse,
    }
}
