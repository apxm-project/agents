//! Google AI backend implementation.
//!
//! Implements the LLMBackend trait for Google's Gemini API.

use crate::llm::backends::traits::StreamChunk;
use crate::llm::backends::{LLMBackend, LLMRequest, LLMResponse, Role};
use anyhow::{Context, Result};
use apxm_core::constants::defaults;
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::constants::http::headers;
use apxm_core::constants::llm::{google as google_keys, roles, sse};
use apxm_core::types::{FinishReason, ModelCapabilities, ModelInfo, TokenUsage};
use apxm_core::{log_debug, log_error};
use async_trait::async_trait;
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use std::pin::Pin;
use tokio_stream::Stream;

use apxm_core::types::model_spec::{default_model_for_provider, models_for_provider};

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Google AI LLM backend.
pub struct GoogleBackend {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl GoogleBackend {
    fn request_model<'a>(&'a self, request: &'a LLMRequest) -> &'a str {
        request.model.as_deref().unwrap_or(&self.model)
    }

    /// Create a new Google backend.
    pub async fn new(api_key: &str, config: Option<serde_json::Value>) -> Result<Self> {
        let model = config
            .as_ref()
            .and_then(|c| c.get(MODEL))
            .and_then(|m| m.as_str())
            .unwrap_or_else(|| default_model_for_provider("google").unwrap_or("gemini-2.5-flash"))
            .to_string();

        let base_url = config
            .as_ref()
            .and_then(|c| c.get(BASE_URL))
            .and_then(|u| u.as_str())
            .unwrap_or(DEFAULT_BASE_URL)
            .to_string();

        Ok(GoogleBackend {
            api_key: api_key.to_string(),
            model,
            base_url,
            client: reqwest::Client::new(),
        })
    }

    /// Build request body for Google AI API.
    fn build_request_body(&self, request: &LLMRequest) -> serde_json::Value {
        let all_messages = request.resolved_messages();

        // Separate system messages for Google's systemInstruction field
        let system_parts: Vec<serde_json::Value> = all_messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| json!({ "text": m.text_content() }))
            .collect();

        // Map conversation messages to Google's contents format
        let contents: Vec<serde_json::Value> = all_messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|msg| {
                let role = match msg.role {
                    Role::User | Role::Tool => roles::USER,
                    Role::Assistant => "model",
                    Role::System => unreachable!(),
                };
                let parts: Vec<serde_json::Value> = msg
                    .content
                    .iter()
                    .map(|p| match p {
                        crate::llm::backends::ContentPart::Text { text } => {
                            json!({ "text": text })
                        }
                        crate::llm::backends::ContentPart::Image { url, .. } => {
                            json!({ "text": format!("[image: {}]", url) })
                        }
                        crate::llm::backends::ContentPart::ToolCall { function, .. } => {
                            json!({ "text": format!("[tool_call: {}]", function.name) })
                        }
                    })
                    .collect();
                json!({ "role": role, "parts": parts })
            })
            .collect();

        let mut body = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": request.temperature,
                "maxOutputTokens": request.max_tokens.unwrap_or(defaults::DEFAULT_GOOGLE_MAX_OUTPUT_TOKENS),
                "topP": request.top_p.unwrap_or(defaults::DEFAULT_GOOGLE_TOP_P),
                "stopSequences": request.stop_sequences,
            }
        });

        if !system_parts.is_empty()
            && let Some(obj) = body.as_object_mut()
        {
            obj.insert(
                "systemInstruction".to_string(),
                json!({ "parts": system_parts }),
            );
        }

        body
    }

    /// Parse Google API response.
    fn parse_response(&self, response: GoogleResponse, model: &str) -> Result<LLMResponse> {
        let candidate = response
            .candidates
            .first()
            .context("No candidates in response")?;

        let part = candidate
            .content
            .parts
            .first()
            .context("No parts in content")?;

        let text = part.text.clone();
        let finish_reason = FinishReason::from_string(&candidate.finish_reason);

        // Google doesn't provide detailed token usage in all responses
        let usage = TokenUsage::new(0, text.split_whitespace().count());

        Ok(LLMResponse::new(text, model, usage, finish_reason))
    }
}

#[async_trait]
impl LLMBackend for GoogleBackend {
    async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        request.validate()?;

        let model = self.request_model(&request).to_string();
        let body = self.build_request_body(&request);
        let url = format!("{}/models/{}:generateContent", self.base_url, model);

        log_debug!(
            "models::google",
            model = %model,
            "Sending request to Google AI"
        );

        let response = self
            .client
            .post(&url)
            .header(headers::CONTENT_TYPE, headers::CONTENT_TYPE_JSON)
            .header(headers::X_GOOG_API_KEY, &self.api_key)
            .json(&body)
            .send()
            .await
            .context("Failed to send request to Google")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            log_error!(
                "models::google",
                status = %status,
                body = %error_text,
                model = %model,
                "Google API request failed"
            );
            anyhow::bail!("Google API error (status {}): {}", status, error_text);
        }

        let api_response: GoogleResponse = response
            .json()
            .await
            .context("Failed to parse Google response")?;

        self.parse_response(api_response, &model)
    }

    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + '_>> {
        Box::pin(async_stream::try_stream! {
            request.validate()?;
            let model = self.request_model(&request).to_string();
            let body = self.build_request_body(&request);
            // Google uses the streamGenerateContent endpoint with alt=sse.
            let url = format!(
                "{}/models/{}:streamGenerateContent?alt=sse",
                self.base_url, model
            );

            log_debug!(
                "models::google",
                model = %model,
                "Sending streaming request to Google AI"
            );

            let response = self.client
                .post(&url)
                .header(headers::CONTENT_TYPE, headers::CONTENT_TYPE_JSON)
                .header(headers::X_GOOG_API_KEY, &self.api_key)
                .json(&body)
                .send()
                .await
                .context("Failed to send streaming request to Google")?
                .error_for_status()
                .map_err(|e| {
                    log_error!(
                        "models::google",
                        error = %e,
                        model = %model,
                        "Google streaming API request failed"
                    );
                    anyhow::anyhow!("Google API error: {}", e)
                })?;

            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut full_content = String::new();
            let mut last_usage = TokenUsage::new(0, 0);

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result.context("Stream read error")?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process SSE lines.
                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim_end().to_string();
                    buffer.drain(..line_end + 1);
                    let line = line.trim();

                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }

                    if let Some(data) = line.strip_prefix(sse::DATA_PREFIX) {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                            // Extract text from candidates[].content.parts[].text
                            if let Some(candidates) = parsed["candidates"].as_array() {
                                for candidate in candidates {
                                    if let Some(parts) = candidate["content"]["parts"].as_array() {
                                        for part in parts {
                                            if let Some(text) = part["text"].as_str() {
                                                if !text.is_empty() {
                                                    full_content.push_str(text);
                                                    yield StreamChunk::Token(text.to_string());
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Extract usage metadata if present.
                            if let Some(usage_meta) = parsed.get(google_keys::USAGE_METADATA) {
                                let input = usage_meta[google_keys::PROMPT_TOKEN_COUNT]
                                    .as_u64().unwrap_or(0) as usize;
                                let output = usage_meta[google_keys::CANDIDATES_TOKEN_COUNT]
                                    .as_u64().unwrap_or(0) as usize;
                                if input > 0 || output > 0 {
                                    last_usage = TokenUsage::new(input, output);
                                    yield StreamChunk::Usage(last_usage.clone());
                                }
                            }
                        }
                    }
                }
            }

            // Emit final Done with last known usage.
            let resp = LLMResponse::new(
                full_content,
                &model,
                last_usage,
                FinishReason::Stop,
            );
            yield StreamChunk::Done(resp);
        })
    }

    fn name(&self) -> &str {
        "google"
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn health_check(&self) -> Result<()> {
        let url = format!("{}/models/{}", self.base_url, self.model);

        let response = self
            .client
            .get(&url)
            .header(headers::X_GOOG_API_KEY, &self.api_key)
            .send()
            .await
            .context("Failed to connect to Google")?;

        if !response.status().is_success() {
            anyhow::bail!("Google health check failed: {}", response.status());
        }

        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(models_for_provider("google")
            .map(|m| ModelInfo {
                id: m.id.to_string(),
                name: m.id.to_string(),
                context_window: 1_000_000,
                supports_vision: true,
                supports_functions: true,
            })
            .collect())
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            streaming: true,
            vision: true,
            functions: true,
            structured_outputs: false,
            batch: false,
            fine_tuning: false,
        }
    }
}

// Google API response types
#[derive(Debug, Deserialize)]
struct GoogleResponse {
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: Content,
    #[serde(rename = "finishReason")]
    finish_reason: String,
}

#[derive(Debug, Deserialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Debug, Deserialize)]
struct Part {
    text: String,
}
