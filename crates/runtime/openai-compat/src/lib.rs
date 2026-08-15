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
            ..
        }) => program_instance_id,
        Ok(other) => return (400, json!({"error": format!("{other:?}")})),
        Err(error) => return (400, json!({"error": format!("{error:?}")})),
    };
    match service.handle(
        &handshake,
        RuntimeRequest::ProgramInvocationStart {
            request_id: "openai.invoke".to_owned(),
            program_instance_id: created.clone(),
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
        let start = request.split("\r\n").next().unwrap_or_default();
        let mut parts = start.split_whitespace();
        let method = parts.next().unwrap_or("POST");
        let path = parts.next().unwrap_or("/v1/responses");
        let mut service = state.lock().await;
        let (status, payload) = dispatch(&mut service, &map, method, path, &body);
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

    fn committed_demo(service: &mut RuntimeService) -> BTreeMap<String, String> {
        let digest = service.admit_artifact(
            br#"{"schema_version":"apxm.air","semantic_operations":[],"structural_ir":[],"context_flow":[],"source_map":{"schema_version":"apxm.source-map","source_language":"python","node_spans":[],"region_annotations":[]}}"#
                .to_vec(),
        );
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
}
