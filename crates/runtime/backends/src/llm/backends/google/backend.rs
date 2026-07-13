//! Google AI backend implementation.
//!
//! Implements the LLMBackend trait for Google's Gemini API.

use crate::llm::ProviderProtocol;
use crate::llm::backends::http::llm_http_client;
use crate::llm::backends::openai::backend::validate_provider_dispatch;
use crate::llm::backends::traits::StreamChunk;
use crate::llm::backends::{
    ConfiguredModelCapabilities, configured_model_capabilities, configured_model_info,
    required_config_string,
};
use crate::llm::backends::{LLMBackend, LLMRequest, LLMResponse, Role};
use crate::llm::wire::{defaults as wire_defaults, google as google_keys, headers, roles, sse};
use anyhow::{Context, Result};
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::types::{FinishReason, ModelCapabilities, ModelInfo, TokenUsage};
use apxm_core::{log_debug, log_error};
use async_trait::async_trait;
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::pin::Pin;
use tokio_stream::Stream;

const PROTOCOL: ProviderProtocol = ProviderProtocol::Google;

/// Google AI LLM backend.
pub struct GoogleBackend {
    api_key: String,
    model: String,
    base_url: String,
    /// Registered per-model capability evidence.
    model_capabilities: HashMap<String, ConfiguredModelCapabilities>,
    /// Registered model metadata exposed through backend inspection.
    registered_models: Vec<ModelInfo>,
    client: reqwest::Client,
}

impl GoogleBackend {
    fn configured_capabilities(&self, model: &str) -> ConfiguredModelCapabilities {
        self.model_capabilities
            .get(model)
            .copied()
            .unwrap_or_default()
    }

    fn request_model<'a>(&'a self, request: &'a LLMRequest) -> &'a str {
        request.model.as_deref().unwrap_or(&self.model)
    }

    /// Create a new Google backend.
    pub async fn new(api_key: &str, config: Option<serde_json::Value>) -> Result<Self> {
        let model = required_config_string(config.as_ref(), PROTOCOL, MODEL)?;
        let base_url = required_config_string(config.as_ref(), PROTOCOL, BASE_URL)?;
        let model_capabilities = configured_model_capabilities(config.as_ref());
        let registered_models = configured_model_info(config.as_ref());

        Ok(GoogleBackend {
            api_key: api_key.to_string(),
            model,
            base_url,
            model_capabilities,
            registered_models,
            client: llm_http_client(),
        })
    }

    /// Build request body for Google AI API.
    fn build_request_body(request: &LLMRequest) -> serde_json::Value {
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
                "maxOutputTokens": request.max_tokens.unwrap_or(wire_defaults::GOOGLE_MAX_OUTPUT_TOKENS),
                "topP": request.top_p.unwrap_or(wire_defaults::GOOGLE_TOP_P),
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
    fn parse_response(response: GoogleResponse, model: &str) -> Result<LLMResponse> {
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

        let usage = response
            .usage_metadata
            .context("Google response omitted observed usage metadata")?
            .token_usage()?;

        Ok(LLMResponse::new(text, model, usage, finish_reason))
    }
}

#[async_trait]
impl LLMBackend for GoogleBackend {
    async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        validate_provider_dispatch(&request)?;

        let model = self.request_model(&request).to_string();
        let body = Self::build_request_body(&request);
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

        Self::parse_response(api_response, &model)
    }

    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + '_>> {
        Box::pin(async_stream::try_stream! {
            validate_provider_dispatch(&request)?;
            let model = self.request_model(&request).to_string();
            let body = Self::build_request_body(&request);
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
            let mut last_usage = None;

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result.context("Stream read error")?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process SSE lines.
                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim_end().to_string();
                    buffer.drain(..=line_end);
                    let line = line.trim();

                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }

                    if let Some(data) = line.strip_prefix(sse::DATA_PREFIX)
                        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                            // Extract text from candidates[].content.parts[].text
                            if let Some(candidates) = parsed["candidates"].as_array() {
                                for candidate in candidates {
                                    if let Some(parts) = candidate["content"]["parts"].as_array() {
                                        for part in parts {
                                            if let Some(text) = part["text"].as_str()
                                                && !text.is_empty() {
                                                    full_content.push_str(text);
                                                    yield StreamChunk::Token(text.to_string());
                                                }
                                        }
                                    }
                                }
                            }

                            // Extract usage metadata if present.
                            if let Some(usage) = google_stream_usage(&parsed)? {
                                last_usage = Some(usage.clone());
                                yield StreamChunk::Usage(usage);
                            }
                        }
                }
            }

            let usage = last_usage.context("Google stream completed without observed usage metadata")?;
            let resp = LLMResponse::new(
                full_content,
                &model,
                usage,
                FinishReason::Stop,
            );
            yield StreamChunk::Done(resp);
        })
    }

    fn name(&self) -> &str {
        PROTOCOL.as_str()
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn context_window_for_model(&self, model: &str) -> Option<usize> {
        let context_window = self.configured_capabilities(model).context_window;
        (context_window > 0).then_some(context_window)
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
        Ok(self.registered_models.clone())
    }

    fn capabilities(&self) -> ModelCapabilities {
        let capabilities = self.configured_capabilities(&self.model);
        ModelCapabilities {
            streaming: true,
            vision: capabilities.supports_vision,
            functions: capabilities.supports_functions,
            structured_outputs: capabilities.supports_structured_outputs.unwrap_or(false),
            batch: false,
            fine_tuning: capabilities.supports_fine_tuning,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn registered_model_capabilities_control_google_inspection() {
        let backend = GoogleBackend::new(
            "",
            Some(json!({
                "model": "gateway-model",
                "base_url": "https://llm.example.test/google",
                "models": [{
                    "id": "gateway-model",
                    "context_window": 49152,
                    "supports_vision": false,
                    "supports_functions": false,
                    "supports_fine_tuning": true,
                    "supports_structured_outputs": true
                }]
            })),
        )
        .await
        .expect("registered backend config");

        let capabilities = backend.capabilities();
        assert!(!capabilities.vision);
        assert!(!capabilities.functions);
        assert!(capabilities.structured_outputs);
        assert!(capabilities.fine_tuning);

        let models = backend.list_models().await.expect("registered models");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gateway-model");
        assert_eq!(models[0].context_window, 49152);
    }

    #[test]
    fn google_response_requires_observed_usage_metadata() {
        let response: GoogleResponse = serde_json::from_value(json!({
            "candidates": [{
                "content": {"parts": [{"text": "answer"}]},
                "finishReason": "STOP"
            }]
        }))
        .expect("response fixture");

        let error = GoogleBackend::parse_response(response, "fixture-model")
            .expect_err("Google responses without usage are not telemetry");
        assert!(
            error
                .to_string()
                .contains("omitted observed usage metadata")
        );
    }
}

// Google API response types
#[derive(Debug, Deserialize)]
struct GoogleResponse {
    candidates: Vec<Candidate>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<GoogleUsageMetadata>,
}

#[derive(Debug, Deserialize)]
struct GoogleUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: Option<usize>,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: Option<usize>,
}

impl GoogleUsageMetadata {
    fn token_usage(&self) -> Result<TokenUsage> {
        let input = self
            .prompt_token_count
            .context("Google usage metadata omitted promptTokenCount")?;
        let output = self
            .candidates_token_count
            .context("Google usage metadata omitted candidatesTokenCount")?;
        Ok(TokenUsage::new(input, output))
    }
}

fn google_stream_usage(value: &serde_json::Value) -> Result<Option<TokenUsage>> {
    let Some(usage) = value.get(google_keys::USAGE_METADATA) else {
        return Ok(None);
    };
    let input = usage
        .get(google_keys::PROMPT_TOKEN_COUNT)
        .and_then(serde_json::Value::as_u64)
        .context("Google stream usage metadata omitted promptTokenCount")?;
    let output = usage
        .get(google_keys::CANDIDATES_TOKEN_COUNT)
        .and_then(serde_json::Value::as_u64)
        .context("Google stream usage metadata omitted candidatesTokenCount")?;
    Ok(Some(TokenUsage::new(
        usize::try_from(input).context("Google stream input token count exceeds platform size")?,
        usize::try_from(output)
            .context("Google stream output token count exceeds platform size")?,
    )))
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
