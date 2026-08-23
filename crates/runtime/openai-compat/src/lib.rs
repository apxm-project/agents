//! Inbound OpenAI HTTP protocol adapter over the Runtime Service.
//!
//! Serving aliases bind committed artifacts only. This crate shares no DTO with
//! retired `apxm chat`.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Semaphore};

use apxm_runtime_protocol::{
    RUNTIME_PROTOCOL_VERSION, RuntimeHandshake, RuntimeRequest, RuntimeResult,
};
use apxm_runtime_service::RuntimeService;

pub const READ_CHUNK_BYTES: usize = 8 * 1024;
pub const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
pub const HTTP_IO_TIMEOUT_MS: u64 = 5_000;
pub const MAX_IN_FLIGHT_CONNECTIONS: usize = 32;

/// Published serving alias. Source packages are not accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServingAlias {
    /// OpenAI `model` id.
    pub model: String,
    /// Already committed artifact digest.
    pub artifact_digest: String,
}

impl ServingAlias {
    /// Reject empty model ids and empty digests.
    pub fn validate(&self) -> Result<(), String> {
        if self.model.trim().is_empty() || self.artifact_digest.trim().is_empty() {
            return Err("serving alias requires model and artifact digest".to_owned());
        }
        Ok(())
    }
}

/// Bounded `/v1/responses` projection. Hidden agent-loop fields fail closed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsesProjection {
    /// Request id.
    pub id: String,
    /// Mapped finish reason from canonical terminal state.
    pub finish_reason: String,
}

/// Fields this adapter never treats as Capability grants.
#[must_use]
pub fn messages_cannot_mint_capabilities() -> bool {
    true
}

/// Dispatch one OpenAI-compatible request. Serving aliases are committed digests.
pub fn dispatch(
    service: &mut RuntimeService,
    aliases: &BTreeMap<String, String>,
    method: &str,
    path: &str,
    body: &Value,
) -> (u16, Value) {
    match (method, path) {
        ("GET", "/v1/models") => dispatch_models(aliases),
        ("POST", "/v1/chat/completions") => {
            dispatch_chat_completions(service, aliases, body, false)
        }
        ("POST", "/v1/responses") => dispatch_responses(service, aliases, body),
        _ => (404, json!({"error": "unknown openai route"})),
    }
}

/// List serving aliases as OpenAI models. Source packages are not listed.
#[must_use]
pub fn dispatch_models(aliases: &BTreeMap<String, String>) -> (u16, Value) {
    let data: Vec<Value> = aliases
        .keys()
        .map(|model| json!({"id": model, "object": "model"}))
        .collect();
    (200, json!({"object": "list", "data": data}))
}

/// JSON or SSE `/v1/chat/completions`. Messages cannot mint capabilities.
pub fn dispatch_chat_completions(
    service: &mut RuntimeService,
    aliases: &BTreeMap<String, String>,
    body: &Value,
    force_sse: bool,
) -> (u16, Value) {
    if body.get("tools").is_some() || body.get("agent").is_some() {
        return (
            400,
            json!({"error": "openai messages cannot mint capabilities"}),
        );
    }
    let stream = force_sse || body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let (status, payload) = invoke_alias(service, aliases, body);
    if status != 200 {
        return (status, payload);
    }
    if stream {
        return (
            200,
            json!({
                "object": "chat.completion.chunk",
                "id": payload.get("id"),
                "choices": [{
                    "delta": {"content": ""},
                    "finish_reason": payload.get("finish_reason")
                }],
                "sse": true
            }),
        );
    }
    (
        200,
        json!({
            "object": "chat.completion",
            "id": payload.get("id"),
            "choices": [{
                "message": {"role": "assistant", "content": ""},
                "finish_reason": payload.get("finish_reason")
            }]
        }),
    )
}

/// Dispatch `/v1/responses` against committed artifact aliases.
pub fn dispatch_responses(
    service: &mut RuntimeService,
    aliases: &BTreeMap<String, String>,
    body: &Value,
) -> (u16, Value) {
    if body.get("tools").is_some() || body.get("agent").is_some() {
        return (
            400,
            json!({"error": "openai messages cannot mint capabilities"}),
        );
    }
    invoke_alias(service, aliases, body)
}

fn invoke_alias(
    service: &mut RuntimeService,
    aliases: &BTreeMap<String, String>,
    body: &Value,
) -> (u16, Value) {
    let Some(model) = body.get("model").and_then(Value::as_str) else {
        return (400, json!({"error": "model is required"}));
    };
    let Some(digest) = aliases.get(model) else {
        return (404, json!({"error": "unknown serving alias"}));
    };
    let handshake = RuntimeHandshake {
        protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
    };
    let created = match service.handle(
        &handshake,
        RuntimeRequest::ProgramInstanceCreate {
            request_id: "openai.create".to_owned(),
            artifact_digest: digest.clone(),
        },
    ) {
        Ok(RuntimeResult::ProgramInstanceCreated {
            program_instance_id,
            owner_claim,
            ..
        }) => (program_instance_id, owner_claim),
        Ok(other) => return (400, json!({"error": format!("{other:?}")})),
        Err(error) => return (400, json!({"error": format!("{error:?}")})),
    };
    let (created, owner_claim) = created;
    match service.handle(
        &handshake,
        RuntimeRequest::ProgramInvocationStart {
            request_id: "openai.invoke".to_owned(),
            program_instance_id: created.clone(),
            owner_claim,
            input: json!({}),
        },
    ) {
        Ok(RuntimeResult::ProgramInvocationStarted { .. }) => (
            200,
            json!({
                "id": created,
                "finish_reason": "returned"
            }),
        ),
        Ok(RuntimeResult::Failed { code, .. }) => (400, json!({"error": code})),
        Ok(other) => (400, json!({"error": format!("{other:?}")})),
        Err(error) => (400, json!({"error": format!("{error:?}")})),
    }
}

/// Bind the OpenAI edge on loopback only.
pub async fn serve_loopback(
    service: RuntimeService,
    aliases: Vec<ServingAlias>,
) -> Result<(), String> {
    let mut map = BTreeMap::new();
    for alias in aliases {
        alias.validate()?;
        map.insert(alias.model, alias.artifact_digest);
    }
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .map_err(|error| error.to_string())?;
    let state = Arc::new(Mutex::new(service));
    let permits = Arc::new(Semaphore::new(MAX_IN_FLIGHT_CONNECTIONS));
    loop {
        let (stream, _) = listener.accept().await.map_err(|error| error.to_string())?;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let state = Arc::clone(&state);
        let aliases = map.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let mut stream = stream;
            let Ok(Ok(bytes)) = tokio::time::timeout(
                Duration::from_millis(HTTP_IO_TIMEOUT_MS),
                read_request(&mut stream),
            )
            .await
            else {
                return;
            };
            let Ok(request) = parse_request(&bytes) else {
                return;
            };
            let Ok(body) = serde_json::from_str::<Value>(request.body) else {
                return;
            };
            let (status, payload) = {
                let mut service = state.lock().await;
                dispatch(&mut service, &aliases, request.method, request.path, &body)
            };
            let Ok(response) = encode_response(status, &payload) else {
                return;
            };
            let _ = tokio::time::timeout(
                Duration::from_millis(HTTP_IO_TIMEOUT_MS),
                stream.write_all(response.as_bytes()),
            )
            .await;
        });
    }
}

struct HttpRequest<'a> {
    method: &'a str,
    path: &'a str,
    body: &'a str,
}

fn content_length(header: &str) -> Result<usize, String> {
    let mut length = None;
    for line in header.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            return Err("invalid HTTP header".to_owned());
        };
        if name.eq_ignore_ascii_case("transfer-encoding")
            && !value.trim().eq_ignore_ascii_case("identity")
        {
            return Err("chunked transfer encoding is not supported".to_owned());
        }
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err("duplicate content-length header".to_owned());
            }
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "invalid content-length header".to_owned())?,
            );
        }
    }
    Ok(length.unwrap_or(0))
}

fn parse_request(bytes: &[u8]) -> Result<HttpRequest<'_>, String> {
    let marker = b"\r\n\r\n";
    let header_end = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or_else(|| "HTTP headers are incomplete".to_owned())?;
    let header = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| "HTTP headers are not valid UTF-8".to_owned())?;
    let mut parts = header
        .lines()
        .next()
        .ok_or_else(|| "HTTP request line is missing".to_owned())?
        .split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "HTTP method is missing".to_owned())?;
    let path = parts
        .next()
        .ok_or_else(|| "HTTP path is missing".to_owned())?;
    let version = parts
        .next()
        .ok_or_else(|| "HTTP version is missing".to_owned())?;
    if parts.next().is_some() || !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Err("invalid HTTP request line".to_owned());
    }
    let body_len = content_length(header)?;
    let body_start = header_end + marker.len();
    let body_end = body_start
        .checked_add(body_len)
        .filter(|length| *length <= MAX_REQUEST_BYTES)
        .ok_or_else(|| "HTTP request exceeds the request-size limit".to_owned())?;
    if bytes.len() != body_end {
        return Err("HTTP request body length does not match content-length".to_owned());
    }
    let body = std::str::from_utf8(&bytes[body_start..body_end])
        .map_err(|_| "HTTP request body is not valid UTF-8".to_owned())?;
    Ok(HttpRequest { method, path, body })
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Result<Vec<u8>, String> {
    let marker = b"\r\n\r\n";
    let mut bytes = Vec::with_capacity(READ_CHUNK_BYTES);
    let header_end = loop {
        let mut chunk = [0_u8; READ_CHUNK_BYTES];
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("HTTP request ended before headers completed".to_owned());
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err("HTTP request exceeds the request-size limit".to_owned());
        }
        if let Some(end) = bytes
            .windows(marker.len())
            .position(|window| window == marker)
        {
            break end;
        }
    };
    let header = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| "HTTP headers are not valid UTF-8".to_owned())?;
    let body_len = content_length(header)?;
    let expected = (header_end + marker.len())
        .checked_add(body_len)
        .filter(|length| *length <= MAX_REQUEST_BYTES)
        .ok_or_else(|| "HTTP request exceeds the request-size limit".to_owned())?;
    if bytes.len() > expected {
        return Err("HTTP request contains bytes beyond its declared body".to_owned());
    }
    while bytes.len() < expected {
        let remaining = expected - bytes.len();
        let mut chunk = vec![0_u8; remaining.min(READ_CHUNK_BYTES)];
        stream
            .read_exact(&mut chunk)
            .await
            .map_err(|error| error.to_string())?;
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Unknown Status",
    }
}

fn encode_response(status: u16, payload: &Value) -> Result<String, String> {
    let body = serde_json::to_string(payload).map_err(|error| error.to_string())?;
    Ok(format!(
        "HTTP/1.1 {status} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        status_reason(status),
        body.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_requires_committed_artifact() {
        assert!(
            ServingAlias {
                model: "alias".to_owned(),
                artifact_digest: String::new(),
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn openai_messages_do_not_mint_capabilities() {
        assert!(messages_cannot_mint_capabilities());
    }

    fn committed_demo(service: &mut RuntimeService) -> BTreeMap<String, String> {
        let bytes = br#"{"schema_version":"apxm.air","semantic_operations":[],"structural_ir":[],"context_flow":[],"source_map":{"schema_version":"apxm.source-map","source_language":"python","node_spans":[],"region_annotations":[]}}"#
            .to_vec();
        let digest = service.admit_artifact(bytes.clone());
        service
            .bind_admission_for_artifact(
                &digest,
                apxm_runtime_service::materials_for_artifact(
                    &bytes,
                    "openai.invocation.1",
                    include_bytes!(
                        "../../../../tools/tests/fixtures/canonical-execute.release.json"
                    )
                    .to_vec(),
                    include_bytes!(
                        "../../../../tools/tests/fixtures/canonical-execute.provenance.json"
                    )
                    .to_vec(),
                ),
            )
            .expect("bind exact canonical admission fixture");
        let mut aliases = BTreeMap::new();
        aliases.insert("demo".to_owned(), digest);
        aliases
    }

    #[test]
    fn responses_bind_committed_alias() {
        let mut service = RuntimeService::default();
        let aliases = committed_demo(&mut service);
        let (status, body) = dispatch_responses(&mut service, &aliases, &json!({"model":"demo"}));
        assert_eq!(status, 200);
        assert_eq!(body["finish_reason"], "returned");
    }

    #[test]
    fn models_list_only_committed_aliases() {
        let mut service = RuntimeService::default();
        let aliases = committed_demo(&mut service);
        let (status, body) = dispatch(&mut service, &aliases, "GET", "/v1/models", &json!({}));
        assert_eq!(status, 200);
        assert_eq!(body["data"][0]["id"], "demo");
    }

    #[test]
    fn chat_completions_json_and_sse_use_the_same_runtime() {
        let mut service = RuntimeService::default();
        let aliases = committed_demo(&mut service);
        let (status, body) = dispatch(
            &mut service,
            &aliases,
            "POST",
            "/v1/chat/completions",
            &json!({"model":"demo"}),
        );
        assert_eq!(status, 200);
        assert_eq!(body["choices"][0]["finish_reason"], "returned");
        let (status, sse) = dispatch_chat_completions(
            &mut service,
            &aliases,
            &json!({"model":"demo","stream":true}),
            true,
        );
        assert_eq!(status, 200);
        assert_eq!(sse["sse"], true);
        assert_eq!(sse["choices"][0]["finish_reason"], "returned");
    }

    #[test]
    fn request_parser_requires_exact_body_framing() {
        let valid = b"POST /v1/responses HTTP/1.1\r\ncontent-length: 2\r\n\r\n{}";
        let request = parse_request(valid).expect("valid request");
        assert_eq!(request.path, "/v1/responses");
        assert_eq!(request.body, "{}");
        assert!(
            parse_request(b"POST /v1/responses HTTP/1.1\r\ncontent-length: 4\r\n\r\n{}").is_err()
        );
        assert!(
            parse_request(
                b"POST /v1/responses HTTP/1.1\r\ntransfer-encoding: chunked\r\n\r\n0\r\n\r\n"
            )
            .is_err()
        );
    }

    #[test]
    fn response_status_line_preserves_error_status() {
        let response = encode_response(404, &json!({"error": "missing"})).expect("response");
        assert!(response.starts_with("HTTP/1.1 404 Not Found\r\n"));
    }
}
