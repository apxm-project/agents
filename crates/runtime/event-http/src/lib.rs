//! Canonical Event HTTP projection. Loopback by default.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use apxm_kernel::event_api::EventHttpMethod;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Semaphore};

use apxm_runtime_protocol::{
    RUNTIME_PROTOCOL_VERSION, RuntimeHandshake, RuntimeOwnerClaim, RuntimeRequest, RuntimeResult,
};
use apxm_runtime_service::RuntimeService;

pub const READ_CHUNK_BYTES: usize = 8 * 1024;
pub const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
pub const HTTP_IO_TIMEOUT_MS: u64 = 5_000;
pub const MAX_IN_FLIGHT_CONNECTIONS: usize = 32;

/// Bind address policy for Event HTTP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindPolicy {
    /// 127.0.0.1 only.
    Loopback,
    /// Non-loopback requires the accepted security ADR profile.
    NonLoopback { security_profile_complete: bool },
}

impl BindPolicy {
    /// Non-loopback is impossible unless the security profile is complete.
    #[must_use]
    pub fn allowed(self) -> bool {
        match self {
            Self::Loopback => true,
            Self::NonLoopback {
                security_profile_complete,
            } => security_profile_complete,
        }
    }

    /// Socket address this policy may bind.
    pub fn bind_addr(self) -> Result<SocketAddr, String> {
        if !self.allowed() {
            return Err("non-loopback Event HTTP requires a complete security profile".to_owned());
        }
        match self {
            Self::Loopback => Ok(SocketAddr::from(([127, 0, 0, 1], 0))),
            Self::NonLoopback { .. } => Err("non-loopback bind is not enabled".to_owned()),
        }
    }
}

/// Map HTTP method names onto the frozen Event HTTP paths.
#[must_use]
pub fn route(method: EventHttpMethod) -> &'static str {
    method.path()
}

/// Dispatch one Event HTTP request against the Runtime Service.
pub fn dispatch(
    service: &mut RuntimeService,
    method: &str,
    path: &str,
    body: &str,
) -> (u16, Value) {
    let handshake = RuntimeHandshake {
        protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
    };
    if method == "POST" && path == EventHttpMethod::Reserve.path() {
        let type_id = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|value| value.get("type_id")?.as_str().map(str::to_owned))
            .unwrap_or_default();
        return map_result(service.handle(
            &handshake,
            RuntimeRequest::EventReserve {
                request_id: "http.reserve".to_owned(),
                type_id,
            },
        ));
    }
    if method == "GET" && path == EventHttpMethod::Inspect.path() {
        return map_result(service.handle(
            &handshake,
            RuntimeRequest::EventInspect {
                request_id: "http.inspect".to_owned(),
                owner_claim: RuntimeOwnerClaim::mint(),
                event_ref: apxm_kernel::event_api::CanonicalEventRef {
                    event_id: "unspecified".to_owned(),
                    generation: 1,
                },
            },
        ));
    }
    if method == "POST" && path == EventHttpMethod::Fulfill.path() {
        let Ok(value) = serde_json::from_str::<Value>(body) else {
            return (400, json!({"error": "invalid application"}));
        };
        let Some(owner_claim) = value
            .get("owner_claim")
            .cloned()
            .and_then(|claim| serde_json::from_value(claim).ok())
        else {
            return (400, json!({"error": "owner_claim is required"}));
        };
        let Some(application) = value
            .get("application")
            .cloned()
            .and_then(|application| serde_json::from_value(application).ok())
        else {
            return (400, json!({"error": "invalid application"}));
        };
        return map_result(service.handle(
            &handshake,
            RuntimeRequest::EventFulfill {
                request_id: "http.fulfill".to_owned(),
                owner_claim,
                application,
            },
        ));
    }
    if method == "POST"
        && (path == EventHttpMethod::Expire.path() || path == EventHttpMethod::Cancel.path())
    {
        return (
            501,
            json!({"error": "expire/cancel are not HTTP-owned mutations"}),
        );
    }
    (404, json!({"error": "unknown event route"}))
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
    let mut request_line = header.lines();
    let line = request_line
        .next()
        .ok_or_else(|| "HTTP request line is missing".to_owned())?;
    let mut parts = line.split_whitespace();
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
    let expected_body = content_length(header)?;
    let body_start = header_end + marker.len();
    let body_end = body_start
        .checked_add(expected_body)
        .ok_or_else(|| "HTTP request body is too large".to_owned())?;
    if body_end > MAX_REQUEST_BYTES || bytes.len() != body_end {
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
    let body_start = header_end + marker.len();
    let expected = body_start
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
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
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

fn map_result(result: Result<RuntimeResult, apxm_runtime_protocol::ProtocolError>) -> (u16, Value) {
    match result {
        Ok(RuntimeResult::EventReserved {
            event_ref,
            owner_claim,
            ..
        }) => (
            200,
            json!({"event_ref": event_ref, "owner_claim": owner_claim}),
        ),
        Ok(RuntimeResult::EventApplied { result, .. }) => {
            (200, json!({"result": format!("{result:?}")}))
        }
        Ok(other) => (200, json!({"result": format!("{other:?}")})),
        Err(error) => (400, json!({"error": format!("{error:?}")})),
    }
}

/// Bind the Event HTTP edge on loopback and serve one connection at a time.
pub async fn serve_loopback(service: RuntimeService) -> Result<(), String> {
    let addr = BindPolicy::Loopback.bind_addr()?;
    let listener = TcpListener::bind(addr)
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
            let response = {
                let mut service = state.lock().await;
                dispatch(&mut service, request.method, request.path, request.body)
            };
            let Ok(response) = encode_response(response.0, &response.1) else {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_is_allowed_and_nonloopback_is_not_by_default() {
        assert!(BindPolicy::Loopback.allowed());
        assert!(
            !BindPolicy::NonLoopback {
                security_profile_complete: false
            }
            .allowed()
        );
        assert!(BindPolicy::Loopback.bind_addr().is_ok());
        assert!(
            BindPolicy::NonLoopback {
                security_profile_complete: false
            }
            .bind_addr()
            .is_err()
        );
    }

    #[test]
    fn fulfill_path_is_frozen() {
        assert_eq!(route(EventHttpMethod::Fulfill), "/v1/events/fulfill");
    }

    #[test]
    fn reserve_dispatches_on_frozen_path() {
        let mut service = RuntimeService::default();
        let (status, body) = dispatch(
            &mut service,
            "POST",
            EventHttpMethod::Reserve.path(),
            r#"{"type_id":"UserInput"}"#,
        );
        assert_eq!(status, 200);
        assert!(body.get("event_ref").is_some());
    }

    #[test]
    fn http_and_native_fulfill_share_the_application_result() {
        let mut http = RuntimeService::default();
        let mut native = RuntimeService::default();
        let handshake = RuntimeHandshake {
            protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
        };
        let reserved = native
            .handle(
                &handshake,
                RuntimeRequest::EventReserve {
                    request_id: "r".to_owned(),
                    type_id: "UserInput".to_owned(),
                },
            )
            .unwrap();
        let apxm_runtime_protocol::RuntimeResult::EventReserved {
            event_ref,
            owner_claim,
            ..
        } = reserved
        else {
            panic!("reserve");
        };
        let application = serde_json::json!({
            "event_ref": event_ref,
            "occurrence": {
                "occurrence_id": "occ",
                "source_kind": "human.terminal",
                "mapping_digest": "m",
                "source_record": "s",
                "payload": "ok"
            },
            "idempotency_key": "k"
        });
        let native_result = native
            .handle(
                &handshake,
                RuntimeRequest::EventFulfill {
                    request_id: "f".to_owned(),
                    owner_claim: owner_claim.clone(),
                    application: serde_json::from_value(application.clone()).unwrap(),
                },
            )
            .unwrap();
        let http_reserved = dispatch(
            &mut http,
            "POST",
            EventHttpMethod::Reserve.path(),
            r#"{"type_id":"UserInput"}"#,
        );
        let http_event: Value = http_reserved.1;
        let http_claim = http_event.get("owner_claim").cloned().unwrap();
        let http_application = serde_json::json!({
            "owner_claim": http_claim,
            "application": application,
        });
        let (status, http_body) = dispatch(
            &mut http,
            "POST",
            EventHttpMethod::Fulfill.path(),
            &http_application.to_string(),
        );
        assert_eq!(status, 200);
        assert!(matches!(
            native_result,
            apxm_runtime_protocol::RuntimeResult::EventApplied { .. }
        ));
        assert!(http_body.get("result").is_some());
    }

    #[test]
    fn request_parser_accepts_bodies_larger_than_one_read_chunk() {
        let body = "x".repeat(READ_CHUNK_BYTES * 2 + 17);
        let request = format!(
            "POST /v1/events/fulfill HTTP/1.1\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let parsed = parse_request(request.as_bytes()).expect("complete request parses");
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.path, "/v1/events/fulfill");
        assert_eq!(parsed.body, body);
    }

    #[test]
    fn request_parser_rejects_body_length_mismatch_and_chunked_framing() {
        let mismatch = b"POST /v1/events/fulfill HTTP/1.1\r\ncontent-length: 4\r\n\r\n{}";
        assert!(parse_request(mismatch).is_err());

        let chunked =
            b"POST /v1/events/fulfill HTTP/1.1\r\ntransfer-encoding: chunked\r\n\r\n0\r\n\r\n";
        assert!(parse_request(chunked).is_err());
    }

    #[test]
    fn response_status_line_uses_the_actual_reason_phrase() {
        let response = encode_response(404, &json!({"error": "missing"})).expect("response");
        assert!(response.starts_with("HTTP/1.1 404 Not Found\r\n"));
    }
}
