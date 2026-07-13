//! Anthropic backend implementation.
//!
//! Implements the LLMBackend trait for Anthropic's Claude API using explicit
//! registered models and endpoints.

use crate::llm::ProviderProtocol;
use crate::llm::backends::http::llm_http_client;
use crate::llm::backends::openai::backend::validate_provider_dispatch;
use crate::llm::backends::traits::StreamChunk;
use crate::llm::backends::{
    ConfiguredModelCapabilities, configured_model_capabilities, configured_model_info,
    required_config_string, resolve_configured_value,
};
use crate::llm::backends::{ContentPart, LLMBackend, LLMRequest, LLMResponse, Role, ToolChoice};
use crate::llm::wire::{
    anthropic_events, api_paths, config_keys, defaults as wire_defaults, headers, message_keys,
    roles, sse, tool_keys,
};
use anyhow::{Context, Result};
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::log_debug;
use apxm_core::types::{FinishReason, ModelCapabilities, ModelInfo, TokenUsage, ToolCall};
use async_trait::async_trait;
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::pin::Pin;
use tokio_stream::Stream;

const PROTOCOL: ProviderProtocol = ProviderProtocol::Anthropic;
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic LLM backend.
///
/// Supports Anthropic's Claude API including on-premises endpoints.
/// Extra headers (e.g. `X-Custom-Gateway-Key`) can be injected via the
/// `extra_headers` config key:
///
/// ```toml
/// [[backends]]
/// name = "my-anthropic"
/// protocol = "anthropic"
/// endpoint = "https://llm.example.com/anthropic"
/// api_key = "env:ANTHROPIC_API_KEY"
///
/// [backends.headers]
/// X-Custom-Gateway-Key = "env:LLM_GATEWAY_KEY"
/// ```
pub struct AnthropicBackend {
    api_key: String,
    model: String,
    base_url: String,
    /// Additional HTTP headers injected on every request.
    extra_headers: Vec<(String, String)>,
    /// Registered per-model capability evidence.
    model_capabilities: HashMap<String, ConfiguredModelCapabilities>,
    /// Registered model metadata exposed through backend inspection.
    registered_models: Vec<ModelInfo>,
    client: reqwest::Client,
}

impl AnthropicBackend {
    fn configured_capabilities(&self, model: &str) -> ConfiguredModelCapabilities {
        self.model_capabilities
            .get(model)
            .copied()
            .unwrap_or_default()
    }

    fn request_model<'a>(&'a self, request: &'a LLMRequest) -> &'a str {
        request.model.as_deref().unwrap_or(&self.model)
    }

    /// Create a new Anthropic backend.
    ///
    /// The optional `config` value may contain:
    /// - `model` – registered model name
    /// - `base_url` – registered API base URL
    /// - `extra_headers` – a JSON object whose keys/values become HTTP headers on every
    ///   request.  Values prefixed with `"env:"` are resolved from environment variables
    ///   at backend-creation time (e.g. `"env:LLM_GATEWAY_KEY"` → current env var).
    pub async fn new(api_key: &str, config: Option<serde_json::Value>) -> Result<Self> {
        let model = required_config_string(config.as_ref(), PROTOCOL, MODEL)?;
        let base_url = required_config_string(config.as_ref(), PROTOCOL, BASE_URL)?;

        // Parse optional extra_headers from config.
        // Values prefixed with "env:" are read from environment variables.
        let extra_headers: Vec<(String, String)> = config
            .as_ref()
            .and_then(|c| c.get(config_keys::EXTRA_HEADERS))
            .and_then(|h| h.as_object())
            .map(|obj| {
                obj.iter()
                    .map(|(key, value)| -> Result<(String, String)> {
                        let field = format!("{}.{}", config_keys::EXTRA_HEADERS, key);
                        let raw = value.as_str().ok_or_else(|| {
                            crate::llm::backends::BackendConfigurationError::InvalidOptionalField {
                                protocol: PROTOCOL,
                                field: field.clone(),
                            }
                        })?;
                        let resolved = resolve_configured_value(PROTOCOL, field, raw)?;
                        Ok::<_, anyhow::Error>((key.clone(), resolved))
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();

        let model_capabilities = configured_model_capabilities(config.as_ref());
        let registered_models = configured_model_info(config.as_ref());

        Ok(AnthropicBackend {
            api_key: api_key.to_string(),
            model,
            base_url,
            extra_headers,
            model_capabilities,
            registered_models,
            client: llm_http_client(),
        })
    }

    /// Convert content parts to Anthropic format.
    fn content_parts_to_anthropic(parts: &[ContentPart]) -> serde_json::Value {
        if parts.len() == 1
            && let Some(ContentPart::Text { text }) = parts.first()
        {
            return json!(text);
        }
        let anthropic_parts: Vec<serde_json::Value> = parts
            .iter()
            .map(|p| match p {
                ContentPart::Text { text } => json!({ "type": "text", "text": text }),
                ContentPart::Image { url, .. } => json!({
                    "type": "image",
                    "source": { "type": "url", "url": url }
                }),
                ContentPart::ToolCall { id, function } => json!({
                    "type": "tool_use",
                    "id": id,
                    "name": function.name,
                    "input": function.arguments
                }),
            })
            .collect();
        json!(anthropic_parts)
    }

    /// Convert a structured Message to Anthropic JSON format.
    fn message_to_anthropic_json(msg: &crate::llm::backends::Message) -> serde_json::Value {
        let role = match msg.role {
            Role::User | Role::Tool | Role::System => roles::USER,
            Role::Assistant => roles::ASSISTANT,
        };

        if msg.role == Role::Tool
            && let Some(ref tool_call_id) = msg.tool_call_id
        {
            return json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": msg.text_content()
                }]
            });
        }

        json!({
            "role": role,
            "content": Self::content_parts_to_anthropic(&msg.content)
        })
    }

    /// Build request body for Anthropic API.
    fn build_request_body(&self, request: &LLMRequest) -> serde_json::Value {
        let model = self.request_model(request);
        let all_messages = request.resolved_messages();

        let system_text: String = all_messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.text_content())
            .collect::<Vec<_>>()
            .join("\n");

        let conversation_messages: Vec<serde_json::Value> = all_messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(Self::message_to_anthropic_json)
            .collect();

        let mut body = json!({
            "model": model,
            "messages": conversation_messages,
            "max_tokens": request.max_tokens.unwrap_or(wire_defaults::ANTHROPIC_MAX_TOKENS),
            "temperature": request.temperature,
        });

        if !system_text.is_empty() {
            body["system"] = json!(system_text);
        }

        // Extended thinking. When enabled, Anthropic requires `budget_tokens` in
        // [1024, max_tokens), temperature == 1, and no `top_p`. Bump max_tokens to
        // keep output room above the thinking budget and force those constraints.
        let thinking_enabled =
            request.enable_thinking == Some(true) || request.thinking_token_budget.is_some();
        if thinking_enabled {
            let budget = request
                .thinking_token_budget
                .unwrap_or(wire_defaults::ANTHROPIC_MAX_TOKENS as u64)
                .max(1024);
            if body["max_tokens"].as_u64().unwrap_or(0) <= budget {
                body["max_tokens"] = json!(budget + wire_defaults::ANTHROPIC_MAX_TOKENS as u64);
            }
            body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
            body["temperature"] = json!(1.0);
        }

        // Add optional parameters. `top_p` is incompatible with extended thinking.
        if !thinking_enabled && let Some(top_p) = request.top_p {
            body["top_p"] = json!(top_p);
        }

        if !request.stop_sequences.is_empty() {
            body["stop_sequences"] = json!(request.stop_sequences);
        }

        // Add tools if provided (Anthropic format)
        if let Some(tools) = &request.tools
            && !tools.is_empty()
        {
            let anthropic_tools: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.parameters
                    })
                })
                .collect();
            body[tool_keys::TOOLS] = json!(anthropic_tools);

            // Add tool_choice if specified
            if let Some(choice) = &request.tool_choice {
                body[tool_keys::TOOL_CHOICE] = match choice {
                    ToolChoice::Auto => json!({"type": "auto"}),
                    ToolChoice::None => json!({"type": "none"}),
                    ToolChoice::Required => json!({"type": "any"}),
                    ToolChoice::Specific(name) => json!({
                        "type": "tool",
                        "name": name
                    }),
                };
            }
        }

        body
    }

    /// Parse Anthropic API response.
    fn parse_response(response: AnthropicResponse, model: &str) -> Result<LLMResponse> {
        let mut text_content = String::new();
        let mut tool_calls = Vec::new();

        // Process content blocks - can be text or tool_use
        for block in &response.content {
            match block {
                ContentBlock::Text { text } => {
                    text_content.push_str(text);
                }
                ContentBlock::ToolUse { id, name, input } => {
                    tool_calls.push(ToolCall::new(id.clone(), name.clone(), input.clone()));
                }
            }
        }

        // Determine finish reason
        let finish_reason = if tool_calls.is_empty() {
            FinishReason::from_string(&response.stop_reason)
        } else {
            FinishReason::ToolUse
        };

        let usage = TokenUsage::new(response.usage.input_tokens, response.usage.output_tokens);

        Ok(LLMResponse::new(text_content, model, usage, finish_reason).with_tool_calls(tool_calls))
    }
}

#[async_trait]
impl LLMBackend for AnthropicBackend {
    async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        validate_provider_dispatch(&request)?;

        let model = self.request_model(&request).to_string();
        let body = self.build_request_body(&request);
        let url = format!("{}{}", self.base_url, api_paths::MESSAGES);

        log_debug!(
            "models::anthropic",
            model = %model,
            url = %url,
            "Sending request to Anthropic"
        );

        let mut req_builder = self
            .client
            .post(&url)
            .header(headers::X_API_KEY, &self.api_key)
            .header(headers::ANTHROPIC_VERSION, ANTHROPIC_VERSION)
            .header(headers::CONTENT_TYPE, headers::CONTENT_TYPE_JSON);

        // Inject extra headers (e.g. on-premises X-Custom-Gateway-Key)
        for (name, value) in &self.extra_headers {
            req_builder = req_builder.header(name.as_str(), value.as_str());
        }

        let response = req_builder
            .json(&body)
            .send()
            .await
            .context("Failed to send request to Anthropic")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("Anthropic API error (status {}): {}", status, error_text);
        }

        let api_response: AnthropicResponse = response
            .json()
            .await
            .context("Failed to parse Anthropic response")?;

        Self::parse_response(api_response, &model)
    }

    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + '_>> {
        Box::pin(async_stream::try_stream! {
            validate_provider_dispatch(&request)?;
            let model = self.request_model(&request).to_string();
            let mut body = self.build_request_body(&request);
            body[message_keys::STREAM] = json!(true);

            let url = format!("{}{}", self.base_url, api_paths::MESSAGES);

            log_debug!(
                "models::anthropic",
                model = %model,
                url = %url,
                "Sending streaming request to Anthropic"
            );

            let mut req_builder = self.client
                .post(&url)
                .header(headers::X_API_KEY, &self.api_key)
                .header(headers::ANTHROPIC_VERSION, ANTHROPIC_VERSION)
                .header(headers::CONTENT_TYPE, headers::CONTENT_TYPE_JSON);

            for (name, value) in &self.extra_headers {
                req_builder = req_builder.header(name.as_str(), value.as_str());
            }

            let response = req_builder
                .json(&body)
                .send()
                .await
                .context("Failed to send streaming request to Anthropic")?
                .error_for_status()
                .map_err(|e| anyhow::anyhow!("Anthropic API error: {}", e))?;

            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut full_content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut input_tokens: usize = 0;
            let mut output_tokens: usize = 0;

            // Track current content block for tool_use accumulation.
            let mut current_tool_id = String::new();
            let mut current_tool_name = String::new();
            let mut current_tool_input = String::new();

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result.context("Stream read error")?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process SSE lines.  Anthropic sends:
                //   event: <type>\n
                //   data: <json>\n\n
                while let Some(double_newline) = buffer.find("\n\n") {
                    let block = buffer[..double_newline].to_string();
                    buffer.drain(..double_newline + 2);

                    let mut event_type = String::new();
                    let mut data_str = String::new();

                    for line in block.lines() {
                        if let Some(et) = line.strip_prefix(sse::EVENT_PREFIX) {
                            event_type = et.trim().to_string();
                        } else if let Some(d) = line.strip_prefix(sse::DATA_PREFIX) {
                            data_str = d.trim().to_string();
                        }
                    }

                    if data_str.is_empty() {
                        continue;
                    }

                    match event_type.as_str() {
                        anthropic_events::MESSAGE_START => {
                            if let Ok(payload) = serde_json::from_str::<MessageStartPayload>(&data_str)
                                && let Some(msg) = payload.message
                                    && let Some(usage) = msg.usage {
                                        input_tokens = usage.input_tokens as usize;
                                    }
                        }
                        anthropic_events::CONTENT_BLOCK_START => {
                            if let Ok(payload) = serde_json::from_str::<ContentBlockStartPayload>(&data_str)
                                && let Some(block) = payload.content_block {
                                    match block {
                                        StreamContentBlock::ToolUse { id, name } => {
                                            current_tool_id = id;
                                            current_tool_name = name;
                                            current_tool_input.clear();
                                            yield StreamChunk::ToolCallStart {
                                                id: current_tool_id.clone(),
                                                name: current_tool_name.clone(),
                                            };
                                        }
                                        StreamContentBlock::Thinking { thinking } => {
                                            if let Some(text) = thinking
                                                && !text.is_empty() {
                                                    yield StreamChunk::Thought(text);
                                                }
                                        }
                                        _ => {}
                                    }
                                }
                        }
                        anthropic_events::CONTENT_BLOCK_DELTA => {
                            if let Ok(payload) = serde_json::from_str::<ContentBlockDeltaPayload>(&data_str)
                                && let Some(delta) = payload.delta {
                                    match delta {
                                        StreamDelta::TextDelta { text } => {
                                            if let Some(text) = text
                                                && !text.is_empty() {
                                                    full_content.push_str(&text);
                                                    yield StreamChunk::Token(text);
                                                }
                                        }
                                        StreamDelta::InputJsonDelta { partial_json } => {
                                            if let Some(partial) = partial_json
                                                && !partial.is_empty() {
                                                    current_tool_input.push_str(&partial);
                                                    yield StreamChunk::ToolCallDelta {
                                                        id: current_tool_id.clone(),
                                                        arguments_delta: partial,
                                                    };
                                                }
                                        }
                                        StreamDelta::ThinkingDelta { thinking } => {
                                            if let Some(text) = thinking
                                                && !text.is_empty() {
                                                    yield StreamChunk::Thought(text);
                                                }
                                        }
                                        StreamDelta::Other => {}
                                    }
                                }
                        }
                        anthropic_events::CONTENT_BLOCK_STOP => {
                            if !current_tool_id.is_empty() {
                                let args: serde_json::Value = serde_json::from_str(&current_tool_input)
                                    .unwrap_or(json!({}));
                                tool_calls.push(ToolCall::new(
                                    current_tool_id.clone(),
                                    current_tool_name.clone(),
                                    args,
                                ));
                                current_tool_id.clear();
                                current_tool_name.clear();
                                current_tool_input.clear();
                            }
                        }
                        anthropic_events::MESSAGE_DELTA => {
                            if let Ok(payload) = serde_json::from_str::<MessageDeltaPayload>(&data_str)
                                && let Some(usage) = payload.usage {
                                    output_tokens = usage.output_tokens as usize;
                                }
                        }
                        anthropic_events::MESSAGE_STOP => {
                            let finish_reason = if tool_calls.is_empty() {
                                FinishReason::Stop
                            } else {
                                FinishReason::ToolUse
                            };

                            let usage = TokenUsage::new(input_tokens, output_tokens);
                            if input_tokens > 0 || output_tokens > 0 {
                                yield StreamChunk::Usage(usage.clone());
                            }

                            let resp = LLMResponse::new(
                                full_content.clone(),
                                &model,
                                usage,
                                finish_reason,
                            ).with_tool_calls(tool_calls.clone());

                            yield StreamChunk::Done(resp);
                            return;
                        }
                        anthropic_events::ERROR => {
                            if let Ok(payload) = serde_json::from_str::<StreamErrorPayload>(&data_str) {
                                let msg = payload.error
                                    .and_then(|e| e.message)
                                    .unwrap_or_else(|| "Unknown streaming error".to_string());
                                yield StreamChunk::Error(msg);
                            } else {
                                yield StreamChunk::Error("Unknown streaming error".to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }

            Err(anyhow::anyhow!(
                "Anthropic stream ended before {} terminal event",
                anthropic_events::MESSAGE_STOP
            ))?;
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
        // Anthropic doesn't have a lightweight public health endpoint for all
        // models; perform a minimal generation request as a check.
        let test_request = LLMRequest::new("test").with_max_tokens(1);

        match self.generate(test_request).await {
            Ok(_) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("Anthropic health check failed: {}", e)),
        }
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
    async fn registered_capabilities_override_anthropic_model_name_inference() {
        let backend = AnthropicBackend::new(
            "",
            Some(json!({
                "model": "claude-compatible-gateway-model",
                "base_url": "https://llm.example.test/anthropic",
                "models": [{
                    "id": "claude-compatible-gateway-model",
                    "supports_vision": false,
                    "supports_functions": false
                }]
            })),
        )
        .await
        .expect("registered backend config");

        let capabilities = backend.capabilities();
        assert!(!capabilities.vision);
        assert!(!capabilities.functions);

        let models = backend.list_models().await.expect("registered models");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "claude-compatible-gateway-model");
        assert!(!models[0].supports_vision);
        assert!(!models[0].supports_functions);
    }
}

// Anthropic API response types
#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
    stop_reason: String,
    usage: Usage,
}

/// Content block from Anthropic API - can be text or tool_use
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
struct Usage {
    input_tokens: usize,
    output_tokens: usize,
}

// Anthropic streaming response types (constructed by serde, not user code)
#[allow(dead_code)]
#[derive(Deserialize)]
struct MessageStartPayload {
    #[serde(default)]
    message: Option<MessageStartMessage>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct MessageStartMessage {
    #[serde(default)]
    usage: Option<StreamInputUsage>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct StreamInputUsage {
    #[serde(default)]
    input_tokens: u64,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ContentBlockStartPayload {
    #[serde(default)]
    content_block: Option<StreamContentBlock>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(tag = "type")]
enum StreamContentBlock {
    #[serde(rename = "tool_use")]
    ToolUse {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
    },
    #[serde(rename = "thinking")]
    Thinking {
        #[serde(default)]
        thinking: Option<String>,
    },
    #[serde(rename = "text")]
    Text {},
    #[serde(other)]
    Other,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ContentBlockDeltaPayload {
    #[serde(default)]
    delta: Option<StreamDelta>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(tag = "type")]
enum StreamDelta {
    #[serde(rename = "text_delta")]
    TextDelta {
        #[serde(default)]
        text: Option<String>,
    },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta {
        #[serde(default)]
        partial_json: Option<String>,
    },
    #[serde(rename = "thinking_delta")]
    ThinkingDelta {
        #[serde(default)]
        thinking: Option<String>,
    },
    #[serde(other)]
    Other,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct MessageDeltaPayload {
    #[serde(default)]
    usage: Option<StreamOutputUsage>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct StreamOutputUsage {
    #[serde(default)]
    output_tokens: u64,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct StreamErrorPayload {
    #[serde(default)]
    error: Option<StreamError>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct StreamError {
    #[serde(default)]
    message: Option<String>,
}
