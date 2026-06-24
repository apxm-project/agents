//! Hand-rolled execute/turn paths not yet in the session OpenAPI contract.

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use reqwest::Response;
use serde::Serialize;

use crate::{Client, ClientInfo};

/// `POST /v1/execute/stream` body (subset used by thin clients).
#[derive(Debug, Clone, Serialize, Default)]
pub struct ExecuteRequest {
    pub air: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delegated_capability_ids: Vec<String>,
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

impl Client {
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
