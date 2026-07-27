//! Runtime-owned presentation of admitted operational-usage facts.
//!
//! Server owns the operational-usage schema, its admission/pricing fields, and
//! the raw fact body. The runtime only supplies a Server-owned preparation port
//! with the real native model measurement after its atomic commit succeeds.
//! The concrete publisher then makes the one direct Auth workload-attestation
//! call and one Server usage-fact call. It never holds a Server bearer, accepts
//! Studio/OS authority, copies Auth claims, retries a presentation, or treats a
//! transport ambiguity as admission.

use std::{
    env, fmt, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Method, Url};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use apxm_inference::Usage;

/// The immutable native measurement associated with a successful runtime
/// execution commit. Peer/ACP usage has no representation here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommittedNativeUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl From<Usage> for CommittedNativeUsage {
    fn from(value: Usage) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
        }
    }
}

/// The exact post-commit data Server needs to create an admitted usage fact.
///
/// `ServerUsageFactPreparationPort` is the composition handoff for the
/// Server-owned fact schema. Agents deliberately does not mirror Server's
/// admission, reservation, price, or evidence DTOs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationalUsageFactPublishRequest {
    pub commit_id: String,
    pub invocation_ref: String,
    pub evidence_position_ref: String,
    pub native_usage: CommittedNativeUsage,
}

/// An exact Server-owned fact body. The bytes must be forwarded without JSON
/// parsing or re-serialization because the workload-admission digest binds the
/// raw request bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedOperationalUsageFact {
    raw_body: Vec<u8>,
}

impl PreparedOperationalUsageFact {
    pub fn from_server_owned_body(raw_body: Vec<u8>) -> Result<Self, UsageFactDeliveryError> {
        if raw_body.is_empty() {
            return Err(UsageFactDeliveryError::EmptyServerOwnedBody);
        }
        Ok(Self { raw_body })
    }

    fn raw_body(&self) -> &[u8] {
        &self.raw_body
    }
}

/// The Server/composition-owned boundary that serializes an admitted usage
/// fact after real runtime measurement is available.
#[async_trait]
pub trait ServerUsageFactPreparationPort: Send + Sync {
    async fn prepare(
        &self,
        request: OperationalUsageFactPublishRequest,
    ) -> Result<PreparedOperationalUsageFact, UsageFactDeliveryError>;
}

/// The runtime-owned boundary invoked only from the completed atomic commit
/// path. Implementations must not internally retry a presentation because Auth
/// attestations are one-use.
#[async_trait]
pub trait OperationalUsageFactPort: Send + Sync {
    async fn publish(
        &self,
        request: OperationalUsageFactPublishRequest,
    ) -> Result<(), UsageFactDeliveryError>;
}

/// The report-visible outcome. A failed delivery is deliberately retained as a
/// failure, rather than masquerading as a Server-admitted operational fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationalUsageOutcome {
    NotApplicable,
    NotConfigured,
    Published,
    Failed(UsageFactDeliveryError),
}

/// The two direct endpoints and the runtime's only credential source.
#[derive(Clone, Debug)]
pub struct RuntimeWorkloadEndpoints {
    auth_workload_attestation: Url,
    server_usage_facts: Url,
    runtime_bearer_file: PathBuf,
}

impl RuntimeWorkloadEndpoints {
    pub const RUNTIME_BEARER_FILE_ENV: &str = "APXM_AUTH_RUNTIME_BEARER_FILE";
    const AUTH_PATH: &str = "/v1/identity/workload-attestation";
    const SERVER_PATH: &str = "/v1/usage/facts";

    /// Build direct, path-pinned endpoints from service origins. Origins must
    /// have no query, fragment, or path so callers cannot divert the protocol.
    pub fn new(
        auth_origin: &str,
        server_origin: &str,
        runtime_bearer_file: PathBuf,
    ) -> Result<Self, UsageFactDeliveryError> {
        Ok(Self {
            auth_workload_attestation: endpoint_from_origin(auth_origin, Self::AUTH_PATH)?,
            server_usage_facts: endpoint_from_origin(server_origin, Self::SERVER_PATH)?,
            runtime_bearer_file,
        })
    }

    /// Read the one authorized runtime credential location. There is no Server
    /// bearer fallback and no ambient credential discovery.
    pub fn from_environment(
        auth_origin: &str,
        server_origin: &str,
    ) -> Result<Self, UsageFactDeliveryError> {
        let runtime_bearer_file = env::var_os(Self::RUNTIME_BEARER_FILE_ENV)
            .map(PathBuf::from)
            .ok_or(UsageFactDeliveryError::MissingRuntimeBearerFile)?;
        Self::new(auth_origin, server_origin, runtime_bearer_file)
    }
}

fn endpoint_from_origin(origin: &str, path: &str) -> Result<Url, UsageFactDeliveryError> {
    let mut url = Url::parse(origin).map_err(|_| UsageFactDeliveryError::InvalidEndpoint)?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(UsageFactDeliveryError::InvalidEndpoint);
    }
    if url.query().is_some() || url.fragment().is_some() || url.path() != "/" {
        return Err(UsageFactDeliveryError::InvalidEndpoint);
    }
    url.set_path(path);
    Ok(url)
}

/// The smallest HTTP abstraction needed to make exact request conformance
/// testable without a fake successful Server implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkloadHttpRequest {
    pub method: Method,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkloadHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait WorkloadHttpTransport: Send + Sync {
    async fn execute(
        &self,
        request: WorkloadHttpRequest,
    ) -> Result<WorkloadHttpResponse, UsageFactDeliveryError>;
}

/// The production HTTP transport. It performs one request for each caller
/// invocation and does not enable retry middleware.
#[derive(Clone, Default)]
pub struct ReqwestWorkloadHttpTransport {
    client: reqwest::Client,
}

impl ReqwestWorkloadHttpTransport {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl WorkloadHttpTransport for ReqwestWorkloadHttpTransport {
    async fn execute(
        &self,
        request: WorkloadHttpRequest,
    ) -> Result<WorkloadHttpResponse, UsageFactDeliveryError> {
        let mut builder = self
            .client
            .request(request.method, request.url)
            .body(request.body);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        let response = builder
            .send()
            .await
            .map_err(|_| UsageFactDeliveryError::TransportUnavailable)?;
        let status = response.status().as_u16();
        let body = response
            .bytes()
            .await
            .map_err(|_| UsageFactDeliveryError::TransportUnavailable)?
            .to_vec();
        Ok(WorkloadHttpResponse { status, body })
    }
}

/// Direct workload-attestation and usage-fact publisher. Server-owned fact
/// preparation is injected; Auth's envelope is parsed only for canonical
/// validation and is then forwarded as its exact two encoded components.
pub struct AuthenticatedUsageFactPublisher {
    preparation: Arc<dyn ServerUsageFactPreparationPort>,
    endpoints: RuntimeWorkloadEndpoints,
    transport: Arc<dyn WorkloadHttpTransport>,
}

impl AuthenticatedUsageFactPublisher {
    pub fn new(
        preparation: Arc<dyn ServerUsageFactPreparationPort>,
        endpoints: RuntimeWorkloadEndpoints,
        transport: Arc<dyn WorkloadHttpTransport>,
    ) -> Self {
        Self {
            preparation,
            endpoints,
            transport,
        }
    }

    async fn deliver(
        &self,
        prepared: PreparedOperationalUsageFact,
    ) -> Result<(), UsageFactDeliveryError> {
        let digest = workload_admission_digest(prepared.raw_body());
        let bearer = read_runtime_bearer(&self.endpoints.runtime_bearer_file)?;
        let nonce = Uuid::new_v4().to_string();
        let auth_response = self
            .transport
            .execute(WorkloadHttpRequest {
                method: Method::POST,
                url: self.endpoints.auth_workload_attestation.clone(),
                headers: vec![
                    ("authorization".into(), format!("Bearer {bearer}")),
                    ("x-apxm-authority-nonce".into(), nonce),
                    ("x-apxm-admission-request-digest".into(), digest),
                ],
                body: Vec::new(),
            })
            .await
            .map_err(auth_transport_error)?;
        if !(200..300).contains(&auth_response.status) {
            return Err(UsageFactDeliveryError::AuthRejected(auth_response.status));
        }
        let presentation = WorkloadPresentation::from_auth_response(&auth_response.body)?;
        let server_response = self
            .transport
            .execute(WorkloadHttpRequest {
                method: Method::POST,
                url: self.endpoints.server_usage_facts.clone(),
                headers: vec![
                    ("content-type".into(), "application/json".into()),
                    (
                        "x-apxm-workload-presentation".into(),
                        presentation.header_value(),
                    ),
                ],
                body: prepared.raw_body,
            })
            .await
            .map_err(server_transport_error)?;
        if !(200..300).contains(&server_response.status) {
            return Err(UsageFactDeliveryError::ServerRejected(
                server_response.status,
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl OperationalUsageFactPort for AuthenticatedUsageFactPublisher {
    async fn publish(
        &self,
        request: OperationalUsageFactPublishRequest,
    ) -> Result<(), UsageFactDeliveryError> {
        let prepared = self.preparation.prepare(request).await?;
        self.deliver(prepared).await
    }
}

/// Compute the digest accepted by Server's workload-admission verifier. The
/// preimage intentionally has no terminal newline after the inner raw-body
/// digest; changing a byte of the JSON changes the resulting digest.
pub fn workload_admission_digest(raw_body: &[u8]) -> String {
    workload_admission_digest_for("POST", RuntimeWorkloadEndpoints::SERVER_PATH, raw_body)
}

fn workload_admission_digest_for(method: &str, path: &str, raw_body: &[u8]) -> String {
    let inner = hex_lower(&Sha256::digest(raw_body));
    let preimage = format!("apxm.server.workload-admission-request.v1\n{method}\n{path}\n{inner}");
    format!("sha256:{}", hex_lower(&Sha256::digest(preimage.as_bytes())))
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn auth_transport_error(error: UsageFactDeliveryError) -> UsageFactDeliveryError {
    match error {
        UsageFactDeliveryError::TransportUnavailable => UsageFactDeliveryError::AuthUnavailable,
        other => other,
    }
}

fn server_transport_error(error: UsageFactDeliveryError) -> UsageFactDeliveryError {
    match error {
        UsageFactDeliveryError::TransportUnavailable => UsageFactDeliveryError::ServerUnavailable,
        other => other,
    }
}

fn read_runtime_bearer(path: &Path) -> Result<String, UsageFactDeliveryError> {
    let contents =
        fs::read_to_string(path).map_err(|_| UsageFactDeliveryError::RuntimeBearerRead)?;
    let bearer = contents.trim();
    if bearer.is_empty() || bearer.chars().any(char::is_whitespace) {
        return Err(UsageFactDeliveryError::InvalidRuntimeBearer);
    }
    Ok(bearer.to_string())
}

struct WorkloadPresentation {
    payload: String,
    signature: String,
}

impl WorkloadPresentation {
    fn from_auth_response(body: &[u8]) -> Result<Self, UsageFactDeliveryError> {
        let value: Value = serde_json::from_slice(body)
            .map_err(|_| UsageFactDeliveryError::MalformedAuthEnvelope)?;
        let payload = value
            .get("payload")
            .and_then(Value::as_str)
            .ok_or(UsageFactDeliveryError::MalformedAuthEnvelope)?;
        let signature = value
            .get("signature")
            .and_then(Value::as_str)
            .ok_or(UsageFactDeliveryError::MalformedAuthEnvelope)?;
        validate_base64url(payload, false)?;
        validate_base64url(signature, true)?;
        Ok(Self {
            payload: payload.to_string(),
            signature: signature.to_string(),
        })
    }

    fn header_value(&self) -> String {
        format!("{}.{}", self.payload, self.signature)
    }
}

fn validate_base64url(value: &str, signature: bool) -> Result<(), UsageFactDeliveryError> {
    if value.is_empty()
        || value.contains('=')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(UsageFactDeliveryError::MalformedAuthEnvelope);
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| UsageFactDeliveryError::MalformedAuthEnvelope)?;
    if URL_SAFE_NO_PAD.encode(&decoded) != value
        || (signature && (value.len() != 86 || decoded.len() != 64))
    {
        return Err(UsageFactDeliveryError::MalformedAuthEnvelope);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsageFactDeliveryError {
    EmptyServerOwnedBody,
    MissingRuntimeBearerFile,
    RuntimeBearerRead,
    InvalidRuntimeBearer,
    InvalidEndpoint,
    TransportUnavailable,
    AuthUnavailable,
    AuthRejected(u16),
    MalformedAuthEnvelope,
    ServerUnavailable,
    ServerRejected(u16),
}

impl fmt::Display for UsageFactDeliveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for UsageFactDeliveryError {}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use super::*;

    const BODY: &[u8] = br#"{"schema_version":"apxm.operational-usage-fact.v1","quantity":3}"#;
    const PAYLOAD: &str = "eyJzY2hlbWFfdmVyc2lvbiI6ImFweG0ud29ya2xvYWQtYXR0ZXN0YXRpb24udjEifQ";
    const SIGNATURE: &str =
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    #[derive(Default)]
    struct Preparation;

    #[async_trait]
    impl ServerUsageFactPreparationPort for Preparation {
        async fn prepare(
            &self,
            _request: OperationalUsageFactPublishRequest,
        ) -> Result<PreparedOperationalUsageFact, UsageFactDeliveryError> {
            PreparedOperationalUsageFact::from_server_owned_body(BODY.to_vec())
        }
    }

    struct Transport {
        responses: Mutex<VecDeque<Result<WorkloadHttpResponse, UsageFactDeliveryError>>>,
        requests: Mutex<Vec<WorkloadHttpRequest>>,
    }

    impl Transport {
        fn with(responses: Vec<Result<WorkloadHttpResponse, UsageFactDeliveryError>>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl WorkloadHttpTransport for Transport {
        async fn execute(
            &self,
            request: WorkloadHttpRequest,
        ) -> Result<WorkloadHttpResponse, UsageFactDeliveryError> {
            self.requests.lock().unwrap().push(request);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("one response per one request")
        }
    }

    fn response(
        status: u16,
        body: impl Into<Vec<u8>>,
    ) -> Result<WorkloadHttpResponse, UsageFactDeliveryError> {
        Ok(WorkloadHttpResponse {
            status,
            body: body.into(),
        })
    }

    fn endpoints() -> (RuntimeWorkloadEndpoints, tempfile::NamedTempFile) {
        let file = tempfile::NamedTempFile::new().unwrap();
        fs::write(file.path(), "runtime-bearer\n").unwrap();
        (
            RuntimeWorkloadEndpoints::new(
                "https://auth.example/",
                "https://server.example/",
                file.path().to_path_buf(),
            )
            .unwrap(),
            file,
        )
    }

    fn publish_request() -> OperationalUsageFactPublishRequest {
        OperationalUsageFactPublishRequest {
            commit_id: "commit.1".into(),
            invocation_ref: "invocation.1".into(),
            evidence_position_ref: "evidence.1".into(),
            native_usage: CommittedNativeUsage {
                input_tokens: 1,
                output_tokens: 2,
            },
        }
    }

    #[test]
    fn workload_digest_matches_server_pinned_event_resume_vector() {
        let raw = br#"{"event_ref":"event-001","delivery_ref":"delivery-001","idempotency_key":"key-001","payload":{"message":"continue"}}"#;
        assert_eq!(
            workload_admission_digest_for("POST", "/v1/event-resumes", raw),
            "sha256:7f06e71ae4434c37da0ab50fd002f332287bdf91ee0afb055783345c9eb253ef"
        );
    }

    #[test]
    fn raw_body_bytes_change_workload_digest() {
        assert_ne!(
            workload_admission_digest(br#"{"a":1}"#),
            workload_admission_digest(br#"{ "a": 1 }"#)
        );
    }

    #[test]
    fn endpoint_requires_plain_origin_and_fixed_paths() {
        assert!(
            RuntimeWorkloadEndpoints::new(
                "https://auth.example/x",
                "https://server.example/",
                PathBuf::from("bearer")
            )
            .is_err()
        );
        assert!(
            RuntimeWorkloadEndpoints::new(
                "https://auth.example/?x=1",
                "https://server.example/",
                PathBuf::from("bearer")
            )
            .is_err()
        );
        let endpoints = RuntimeWorkloadEndpoints::new(
            "https://auth.example/",
            "https://server.example/",
            PathBuf::from("bearer"),
        )
        .unwrap();
        assert_eq!(
            endpoints.auth_workload_attestation.path(),
            RuntimeWorkloadEndpoints::AUTH_PATH
        );
        assert_eq!(
            endpoints.server_usage_facts.path(),
            RuntimeWorkloadEndpoints::SERVER_PATH
        );
    }

    #[tokio::test]
    async fn forwards_auth_envelope_and_raw_body_exactly_once() {
        let (endpoints, _file) = endpoints();
        let transport = Arc::new(Transport::with(vec![
            response(
                200,
                format!(r#"{{"payload":"{PAYLOAD}","signature":"{SIGNATURE}"}}"#),
            ),
            response(201, Vec::new()),
        ]));
        let publisher = AuthenticatedUsageFactPublisher::new(
            Arc::new(Preparation),
            endpoints,
            transport.clone(),
        );

        publisher.publish(publish_request()).await.unwrap();

        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, Method::POST);
        assert_eq!(requests[0].url.path(), RuntimeWorkloadEndpoints::AUTH_PATH);
        assert_eq!(requests[0].body, Vec::<u8>::new());
        assert!(
            requests[0]
                .headers
                .iter()
                .any(|(name, value)| name == "authorization" && value == "Bearer runtime-bearer")
        );
        let nonce = requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "x-apxm-authority-nonce")
            .unwrap()
            .1
            .clone();
        assert!(Uuid::parse_str(&nonce).is_ok());
        assert!(
            requests[0]
                .headers
                .iter()
                .any(|(name, value)| name == "x-apxm-admission-request-digest"
                    && value == &workload_admission_digest(BODY))
        );
        assert_eq!(
            requests[1].url.path(),
            RuntimeWorkloadEndpoints::SERVER_PATH
        );
        assert_eq!(requests[1].body, BODY);
        assert_eq!(
            requests[1].headers,
            vec![
                ("content-type".into(), "application/json".into()),
                (
                    "x-apxm-workload-presentation".into(),
                    format!("{PAYLOAD}.{SIGNATURE}")
                ),
            ]
        );
    }

    #[tokio::test]
    async fn malformed_auth_envelope_never_calls_server() {
        let (endpoints, _file) = endpoints();
        let transport = Arc::new(Transport::with(vec![response(
            200,
            br#"{"payload":"bad=","signature":"bad"}"#.to_vec(),
        )]));
        let publisher = AuthenticatedUsageFactPublisher::new(
            Arc::new(Preparation),
            endpoints,
            transport.clone(),
        );

        assert_eq!(
            publisher.publish(publish_request()).await,
            Err(UsageFactDeliveryError::MalformedAuthEnvelope)
        );
        assert_eq!(transport.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn auth_rejection_never_calls_server() {
        let (endpoints, _file) = endpoints();
        let transport = Arc::new(Transport::with(vec![response(401, Vec::new())]));
        let publisher = AuthenticatedUsageFactPublisher::new(
            Arc::new(Preparation),
            endpoints,
            transport.clone(),
        );

        assert_eq!(
            publisher.publish(publish_request()).await,
            Err(UsageFactDeliveryError::AuthRejected(401))
        );
        assert_eq!(transport.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn auth_unavailability_never_calls_server() {
        let (endpoints, _file) = endpoints();
        let transport = Arc::new(Transport::with(vec![Err(
            UsageFactDeliveryError::TransportUnavailable,
        )]));
        let publisher = AuthenticatedUsageFactPublisher::new(
            Arc::new(Preparation),
            endpoints,
            transport.clone(),
        );

        assert_eq!(
            publisher.publish(publish_request()).await,
            Err(UsageFactDeliveryError::AuthUnavailable)
        );
        assert_eq!(transport.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn each_publish_attempt_gets_a_fresh_auth_nonce() {
        let (endpoints, _file) = endpoints();
        let envelope = format!(r#"{{"payload":"{PAYLOAD}","signature":"{SIGNATURE}"}}"#);
        let transport = Arc::new(Transport::with(vec![
            response(200, envelope.clone()),
            response(201, Vec::new()),
            response(200, envelope),
            response(201, Vec::new()),
        ]));
        let publisher = AuthenticatedUsageFactPublisher::new(
            Arc::new(Preparation),
            endpoints,
            transport.clone(),
        );

        publisher.publish(publish_request()).await.unwrap();
        publisher.publish(publish_request()).await.unwrap();

        let requests = transport.requests.lock().unwrap();
        let nonces: Vec<_> = requests
            .iter()
            .step_by(2)
            .map(|request| {
                request
                    .headers
                    .iter()
                    .find(|(name, _)| name == "x-apxm-authority-nonce")
                    .expect("auth request contains nonce")
                    .1
                    .clone()
            })
            .collect();
        assert_eq!(nonces.len(), 2);
        assert_ne!(nonces[0], nonces[1]);
    }

    #[tokio::test]
    async fn failed_server_delivery_does_not_retry_or_reuse_presentation() {
        let (endpoints, _file) = endpoints();
        let transport = Arc::new(Transport::with(vec![
            response(
                200,
                format!(r#"{{"payload":"{PAYLOAD}","signature":"{SIGNATURE}"}}"#),
            ),
            Err(UsageFactDeliveryError::TransportUnavailable),
        ]));
        let publisher = AuthenticatedUsageFactPublisher::new(
            Arc::new(Preparation),
            endpoints,
            transport.clone(),
        );

        assert_eq!(
            publisher.publish(publish_request()).await,
            Err(UsageFactDeliveryError::ServerUnavailable)
        );
        assert_eq!(transport.requests.lock().unwrap().len(), 2);
    }
}
