//! Canonical Event HTTP projection. Loopback by default.

use std::net::SocketAddr;

use apxm_kernel::event_api::EventHttpMethod;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use apxm_runtime_protocol::{
    RUNTIME_PROTOCOL_VERSION, RuntimeHandshake, RuntimeRequest, RuntimeResult,
};
use apxm_runtime_service::RuntimeService;

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
                event_ref: apxm_kernel::event_api::CanonicalEventRef {
                    event_id: "unspecified".to_owned(),
                    generation: 1,
                },
            },
        ));
    }
    if method == "POST" && path == EventHttpMethod::Fulfill.path() {
        let Ok(application) = serde_json::from_str(body) else {
            return (400, json!({"error": "invalid application"}));
        };
        return map_result(service.handle(
            &handshake,
            RuntimeRequest::EventFulfill {
                request_id: "http.fulfill".to_owned(),
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

fn map_result(result: Result<RuntimeResult, apxm_runtime_protocol::ProtocolError>) -> (u16, Value) {
    match result {
        Ok(RuntimeResult::EventReserved { event_ref, .. }) => {
            (200, json!({"event_ref": event_ref}))
        }
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
    let state = Mutex::new(service);
    loop {
        let (mut stream, _) = listener.accept().await.map_err(|error| error.to_string())?;
        let mut buf = vec![0_u8; 8192];
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|error| error.to_string())?;
        let request = String::from_utf8_lossy(&buf[..n]);
        let mut lines = request.split("\r\n");
        let start = lines.next().unwrap_or_default();
        let mut parts = start.split_whitespace();
        let method = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("");
        let body = request.rsplit("\r\n\r\n").next().unwrap_or("");
        let mut service = state.lock().await;
        let (status, payload) = dispatch(&mut service, method, path, body);
        let body = serde_json::to_string(&payload).map_err(|error| error.to_string())?;
        let response = format!(
            "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .map_err(|error| error.to_string())?;
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
}
