//! Inbound OpenAI HTTP protocol adapter over the Runtime Service.
//!
//! Serving aliases bind committed artifacts only. This crate shares no DTO with
//! retired `apxm chat`.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use apxm_runtime_protocol::{
    RUNTIME_PROTOCOL_VERSION, RuntimeHandshake, RuntimeRequest, RuntimeResult,
};
use apxm_runtime_service::RuntimeService;

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
    let Some(model) = body.get("model").and_then(Value::as_str) else {
        return (400, json!({"error": "model is required"}));
    };
    let Some(digest) = aliases.get(model) else {
        return (404, json!({"error": "unknown serving alias"}));
    };
    match service.handle(
        &RuntimeHandshake {
            protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
        },
        RuntimeRequest::ProgramInstanceCreate {
            request_id: "openai.create".to_owned(),
            artifact_digest: digest.clone(),
        },
    ) {
        Ok(RuntimeResult::ProgramInstanceCreated {
            program_instance_id,
            ..
        }) => (
            200,
            json!({
                "id": program_instance_id,
                "finish_reason": "returned"
            }),
        ),
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
    let state = Mutex::new(service);
    loop {
        let (mut stream, _) = listener.accept().await.map_err(|error| error.to_string())?;
        let mut buf = vec![0_u8; 8192];
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|error| error.to_string())?;
        let request = String::from_utf8_lossy(&buf[..n]);
        let body = request
            .rsplit("\r\n\r\n")
            .next()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .unwrap_or_else(|| json!({}));
        let mut service = state.lock().await;
        let (status, payload) = dispatch_responses(&mut service, &map, &body);
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

    #[test]
    fn responses_bind_committed_alias() {
        let mut aliases = BTreeMap::new();
        aliases.insert("demo".to_owned(), "artifact:abc".to_owned());
        let mut service = RuntimeService::default();
        let (status, body) = dispatch_responses(&mut service, &aliases, &json!({"model":"demo"}));
        assert_eq!(status, 200);
        assert_eq!(body["finish_reason"], "returned");
    }
}
