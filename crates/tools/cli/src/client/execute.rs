//! Execute and conversation-message client paths outside the generated
//! session client.

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use reqwest::Response;
use serde::{Deserialize, Serialize};

use super::{Client, ClientInfo};

/// `POST /v1/execute/stream` body (subset used by thin clients).
#[derive(Debug, Clone, Serialize, Default)]
pub struct ExecuteRequest {
    pub air: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_grant_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tool_call_budgets: HashMap<String, usize>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tool_credentials: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_text: Option<String>,
}

/// `POST /v1/agents/{id}/sessions` request body.
#[derive(Debug, Clone, Serialize, Default)]
pub struct CreateAgentSessionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// `POST /v1/agents/{id}/sessions` response.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateAgentSessionResponse {
    pub session_id: String,
    pub events_url: String,
    pub stream_url: String,
}

impl Client {
    /// `POST /v1/capability-grants` — mint a runtime grant for a tool binding.
    pub async fn mint_capability_grant(&self, capability_binding: &str) -> Result<String> {
        let url = format!("{}/v1/capability-grants", self.baseurl());
        let resp = self
            .client()
            .post(&url)
            .json(&serde_json::json!({ "capability_binding": capability_binding }))
            .send()
            .await
            .with_context(|| format!("failed to POST {url}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("mint returned {status} for {url}: {text}"));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .with_context(|| format!("bad mint JSON from {url}"))?;
        body.get("grant_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("mint response missing grant_id from {url}"))
    }

    /// Map tool bindings to minted `grant_*` ids; pass through existing grant ids.
    pub async fn resolve_capability_grant_ids(&self, ids: &[String]) -> Result<Vec<String>> {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if id.starts_with("grant_") {
                out.push(id.clone());
            } else {
                out.push(self.mint_capability_grant(id).await?);
            }
        }
        Ok(out)
    }

    /// `POST /v1/capability-grants/{grant_id}/revoke` — revoke a minted grant.
    pub async fn revoke_capability_grant(&self, grant_id: &str) -> Result<serde_json::Value> {
        let url = format!("{}/v1/capability-grants/{grant_id}/revoke", self.baseurl());
        let resp = self
            .client()
            .post(&url)
            .json(&serde_json::json!({}))
            .send()
            .await
            .with_context(|| format!("failed to POST {url}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("revoke returned {status} for {url}: {text}"));
        }
        resp.json()
            .await
            .with_context(|| format!("bad revoke JSON from {url}"))
    }

    /// `POST /v1/agents/{agent_id}/sessions` — start a thin agent chat session.
    pub async fn create_agent_session(
        &self,
        agent_id: &str,
        req: &CreateAgentSessionRequest,
    ) -> Result<CreateAgentSessionResponse> {
        let url = format!("{}/v1/agents/{agent_id}/sessions", self.baseurl());
        let resp = self
            .client()
            .post(&url)
            .json(req)
            .send()
            .await
            .with_context(|| format!("failed to POST {url}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("agent session returned {status} for {url}: {text}"));
        }
        resp.json()
            .await
            .with_context(|| format!("bad agent session JSON from {url}"))
    }

    /// `POST /v1/execute/stream` — raw SSE response for the caller to render.
    pub async fn execute_stream(&self, req: &ExecuteRequest) -> Result<Response> {
        let url = format!("{}/v1/execute/stream", self.baseurl());
        self.client()
            .post(&url)
            .header("Accept", "text/event-stream")
            .json(req)
            .send()
            .await
            .with_context(|| format!("failed to POST {url}"))
    }

    /// `POST /v1/conversations/{session_id}/message`
    pub async fn post_conversation_message(
        &self,
        session_id: &str,
        message: &str,
    ) -> Result<Response> {
        let url = format!("{}/v1/conversations/{session_id}/message", self.baseurl());
        self.client()
            .post(&url)
            .json(&serde_json::json!({ "message": message }))
            .send()
            .await
            .with_context(|| format!("failed to POST {url}"))
    }

    /// `GET` helper for discovery list endpoints.
    pub async fn get_json(&self, path: &str) -> Result<serde_json::Value> {
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", self.baseurl(), path)
        };
        let resp = self
            .client()
            .get(&url)
            .send()
            .await
            .with_context(|| format!("failed to GET {url}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("GET returned {status} for {url}: {text}"));
        }
        resp.json()
            .await
            .with_context(|| format!("invalid JSON from {url}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// `apxm chat`'s in-program-loop path (`run_dumb_pipe`, `chat.rs`) and
    /// apxm-os's channel-chat convergence target both name
    /// `POST /v1/conversations/{session_id}/message` as the shared
    /// message-input
    /// primitive (`server/crates/core/src/conversations.rs`,
    /// `CONVERSATION_MESSAGE` route constant). This pins the CLI side of that
    /// contract: the exact path shape and body — so a change to either drifts
    /// loudly instead of silently.
    #[tokio::test]
    async fn post_conversation_message_hits_the_shared_message_input_route() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .await
                .unwrap();
            request
        });

        let client = Client::new(&format!("http://{addr}"));
        let resp = client
            .post_conversation_message("sess-abc123", "hello there")
            .await
            .expect("request should succeed against the stub server");
        assert!(resp.status().is_success());

        let request = server.await.unwrap();
        let request_line = request.lines().next().unwrap_or_default();
        assert_eq!(
            request_line, "POST /v1/conversations/sess-abc123/message HTTP/1.1",
            "apxm chat must address the session-scoped message-input route, not a bespoke path"
        );
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        let parsed: serde_json::Value =
            serde_json::from_str(body.trim_end_matches('\0')).expect("body must be JSON");
        assert_eq!(
            parsed,
            serde_json::json!({ "message": "hello there" }),
            "wire body must match ConversationMessageRequest {{ message }} on the server"
        );
    }

    /// Session creation uses the same client as the long-lived event stream.
    /// A fully framed HTTP/1.1 response remains readable even when the server
    /// sends headers before the body; the streaming client must not install an
    /// immediate zero-duration read deadline on this request.
    #[cfg(feature = "driver")]
    #[tokio::test]
    async fn create_agent_session_reads_delayed_framed_http_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0_u8; 4096];
            let _ = sock.read(&mut buf).await.unwrap();
            let body = r#"{"session_id":"sess-framed","events_url":"/events","stream_url":"/stream","execution_id":"exec-framed"}"#;
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(headers.as_bytes()).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            sock.write_all(body.as_bytes()).await.unwrap();
        });

        let client = crate::client::client_for_sse(&format!("http://{addr}"));
        let response = client
            .create_agent_session(
                "reference-agent",
                &CreateAgentSessionRequest {
                    session_id: Some("sess-framed".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("framed session response must be readable");

        assert_eq!(response.session_id, "sess-framed");
        assert_eq!(response.stream_url, "/stream");
        server.await.unwrap();
    }
}
