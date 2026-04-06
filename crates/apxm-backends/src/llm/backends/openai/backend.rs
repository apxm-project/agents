//! OpenAI backend implementation.
//!
//! Implements the LLMBackend trait for OpenAI's API, supporting modern OpenAI
//! model identifiers (gpt-4o, gpt-4o-mini, gpt-4-turbo, gpt-4, gpt-3.5-turbo, etc.).
//!
//! This file updates the provider default model and the list of known models
//! surfaced by `list_models()` to reflect more recent model names.

use crate::llm::backends::traits::StreamChunk;
use crate::llm::backends::{ContentPart, LLMBackend, LLMRequest, LLMResponse, Role, ToolChoice};
use anyhow::{Context, Result};
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::types::{FinishReason, ModelCapabilities, ModelInfo, TokenUsage, ToolCall};
use async_trait::async_trait;
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use std::pin::Pin;
use tokio_stream::Stream;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-4o-mini";

/// OpenAI LLM backend.
///
/// Supports any OpenAI-compatible API including on-premises endpoints.
/// Extra headers (e.g. `X-Custom-Gateway-Key`) can be injected via the
/// `extra_headers` config key:
///
/// ```toml
/// [[llm_backends]]
/// name = "apxm"
/// provider = "openai"
/// model = "gpt-4o-mini"   # or your model name
/// api_key = "dummy"
/// base_url = "https://your-openai-compatible-gateway/v1"
///
/// [llm_backends.extra_headers]
/// X-Custom-Gateway-Key = "env:OCP_APIM_KEY"
/// user = "env:USERNAME"
/// ```
pub struct OpenAIBackend {
    api_key: String,
    model: String,
    base_url: String,
    /// Additional HTTP headers injected on every request.
    extra_headers: Vec<(String, String)>,
    client: reqwest::Client,
}

impl OpenAIBackend {
    fn request_model<'a>(&'a self, request: &'a LLMRequest) -> &'a str {
        request.model.as_deref().unwrap_or(&self.model)
    }

    /// Create a new OpenAI backend.
    ///
    /// The optional `config` value may contain:
    /// - `model` – override the default model name
    /// - `base_url` – override the API base URL (enables on-premises / Azure / OpenRouter endpoints)
    /// - `extra_headers` – a JSON object whose keys/values become HTTP headers on every
    ///   request.  Values prefixed with `"env:"` are resolved from environment variables
    ///   at backend-creation time (e.g. `"env:USERNAME"` → current OS user).
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
            .and_then(|c| c.get("extra_headers"))
            .and_then(|h| h.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        let raw = v.as_str()?;
                        let resolved = if let Some(var_name) = raw.strip_prefix("env:") {
                            std::env::var(var_name).unwrap_or_else(|_| raw.to_string())
                        } else {
                            raw.to_string()
                        };
                        Some((k.clone(), resolved))
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(OpenAIBackend {
            api_key: api_key.to_string(),
            model,
            base_url,
            extra_headers,
            client: reqwest::Client::new(),
        })
    }

    /// Convert a structured Message to OpenAI JSON format.
    fn message_to_openai_json(msg: &crate::llm::backends::Message) -> serde_json::Value {
        let role = match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        let mut obj = json!({ "role": role });

        if let Some(ref tool_call_id) = msg.tool_call_id {
            obj["tool_call_id"] = json!(tool_call_id);
        }

        let text_parts: Vec<&str> = msg
            .content
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();

        let tool_call_parts: Vec<serde_json::Value> = msg
            .content
            .iter()
            .filter_map(|p| match p {
                ContentPart::ToolCall { id, function } => Some(json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": function.name,
                        "arguments": function.arguments.to_string()
                    }
                })),
                _ => None,
            })
            .collect();

        let image_parts: Vec<serde_json::Value> = msg
            .content
            .iter()
            .filter_map(|p| match p {
                ContentPart::Image { url, detail } => {
                    let mut img = json!({ "type": "image_url", "image_url": { "url": url } });
                    if let Some(d) = detail {
                        img["image_url"]["detail"] = json!(d);
                    }
                    Some(img)
                }
                _ => None,
            })
            .collect();

        if !tool_call_parts.is_empty() {
            obj["tool_calls"] = json!(tool_call_parts);
        }

        if image_parts.is_empty() {
            obj["content"] = json!(text_parts.join(""));
        } else {
            let mut content_arr: Vec<serde_json::Value> = text_parts
                .iter()
                .map(|t| json!({ "type": "text", "text": t }))
                .collect();
            content_arr.extend(image_parts);
            obj["content"] = json!(content_arr);
        }

        obj
    }

    /// Build request body for OpenAI API.
    fn build_request_body(&self, request: &LLMRequest) -> serde_json::Value {
        let model = self.request_model(request);
        let messages: Vec<serde_json::Value> = request
            .resolved_messages()
            .iter()
            .map(Self::message_to_openai_json)
            .collect();

        // Some models (gpt-5, o1, o3, o4, reasoning models) only accept
        // the default temperature and reject custom values with a 400 error.
        // Detect these by model name prefix and skip the temperature field.
        let is_reasoning_model = model.starts_with("o1") || model.starts_with("o3") ||
            model.starts_with("o4") || model.starts_with("gpt-5") ||
            model.contains("-codex") || model.contains("reasoning");

        let mut body = if is_reasoning_model {
            json!({
                "model": model,
                "messages": messages,
            })
        } else {
            json!({
                "model": model,
                "messages": messages,
                "temperature": request.temperature,
            })
        };

        // Add optional parameters
        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }

        if let Some(top_p) = request.top_p {
            body["top_p"] = json!(top_p);
        }

        if let Some(freq_penalty) = request.frequency_penalty {
            body["frequency_penalty"] = json!(freq_penalty);
        }

        if let Some(pres_penalty) = request.presence_penalty {
            body["presence_penalty"] = json!(pres_penalty);
        }

        if !request.stop_sequences.is_empty() {
            body["stop"] = json!(request.stop_sequences);
        }

        // Add tools if provided
        if let Some(tools) = &request.tools
            && !tools.is_empty()
        {
            let openai_tools: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters
                        }
                    })
                })
                .collect();
            body["tools"] = json!(openai_tools);

            // Add tool_choice if specified
            if let Some(choice) = &request.tool_choice {
                body["tool_choice"] = match choice {
                    ToolChoice::Auto => json!("auto"),
                    ToolChoice::None => json!("none"),
                    ToolChoice::Required => json!("required"),
                    ToolChoice::Specific(name) => json!({
                        "type": "function",
                        "function": {"name": name}
                    }),
                };
            }
        }

        // Merge in extra_body if provided (for vLLM extensions, etc.)
        if let Some(extra) = &request.extra_body {
            if let serde_json::Value::Object(extra_map) = extra {
                if let serde_json::Value::Object(body_map) = &mut body {
                    for (key, value) in extra_map {
                        body_map.insert(key.clone(), value.clone());
                    }
                }
            }
        }

        body
    }

    /// Parse OpenAI API response.
    ///
    /// Some OpenAI-compatible APIs (e.g. on-premises reasoning models) return
    /// `content: null` and place the generated text in a `reasoning` field instead.
    /// We fall back to `reasoning` when `content` is absent so those endpoints work
    /// transparently.
    fn parse_response(&self, response: OpenAIResponse, model: &str) -> Result<LLMResponse> {
        let choice = response.choices.first().context("No choices in response")?;

        // Prefer `content`; fall back to `reasoning` for on-premises reasoning models.
        let content = choice
            .message
            .content
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| choice.message.reasoning.clone())
            .unwrap_or_default();

        // Parse tool calls if present
        let tool_calls: Vec<ToolCall> = choice
            .message
            .tool_calls
            .as_ref()
            .map(|calls| {
                calls
                    .iter()
                    .map(|tc| {
                        let args: serde_json::Value =
                            serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                        ToolCall::new(tc.id.clone(), tc.function.name.clone(), args)
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Determine finish reason
        let finish_reason = if !tool_calls.is_empty() {
            FinishReason::ToolUse
        } else {
            FinishReason::from_string(&choice.finish_reason)
        };

        let usage = TokenUsage::new(
            response.usage.prompt_tokens,
            response.usage.completion_tokens,
        );

        Ok(LLMResponse::new(content, model, usage, finish_reason).with_tool_calls(tool_calls))
    }
}

#[async_trait]
impl LLMBackend for OpenAIBackend {
    async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        request.validate()?;

        let model = self.request_model(&request).to_string();
        let body = self.build_request_body(&request);
        let url = format!("{}/chat/completions", self.base_url);

        tracing::debug!(
            model = %model,
            url = %url,
            "Sending request to OpenAI"
        );

        let mut req_builder = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json");

        // Inject extra headers (e.g. on-premises X-Custom-Gateway-Key)
        for (name, value) in &self.extra_headers {
            req_builder = req_builder.header(name.as_str(), value.as_str());
        }

        let response = req_builder
            .json(&body)
            .send()
            .await
            .context("Failed to send request to OpenAI")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("OpenAI API error (status {}): {}", status, error_text);
        }

        let api_response: OpenAIResponse = response
            .json()
            .await
            .context("Failed to parse OpenAI response")?;

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
            body["stream"] = json!(true);

            let url = format!("{}/chat/completions", self.base_url);

            tracing::debug!(
                model = %model,
                url = %url,
                "Sending streaming request to OpenAI"
            );

            let mut req_builder = self.client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json");

            for (name, value) in &self.extra_headers {
                req_builder = req_builder.header(name.as_str(), value.as_str());
            }

            let response = req_builder.json(&body).send().await
                .context("Failed to send streaming request to OpenAI")?
                .error_for_status()
                .map_err(|e| anyhow::anyhow!("OpenAI API error: {}", e))?;

            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut full_content = String::new();
            let mut last_usage = TokenUsage::new(0, 0);
            let mut tool_calls_map: std::collections::HashMap<usize, (String, String, String)> =
                std::collections::HashMap::new();

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

                    if let Some(data) = line.strip_prefix("data: ") {
                        if data.trim() == "[DONE]" {
                            // Build final response.
                            let tool_calls: Vec<ToolCall> = tool_calls_map.values()
                                .map(|(id, name, args)| {
                                    let args_val = serde_json::from_str(args)
                                        .unwrap_or(json!({}));
                                    ToolCall::new(id.clone(), name.clone(), args_val)
                                })
                                .collect();

                            let finish_reason = if !tool_calls.is_empty() {
                                FinishReason::ToolUse
                            } else {
                                FinishReason::Stop
                            };

                            let resp = LLMResponse::new(
                                full_content.clone(),
                                &model,
                                last_usage.clone(),
                                finish_reason,
                            ).with_tool_calls(tool_calls);

                            yield StreamChunk::Done(resp);
                            return;
                        }

                        if let Ok(parsed) = serde_json::from_str::<StreamChunkPayload>(data) {
                            // Emit usage if present at top level.
                            if let Some(ref usage_obj) = parsed.usage {
                                let input = usage_obj.prompt_tokens as usize;
                                let output = usage_obj.completion_tokens as usize;
                                if input > 0 || output > 0 {
                                    last_usage = TokenUsage::new(input, output);
                                    yield StreamChunk::Usage(last_usage.clone());
                                }
                            }

                            for choice in &parsed.choices {
                                if let Some(ref delta) = choice.delta {
                                    // Text content.
                                    if let Some(ref content) = delta.content {
                                        if !content.is_empty() {
                                            full_content.push_str(content);
                                            yield StreamChunk::Token(content.to_string());
                                        }
                                    }

                                    // Tool calls.
                                    if let Some(ref tc_arr) = delta.tool_calls {
                                        for tc in tc_arr {
                                            let index = tc.index as usize;

                                            if let Some(ref id) = tc.id {
                                                let name = tc.function
                                                    .as_ref()
                                                    .and_then(|f| f.name.as_deref())
                                                    .unwrap_or("")
                                                    .to_string();
                                                tool_calls_map.insert(
                                                    index,
                                                    (id.to_string(), name.clone(), String::new()),
                                                );
                                                yield StreamChunk::ToolCallStart {
                                                    id: id.to_string(),
                                                    name,
                                                };
                                            }

                                            if let Some(ref args) = tc.function.as_ref().and_then(|f| f.arguments.as_ref()) {
                                                if !args.is_empty() {
                                                    if let Some((id, _, accumulated_args)) =
                                                        tool_calls_map.get_mut(&index)
                                                    {
                                                        accumulated_args.push_str(args);
                                                        yield StreamChunk::ToolCallDelta {
                                                            id: id.clone(),
                                                            arguments_delta: args.to_string(),
                                                        };
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Stream ended without [DONE] -- emit what we have.
            let tool_calls: Vec<ToolCall> = tool_calls_map.values()
                .map(|(id, name, args)| {
                    let args_val = serde_json::from_str(args).unwrap_or(json!({}));
                    ToolCall::new(id.clone(), name.clone(), args_val)
                })
                .collect();

            let finish_reason = if !tool_calls.is_empty() {
                FinishReason::ToolUse
            } else {
                FinishReason::Stop
            };

            let resp = LLMResponse::new(
                full_content,
                &model,
                last_usage,
                finish_reason,
            ).with_tool_calls(tool_calls);

            yield StreamChunk::Done(resp);
        })
    }

    fn name(&self) -> &str {
        "openai"
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn health_check(&self) -> Result<()> {
        let url = format!("{}/models", self.base_url);

        let mut req_builder = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key));

        for (name, value) in &self.extra_headers {
            req_builder = req_builder.header(name.as_str(), value.as_str());
        }

        let response = req_builder
            .send()
            .await
            .context("Failed to connect to OpenAI")?;

        if !response.status().is_success() {
            // Some compatible APIs (e.g. on-premises endpoints) don't expose /models —
            // treat 404 as "connected but endpoint absent" rather than a hard failure.
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                tracing::debug!(
                    url = %url,
                    "OpenAI-compatible endpoint does not expose /models — treating as healthy"
                );
                return Ok(());
            }
            anyhow::bail!("OpenAI health check failed: {}", response.status());
        }

        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        // Return well-known models as documentation hints.
        // The API will accept any model ID — this list is not enforced.
        Ok(crate::llm::backends::openai::WELL_KNOWN_MODELS
            .iter()
            .map(|id| ModelInfo {
                id: id.to_string(),
                name: id.to_string(),
                context_window: 128_000,
                supports_vision: id.contains("4o") || id.contains("5") || id.contains("turbo"),
                supports_functions: true,
            })
            .collect())
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            streaming: true,
            vision: self.model.contains("vision")
                || self.model.contains("turbo")
                || self.model.contains("gpt-4o"),
            functions: true,
            batch: false,
            fine_tuning: self.model.starts_with("gpt-3.5"),
        }
    }
}

// OpenAI API response types
#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
    finish_reason: String,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAIToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAIToolCall {
    id: String,
    function: OpenAIFunction,
}

#[derive(Debug, Deserialize)]
struct OpenAIFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct Usage {
    prompt_tokens: usize,
    completion_tokens: usize,
}

// OpenAI streaming response types (constructed by serde, not user code)
#[allow(dead_code)]
#[derive(Deserialize)]
struct StreamChunkPayload {
    #[serde(default)]
    usage: Option<StreamUsage>,
    #[serde(default)]
    choices: Vec<StreamChoice>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct StreamUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: Option<StreamDelta>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct StreamToolCallDelta {
    #[serde(default)]
    index: u64,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<StreamFunctionDelta>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct StreamFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::backends::{LLMRequest, ToolDefinition};

    #[test]
    fn test_build_request_body_basic() {
        let backend = OpenAIBackend {
            api_key: "test".to_string(),
            model: "gpt-4".to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            extra_headers: vec![],
            client: reqwest::Client::new(),
        };

        let request = LLMRequest::new("Hello").with_temperature(0.9);

        let body = backend.build_request_body(&request);

        assert_eq!(body["model"], "gpt-4");
        assert_eq!(body["temperature"], 0.9);
        assert_eq!(body["messages"][0]["content"], "Hello");
    }

    #[test]
    fn test_build_request_body_with_system() {
        let backend = OpenAIBackend {
            api_key: "test".to_string(),
            model: "gpt-4-turbo".to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            extra_headers: vec![],
            client: reqwest::Client::new(),
        };

        let request = LLMRequest::new("Hello")
            .with_system_prompt("You are helpful")
            .with_temperature(0.5);

        let body = backend.build_request_body(&request);

        assert_eq!(body["model"], "gpt-4-turbo");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "Hello");
    }

    #[test]
    fn test_build_request_body_with_tools() {
        let backend = OpenAIBackend {
            api_key: "test".to_string(),
            model: "gpt-4".to_string(),
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
        assert_eq!(tools_arr[0]["function"]["name"], "bash");
        assert_eq!(tools_arr[1]["function"]["name"], "read");
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn test_build_request_body_with_specific_tool_choice() {
        let backend = OpenAIBackend {
            api_key: "test".to_string(),
            model: "gpt-4".to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            extra_headers: vec![],
            client: reqwest::Client::new(),
        };

        let tools = vec![ToolDefinition::new("bash", "Execute shell", json!({}))];

        let request = LLMRequest::new("Run command")
            .with_tools(tools)
            .with_tool_choice(ToolChoice::Specific("bash".to_string()));

        let body = backend.build_request_body(&request);

        assert_eq!(body["tool_choice"]["type"], "function");
        assert_eq!(body["tool_choice"]["function"]["name"], "bash");
    }
}
