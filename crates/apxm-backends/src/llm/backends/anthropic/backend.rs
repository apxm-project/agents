//! Anthropic backend implementation.
//!
//! Implements the LLMBackend trait for Anthropic's Claude API.
//! This file updates the default model and the set of models returned by
//! `list_models()` to include newer Claude model identifiers.

use crate::llm::backends::traits::StreamChunk;
use crate::llm::backends::{ContentPart, LLMBackend, LLMRequest, LLMResponse, Role, ToolChoice};
use anyhow::{Context, Result};
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::constants::http::headers;
use apxm_core::constants::llm::{
    anthropic_events, api_paths, config_keys, message_keys, roles, sse, tool_keys,
};
use apxm_core::log_debug;
use apxm_core::types::{FinishReason, ModelCapabilities, ModelInfo, TokenUsage, ToolCall};
use async_trait::async_trait;
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use std::pin::Pin;
use tokio_stream::Stream;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
const DEFAULT_MODEL: &str = "claude-opus-4";
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
    client: reqwest::Client,
}

impl AnthropicBackend {
    fn request_model<'a>(&'a self, request: &'a LLMRequest) -> &'a str {
        request.model.as_deref().unwrap_or(&self.model)
    }

    /// Create a new Anthropic backend.
    ///
    /// The optional `config` value may contain:
    /// - `model` – override the default model name
    /// - `base_url` – override the API base URL (enables on-premises endpoints)
    /// - `extra_headers` – a JSON object whose keys/values become HTTP headers on every
    ///   request.  Values prefixed with `"env:"` are resolved from environment variables
    ///   at backend-creation time (e.g. `"env:LLM_GATEWAY_KEY"` → current env var).
    pub async fn new(api_key: &str, config: Option<serde_json::Value>) -> Result<Self> {
        let model = config
            .as_ref()
            .and_then(|c| c.get(MODEL))
            .and_then(|m| m.as_str())
            .unwrap_or(DEFAULT_MODEL)
            .to_string();

        let base_url = config
            .as_ref()
            .and_then(|c| c.get(BASE_URL))
            .and_then(|u| u.as_str())
            .unwrap_or(DEFAULT_BASE_URL)
            .to_string();

        // Parse optional extra_headers from config.
        // Values prefixed with "env:" are read from environment variables.
        let extra_headers: Vec<(String, String)> = config
            .as_ref()
            .and_then(|c| c.get(config_keys::EXTRA_HEADERS))
            .and_then(|h| h.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        let raw = v.as_str()?;
                        let resolved =
                            if let Some(var_name) = raw.strip_prefix(config_keys::ENV_PREFIX) {
                                std::env::var(var_name).unwrap_or_else(|_| raw.to_string())
                            } else {
                                raw.to_string()
                            };
                        Some((k.clone(), resolved))
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(AnthropicBackend {
            api_key: api_key.to_string(),
            model,
            base_url,
            extra_headers,
            client: reqwest::Client::new(),
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
            Role::User | Role::Tool => roles::USER,
            Role::Assistant => roles::ASSISTANT,
            Role::System => roles::USER,
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
            "max_tokens": request.max_tokens.unwrap_or(apxm_core::constants::defaults::DEFAULT_ANTHROPIC_MAX_TOKENS),
            "temperature": request.temperature,
        });

        if !system_text.is_empty() {
            body["system"] = json!(system_text);
        }

        // Add optional parameters
        if let Some(top_p) = request.top_p {
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
    fn parse_response(&self, response: AnthropicResponse, model: &str) -> Result<LLMResponse> {
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
        let finish_reason = if !tool_calls.is_empty() {
            FinishReason::ToolUse
        } else {
            FinishReason::from_string(&response.stop_reason)
        };

        let usage = TokenUsage::new(response.usage.input_tokens, response.usage.output_tokens);

        Ok(LLMResponse::new(text_content, model, usage, finish_reason).with_tool_calls(tool_calls))
    }
}

#[async_trait]
impl LLMBackend for AnthropicBackend {
    async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        request.validate()?;

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

        self.parse_response(api_response, &model)
    }

    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + '_>> {
        Box::pin(async_stream::try_stream! {
            request.validate()?;
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
                            if let Ok(payload) = serde_json::from_str::<MessageStartPayload>(&data_str) {
                                if let Some(msg) = payload.message {
                                    if let Some(usage) = msg.usage {
                                        input_tokens = usage.input_tokens as usize;
                                    }
                                }
                            }
                        }
                        anthropic_events::CONTENT_BLOCK_START => {
                            if let Ok(payload) = serde_json::from_str::<ContentBlockStartPayload>(&data_str) {
                                if let Some(block) = payload.content_block {
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
                                            if let Some(text) = thinking {
                                                if !text.is_empty() {
                                                    yield StreamChunk::Thought(text);
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        anthropic_events::CONTENT_BLOCK_DELTA => {
                            if let Ok(payload) = serde_json::from_str::<ContentBlockDeltaPayload>(&data_str) {
                                if let Some(delta) = payload.delta {
                                    match delta {
                                        StreamDelta::TextDelta { text } => {
                                            if let Some(text) = text {
                                                if !text.is_empty() {
                                                    full_content.push_str(&text);
                                                    yield StreamChunk::Token(text);
                                                }
                                            }
                                        }
                                        StreamDelta::InputJsonDelta { partial_json } => {
                                            if let Some(partial) = partial_json {
                                                if !partial.is_empty() {
                                                    current_tool_input.push_str(&partial);
                                                    yield StreamChunk::ToolCallDelta {
                                                        id: current_tool_id.clone(),
                                                        arguments_delta: partial,
                                                    };
                                                }
                                            }
                                        }
                                        StreamDelta::ThinkingDelta { thinking } => {
                                            if let Some(text) = thinking {
                                                if !text.is_empty() {
                                                    yield StreamChunk::Thought(text);
                                                }
                                            }
                                        }
                                        StreamDelta::Other => {}
                                    }
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
                            if let Ok(payload) = serde_json::from_str::<MessageDeltaPayload>(&data_str) {
                                if let Some(usage) = payload.usage {
                                    output_tokens = usage.output_tokens as usize;
                                }
                            }
                        }
                        anthropic_events::MESSAGE_STOP => {
                            let finish_reason = if !tool_calls.is_empty() {
                                FinishReason::ToolUse
                            } else {
                                FinishReason::Stop
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

            // Stream ended without message_stop -- emit what we have.
            let finish_reason = if !tool_calls.is_empty() {
                FinishReason::ToolUse
            } else {
                FinishReason::Stop
            };

            let usage = TokenUsage::new(input_tokens, output_tokens);
            let resp = LLMResponse::new(
                full_content,
                &model,
                usage,
                finish_reason,
            ).with_tool_calls(tool_calls);

            yield StreamChunk::Done(resp);
        })
    }

    fn name(&self) -> &str {
        "anthropic"
    }

    fn model(&self) -> &str {
        &self.model
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
        // Return well-known models as documentation hints.
        // The API will accept any model ID — this list is not enforced.
        Ok(crate::llm::backends::anthropic::WELL_KNOWN_MODELS
            .iter()
            .map(|id| ModelInfo {
                id: id.to_string(),
                name: id.to_string(),
                context_window: 200_000,
                supports_vision: true,
                supports_functions: true,
            })
            .collect())
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            streaming: true,
            vision: self.model.starts_with("claude-") && !self.model.contains("legacy"),
            functions: true, // Claude models support tool use
            batch: false,
            fine_tuning: false,
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::backends::{LLMRequest, ToolDefinition};

    #[test]
    fn test_build_request_body() {
        let backend = AnthropicBackend {
            api_key: "test".to_string(),
            model: "claude-opus-4".to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            extra_headers: vec![],
            client: reqwest::Client::new(),
        };

        let request = LLMRequest::new("Hello")
            .with_system_prompt("You are helpful")
            .with_temperature(0.9);

        let body = backend.build_request_body(&request);

        assert_eq!(body["model"], "claude-opus-4");
        assert_eq!(body["temperature"], 0.9);
        assert_eq!(body["system"], "You are helpful");
        assert_eq!(body["messages"][0]["content"], "Hello");
    }

    #[test]
    fn test_build_request_body_with_tools() {
        let backend = AnthropicBackend {
            api_key: "test".to_string(),
            model: "claude-opus-4".to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            extra_headers: vec![],
            client: reqwest::Client::new(),
        };

        let tools = vec![
            ToolDefinition::new(
                "bash",
                "Execute shell commands",
                json!({
                    "type": "object",
                    "properties": {
                        "command": {"type": "string"}
                    },
                    "required": ["command"]
                }),
            ),
            ToolDefinition::new(
                "read",
                "Read file contents",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"}
                    },
                    "required": ["path"]
                }),
            ),
        ];

        let request = LLMRequest::new("List files")
            .with_tools(tools)
            .with_tool_choice(ToolChoice::Auto);

        let body = backend.build_request_body(&request);

        assert!(body.get("tools").is_some());
        let tools_arr = body["tools"].as_array().unwrap();
        assert_eq!(tools_arr.len(), 2);
        assert_eq!(tools_arr[0]["name"], "bash");
        assert_eq!(tools_arr[1]["name"], "read");
        // Anthropic uses input_schema instead of parameters
        assert!(tools_arr[0].get("input_schema").is_some());
        assert_eq!(body["tool_choice"]["type"], "auto");
    }

    #[test]
    fn test_build_request_body_with_specific_tool_choice() {
        let backend = AnthropicBackend {
            api_key: "test".to_string(),
            model: "claude-opus-4".to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            extra_headers: vec![],
            client: reqwest::Client::new(),
        };

        let tools = vec![ToolDefinition::new("bash", "Execute shell", json!({}))];

        let request = LLMRequest::new("Run command")
            .with_tools(tools)
            .with_tool_choice(ToolChoice::Specific("bash".to_string()));

        let body = backend.build_request_body(&request);

        assert_eq!(body["tool_choice"]["type"], "tool");
        assert_eq!(body["tool_choice"]["name"], "bash");
    }

    #[test]
    fn test_parse_content_block_text() {
        let json_str = r#"{"type": "text", "text": "Hello world"}"#;
        let block: ContentBlock = serde_json::from_str(json_str).unwrap();
        match block {
            ContentBlock::Text { text } => assert_eq!(text, "Hello world"),
            _ => panic!("Expected Text block"),
        }
    }

    #[test]
    fn test_parse_content_block_tool_use() {
        let json_str = r#"{
            "type": "tool_use",
            "id": "call_123",
            "name": "bash",
            "input": {"command": "ls -la"}
        }"#;
        let block: ContentBlock = serde_json::from_str(json_str).unwrap();
        match block {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "call_123");
                assert_eq!(name, "bash");
                assert_eq!(input["command"], "ls -la");
            }
            _ => panic!("Expected ToolUse block"),
        }
    }

    #[tokio::test]
    async fn test_extra_headers_are_parsed() {
        let config = json!({
            "extra_headers": {
                "X-Test-Header": "test-value",
                "X-Custom": "custom-value"
            }
        });

        let backend = AnthropicBackend::new("test-key", Some(config))
            .await
            .unwrap();

        assert_eq!(backend.extra_headers.len(), 2);
        assert!(
            backend
                .extra_headers
                .contains(&("X-Test-Header".to_string(), "test-value".to_string()))
        );
        assert!(
            backend
                .extra_headers
                .contains(&("X-Custom".to_string(), "custom-value".to_string()))
        );
    }

    #[tokio::test]
    async fn test_extra_headers_env_resolution() {
        unsafe {
            std::env::set_var("TEST_ANTHROPIC_HEADER", "from-env");
        }

        let config = json!({
            "extra_headers": {
                "X-From-Env": "env:TEST_ANTHROPIC_HEADER",
                "X-Literal": "literal-value"
            }
        });

        let backend = AnthropicBackend::new("test-key", Some(config))
            .await
            .unwrap();

        assert_eq!(backend.extra_headers.len(), 2);
        assert!(
            backend
                .extra_headers
                .contains(&("X-From-Env".to_string(), "from-env".to_string()))
        );
        assert!(
            backend
                .extra_headers
                .contains(&("X-Literal".to_string(), "literal-value".to_string()))
        );

        unsafe {
            std::env::remove_var("TEST_ANTHROPIC_HEADER");
        }
    }
}
