//! OpenAI backend implementation.
//!
//! Implements the LLMBackend trait for OpenAI's API, supporting modern OpenAI
//! model identifiers (gpt-4o, gpt-4o-mini, gpt-4-turbo, gpt-4, gpt-3.5-turbo, etc.).
//!
//! The adapter exposes the protocol mechanics and discovery catalog for
//! explicitly registered models.

use crate::llm::ProviderProtocol;
use crate::llm::backends::http::llm_http_client;
use crate::llm::backends::traits::StreamChunk;
use crate::llm::backends::{
    ConfiguredModelCapabilities, configured_model_capabilities, configured_model_info,
    required_config_string, resolve_configured_value,
};
use crate::llm::backends::{ContentPart, LLMBackend, LLMRequest, LLMResponse, Role, ToolChoice};
#[cfg(test)]
use crate::llm::backends::{FunctionCall, Message};
use crate::llm::wire::{
    api_paths, config_keys, headers, message_keys, openai as openai_keys, roles, sse, tool_keys,
};
use anyhow::{Context, Result};
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::types::{FinishReason, ModelCapabilities, ModelInfo, TokenUsage, ToolCall};
use async_trait::async_trait;
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use tokio_stream::Stream;

impl LLMRequest {
    /// Validate the normalized request shape before any provider adapter sends it.
    ///
    /// Tool results are valid only when they reference an earlier assistant
    /// tool call. Provider-bound prompt inputs cannot synthesize that
    /// correlation, so an uncorrelated tool payload fails closed here.
    pub fn validate_provider_dispatch(&self) -> Result<()> {
        self.validate()?;

        let mut prior_tool_calls = HashSet::new();
        for (index, message) in self.resolved_messages().iter().enumerate() {
            if message.content.is_empty() {
                anyhow::bail!("provider message at index {index} has no content");
            }

            match message.role {
                Role::Assistant => {
                    if message.tool_call_id.is_some() {
                        anyhow::bail!(
                            "assistant message at index {index} carries an invalid tool_call_id"
                        );
                    }
                    for part in &message.content {
                        if let ContentPart::ToolCall { id, function } = part {
                            if id.trim().is_empty() || function.name.trim().is_empty() {
                                anyhow::bail!(
                                    "assistant tool call at index {index} has a missing or malformed id or name"
                                );
                            }
                            if !prior_tool_calls.insert(id.clone()) {
                                anyhow::bail!(
                                    "assistant tool call at index {index} reuses tool_call_id '{id}'"
                                );
                            }
                        }
                    }
                }
                Role::Tool => {
                    let tool_call_id = message
                        .tool_call_id
                        .as_deref()
                        .filter(|id| !id.trim().is_empty())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "tool message at index {index} is missing a correlated tool_call_id"
                            )
                        })?;
                    if !prior_tool_calls.contains(tool_call_id) {
                        anyhow::bail!(
                            "tool message at index {index} references uncorrelated tool_call_id '{tool_call_id}'"
                        );
                    }
                    if message.text_content().trim().is_empty()
                        || message
                            .content
                            .iter()
                            .any(|part| matches!(part, ContentPart::ToolCall { .. }))
                    {
                        anyhow::bail!("tool message at index {index} has malformed content");
                    }
                }
                Role::System | Role::User => {
                    if message.tool_call_id.is_some()
                        || message
                            .content
                            .iter()
                            .any(|part| matches!(part, ContentPart::ToolCall { .. }))
                    {
                        anyhow::bail!(
                            "provider message at index {index} has a malformed {:?} role payload",
                            message.role
                        );
                    }
                }
            }
        }

        Ok(())
    }
}

/// Shared final admission guard used by every provider adapter.
pub(crate) fn validate_provider_dispatch(request: &LLMRequest) -> Result<()> {
    request.validate_provider_dispatch()
}

const PROTOCOL: ProviderProtocol = ProviderProtocol::OpenAI;
/// Output-budget key for reasoning-family models (the classic key is `max_tokens`).
const MAX_COMPLETION_TOKENS: &str = "max_completion_tokens";

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
/// api_key = "env:OPENAI_API_KEY"
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
    /// Registered per-model request-shaping and capability evidence.
    model_capabilities: HashMap<String, ConfiguredModelCapabilities>,
    /// Registered model metadata exposed through backend inspection.
    registered_models: Vec<ModelInfo>,
    /// Whether this OpenAI-compatible endpoint accepts structured-output
    /// request fields. Runtime schema validation still applies when disabled.
    structured_outputs_supported: bool,
    client: reqwest::Client,
}

impl OpenAIBackend {
    fn configured_capabilities(&self, model: &str) -> ConfiguredModelCapabilities {
        self.model_capabilities
            .get(model)
            .copied()
            .unwrap_or_default()
    }

    fn apply_auth_header(&self, req_builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.api_key.is_empty() {
            req_builder
        } else {
            req_builder.header(headers::AUTHORIZATION, format!("Bearer {}", self.api_key))
        }
    }

    pub(crate) fn apply_transport_headers(
        &self,
        req_builder: reqwest::RequestBuilder,
    ) -> reqwest::RequestBuilder {
        let mut req_builder = self.apply_auth_header(req_builder);
        for (name, value) in &self.extra_headers {
            req_builder = req_builder.header(name.as_str(), value.as_str());
        }
        req_builder
    }

    fn request_model<'a>(&'a self, request: &'a LLMRequest) -> &'a str {
        request.model.as_deref().unwrap_or(&self.model)
    }

    /// Create a new OpenAI backend.
    ///
    /// The optional `config` value may contain:
    /// - `model` – registered model name
    /// - `base_url` – registered API base URL
    /// - `extra_headers` – a JSON object whose keys/values become HTTP headers on every
    ///   request.  Values prefixed with `"env:"` are resolved from environment variables
    ///   at backend-creation time (e.g. `"env:USERNAME"` → current OS user).
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

        let structured_outputs_supported = config
            .as_ref()
            .and_then(|c| c.get(config_keys::SUPPORTS_STRUCTURED_OUTPUTS))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        Ok(OpenAIBackend {
            api_key: api_key.to_string(),
            model,
            base_url,
            extra_headers,
            model_capabilities,
            registered_models,
            structured_outputs_supported,
            client: llm_http_client(),
        })
    }

    /// Convert a structured Message to OpenAI JSON format.
    fn message_to_openai_json(msg: &crate::llm::backends::Message) -> serde_json::Value {
        let role = match msg.role {
            Role::System => roles::SYSTEM,
            Role::User => roles::USER,
            Role::Assistant => roles::ASSISTANT,
            Role::Tool => roles::TOOL,
        };
        let mut obj = json!({});
        obj[openai_keys::ROLE] = json!(role);

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
            obj[tool_keys::TOOL_CALLS] = json!(tool_call_parts);
        }

        if image_parts.is_empty() {
            obj[openai_keys::CONTENT] = json!(text_parts.join(""));
        } else {
            let mut content_arr: Vec<serde_json::Value> = text_parts
                .iter()
                .map(|t| json!({ "type": "text", "text": t }))
                .collect();
            content_arr.extend(image_parts);
            obj[openai_keys::CONTENT] = json!(content_arr);
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

        let mut body = json!({});
        body[openai_keys::MODEL] = json!(model);
        body[openai_keys::MESSAGES] = json!(messages);

        let capabilities = self.configured_capabilities(model);
        let reasoning = capabilities.uses_reasoning_token_fields;

        if !reasoning && capabilities.supports_custom_temperature != Some(false) {
            body[openai_keys::TEMPERATURE] = json!(request.temperature);
        }

        if let Some(max_tokens) = request.max_tokens {
            let key = if reasoning {
                MAX_COMPLETION_TOKENS
            } else {
                message_keys::MAX_TOKENS
            };
            body[key] = json!(max_tokens);
        }

        if !reasoning {
            if let Some(top_p) = request.top_p {
                body[openai_keys::TOP_P] = json!(top_p);
            }

            if let Some(freq_penalty) = request.frequency_penalty {
                body[openai_keys::FREQUENCY_PENALTY] = json!(freq_penalty);
            }

            if let Some(pres_penalty) = request.presence_penalty {
                body[openai_keys::PRESENCE_PENALTY] = json!(pres_penalty);
            }
        }

        if !request.stop_sequences.is_empty() {
            body[openai_keys::STOP] = json!(request.stop_sequences);
        }

        if capabilities
            .supports_structured_outputs
            .unwrap_or(self.structured_outputs_supported)
            && let Some(output_schema) = &request.output_schema
        {
            body[openai_keys::RESPONSE_FORMAT] = json!({});
            body[openai_keys::RESPONSE_FORMAT][openai_keys::RESPONSE_FORMAT_TYPE] =
                json!(openai_keys::RESPONSE_FORMAT_JSON_SCHEMA);
            body[openai_keys::RESPONSE_FORMAT][openai_keys::JSON_SCHEMA] = json!({});
            body[openai_keys::RESPONSE_FORMAT][openai_keys::JSON_SCHEMA]
                [openai_keys::JSON_SCHEMA_NAME] = json!(openai_keys::APXM_OUTPUT_SCHEMA_NAME);
            body[openai_keys::RESPONSE_FORMAT][openai_keys::JSON_SCHEMA]
                [openai_keys::JSON_SCHEMA_SCHEMA] = json!(output_schema);
            body[openai_keys::RESPONSE_FORMAT][openai_keys::JSON_SCHEMA]
                [openai_keys::JSON_SCHEMA_STRICT] = json!(true);
        }

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
            body[tool_keys::TOOLS] = json!(openai_tools);

            // Add tool_choice if specified
            if let Some(choice) = &request.tool_choice {
                body[tool_keys::TOOL_CHOICE] = match choice {
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
        if let Some(extra) = &request.extra_body
            && let serde_json::Value::Object(extra_map) = extra
            && let serde_json::Value::Object(body_map) = &mut body
        {
            for (key, value) in extra_map {
                body_map.insert(key.clone(), value.clone());
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
    fn parse_response(response: OpenAIResponse, model: &str) -> Result<LLMResponse> {
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
        let finish_reason = if tool_calls.is_empty() {
            FinishReason::from_string(&choice.finish_reason)
        } else {
            FinishReason::ToolUse
        };

        let (input_tokens, output_tokens) = response.usage.observed_token_counts()?;
        let usage = TokenUsage::new(input_tokens, output_tokens).with_details(
            response.usage.cached_input_tokens(),
            response.usage.reasoning_output_tokens(),
        );

        Ok(LLMResponse::new(content, model, usage, finish_reason).with_tool_calls(tool_calls))
    }
}

#[async_trait]
impl LLMBackend for OpenAIBackend {
    async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        request.validate_provider_dispatch()?;

        let model = self.request_model(&request).to_string();
        let body = self.build_request_body(&request);
        let url = format!("{}{}", self.base_url, api_paths::CHAT_COMPLETIONS);

        tracing::debug!(
            model = %model,
            url = %url,
            "Sending request to OpenAI"
        );

        let req_builder = self.apply_transport_headers(
            self.client
                .post(&url)
                .header(headers::CONTENT_TYPE, headers::CONTENT_TYPE_JSON),
        );

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

        // Parse raw JSON first to handle multiple response formats.
        // The OpenAI-compatible gateway returns Claude responses in Anthropic format:
        //   {"model": "...", "response": {"type": "text", "text": "..."}}
        // Standard OpenAI format:
        //   {"choices": [{"message": {"content": "..."}}]}
        let raw: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse LLM response JSON")?;

        // Try to normalize OpenAI-compatible gateway Claude format to OpenAI format
        let api_response = if raw.get("choices").is_some() {
            // Standard OpenAI format
            serde_json::from_value::<OpenAIResponse>(raw).context("Failed to parse OpenAI response")
        } else if let Some(text) = raw.pointer("/response/text").and_then(|v| v.as_str()) {
            // OpenAI-compatible gateway Claude format: {response: {type: text, text: "..."}}
            let usage = raw
                .get("usage")
                .cloned()
                .context("OpenAI-compatible gateway response omitted observed usage")?;
            let normalized = serde_json::json!({
                "choices": [{"message": {"content": text}, "finish_reason": "stop"}],
                "usage": usage
            });
            serde_json::from_value::<OpenAIResponse>(normalized)
                .context("Failed to normalize OpenAI-compatible gateway response")
        } else if let Some(content) = raw
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
        {
            // Already correct but nested differently
            let usage = raw
                .get("usage")
                .cloned()
                .context("OpenAI-compatible response omitted observed usage")?;
            let normalized = serde_json::json!({
                "choices": [{"message": {"content": content}, "finish_reason": "stop"}],
                "usage": usage
            });
            serde_json::from_value::<OpenAIResponse>(normalized)
                .context("Failed to normalize response")
        } else {
            // Unknown format — try standard parse and let it fail with useful context
            Err(anyhow::anyhow!(
                "Unrecognized LLM response format: {}",
                &raw.to_string()[..raw.to_string().len().min(200)]
            ))
        }?;

        Self::parse_response(api_response, &model)
    }

    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + '_>> {
        Box::pin(async_stream::try_stream! {
            request.validate_provider_dispatch()?;
            let model = self.request_model(&request).to_string();
            let mut body = self.build_request_body(&request);
            body[message_keys::STREAM] = json!(true);
            if body.get(openai_keys::STREAM_OPTIONS).is_none() {
                body[openai_keys::STREAM_OPTIONS] =
                    json!({ openai_keys::INCLUDE_USAGE: true });
            }

            let url = format!("{}{}", self.base_url, api_paths::CHAT_COMPLETIONS);

            tracing::debug!(
                model = %model,
                url = %url,
                "Sending streaming request to OpenAI"
            );

            let req_builder = self.apply_transport_headers(
                self.client
                    .post(&url)
                    .header(headers::CONTENT_TYPE, headers::CONTENT_TYPE_JSON),
            );

            let response = req_builder.json(&body).send().await
                .context("Failed to send streaming request to OpenAI")?
                .error_for_status()
                .map_err(|e| anyhow::anyhow!("OpenAI API error: {}", e))?;

            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut full_content = String::new();
            let mut last_usage = None;
            let mut tool_calls_map: std::collections::HashMap<usize, (String, String, String)> =
                std::collections::HashMap::new();

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

                    if let Some(data) = line.strip_prefix(sse::DATA_PREFIX) {
                        if data.trim() == sse::DONE_MARKER {
                            // Build final response.
                            let tool_calls: Vec<ToolCall> = tool_calls_map.values()
                                .map(|(id, name, args)| {
                                    let args_val = serde_json::from_str(args)
                                        .unwrap_or(json!({}));
                                    ToolCall::new(id.clone(), name.clone(), args_val)
                                })
                                .collect();

                            let finish_reason = if tool_calls.is_empty() {
                                FinishReason::Stop
                            } else {
                                FinishReason::ToolUse
                            };

                            let usage = last_usage.context("OpenAI stream completed without observed usage")?;
                            let resp = LLMResponse::new(
                                full_content.clone(),
                                &model,
                                usage,
                                finish_reason,
                            ).with_tool_calls(tool_calls);

                            yield StreamChunk::Done(resp);
                            return;
                        }

                        if let Ok(parsed) = serde_json::from_str::<StreamChunkPayload>(data) {
                            // Emit usage if present at top level.
                            if let Some(ref usage_obj) = parsed.usage {
                                let (input, output) = usage_obj.observed_token_counts()?;
                                let usage = TokenUsage::new(input, output).with_details(
                                    usage_obj.cached_input_tokens(),
                                    usage_obj.reasoning_output_tokens(),
                                );
                                last_usage = Some(usage.clone());
                                yield StreamChunk::Usage(usage);
                            }

                            for choice in &parsed.choices {
                                if let Some(ref delta) = choice.delta {
                                    // Text content.
                                    if let Some(ref content) = delta.content
                                        && !content.is_empty() {
                                            full_content.push_str(content);
                                            yield StreamChunk::Token(content.clone());
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
                                                    (id.clone(), name.clone(), String::new()),
                                                );
                                                yield StreamChunk::ToolCallStart {
                                                    id: id.clone(),
                                                    name,
                                                };
                                            }

                                            if let Some(args) = tc.function.as_ref().and_then(|f| f.arguments.as_ref())
                                                && !args.is_empty()
                                                    && let Some((id, _, accumulated_args)) =
                                                        tool_calls_map.get_mut(&index)
                                                    {
                                                        accumulated_args.push_str(args);
                                                        yield StreamChunk::ToolCallDelta {
                                                            id: id.clone(),
                                                            arguments_delta: args.clone(),
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

            Err(anyhow::anyhow!(
                "OpenAI stream ended before {} terminal marker",
                sse::DONE_MARKER
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
        let url = format!("{}{}", self.base_url, api_paths::MODELS);

        let req_builder = self.apply_transport_headers(self.client.get(&url));

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
        Ok(self.registered_models.clone())
    }

    fn capabilities(&self) -> ModelCapabilities {
        let capabilities = self.configured_capabilities(&self.model);
        ModelCapabilities {
            streaming: true,
            vision: capabilities.supports_vision,
            functions: capabilities.supports_functions,
            structured_outputs: capabilities
                .supports_structured_outputs
                .unwrap_or(self.structured_outputs_supported),
            batch: false,
            fine_tuning: capabilities.supports_fine_tuning,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_admission_requires_correlated_tool_messages() {
        let uncorrelated = LLMRequest::from_messages(vec![
            Message::text(Role::User, "status"),
            Message::text(Role::Tool, "untrusted tool context"),
        ]);
        assert!(
            uncorrelated
                .validate_provider_dispatch()
                .expect_err("tool context needs a call id")
                .to_string()
                .contains("correlated tool_call_id")
        );

        let correlated = LLMRequest::from_messages(vec![
            Message {
                role: Role::Assistant,
                content: vec![ContentPart::ToolCall {
                    id: "call_1".to_string(),
                    function: FunctionCall {
                        name: "lookup".to_string(),
                        arguments: json!({"key": "status"}),
                    },
                }],
                tool_call_id: None,
                name: None,
            },
            Message::tool_result("call_1", "available"),
        ]);
        correlated
            .validate_provider_dispatch()
            .expect("correlated provider tool exchange");
    }

    #[tokio::test]
    async fn registered_model_capabilities_control_openai_request_shape() {
        let backend = OpenAIBackend::new(
            "",
            Some(json!({
                "model": "deployment-model",
                "base_url": "https://llm.example.test/v1",
                "models": [{
                    "id": "deployment-model",
                    "supports_vision": true,
                    "supports_functions": false,
                    "supports_fine_tuning": true,
                    "supports_custom_temperature": false,
                    "uses_reasoning_token_fields": true,
                    "supports_structured_outputs": true
                }]
            })),
        )
        .await
        .expect("registered backend config");
        let mut request = LLMRequest::new("status");
        request.max_tokens = Some(64);

        let body = backend.build_request_body(&request);
        assert_eq!(body[MAX_COMPLETION_TOKENS], 64);
        assert!(body.get(message_keys::MAX_TOKENS).is_none());
        assert!(body.get(openai_keys::TEMPERATURE).is_none());

        let capabilities = backend.capabilities();
        assert!(capabilities.vision);
        assert!(!capabilities.functions);
        assert!(capabilities.structured_outputs);
        assert!(capabilities.fine_tuning);

        let models = backend.list_models().await.expect("registered models");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "deployment-model");
        assert!(models[0].supports_vision);
        assert!(!models[0].supports_functions);
    }

    #[test]
    fn openai_usage_requires_observed_input_and_output_counts() {
        let usage = Usage {
            prompt_tokens: Some(4),
            completion_tokens: None,
            input_tokens: None,
            output_tokens: None,
            prompt_tokens_details: None,
            input_tokens_details: None,
            completion_tokens_details: None,
            output_tokens_details: None,
        };

        let error = usage
            .observed_token_counts()
            .expect_err("missing output count is not observed usage");
        assert!(error.to_string().contains("omitted output token count"));
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
    prompt_tokens: Option<usize>,
    completion_tokens: Option<usize>,
    input_tokens: Option<usize>,
    output_tokens: Option<usize>,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default)]
    input_tokens_details: Option<InputTokensDetails>,
    #[serde(default)]
    completion_tokens_details: Option<CompletionTokensDetails>,
    #[serde(default)]
    output_tokens_details: Option<OutputTokensDetails>,
}

impl Usage {
    fn observed_token_counts(&self) -> Result<(usize, usize)> {
        let input = self
            .prompt_tokens
            .or(self.input_tokens)
            .context("OpenAI response usage omitted input token count")?;
        let output = self
            .completion_tokens
            .or(self.output_tokens)
            .context("OpenAI response usage omitted output token count")?;
        Ok((input, output))
    }

    fn cached_input_tokens(&self) -> usize {
        self.prompt_tokens_details
            .as_ref()
            .and_then(|details| details.cached_tokens)
            .or_else(|| {
                self.input_tokens_details
                    .as_ref()
                    .map(|details| details.cached_tokens)
            })
            .unwrap_or(0)
    }

    fn reasoning_output_tokens(&self) -> usize {
        self.completion_tokens_details
            .as_ref()
            .map(|details| details.reasoning_tokens)
            .or_else(|| {
                self.output_tokens_details
                    .as_ref()
                    .map(|details| details.reasoning_tokens)
            })
            .unwrap_or(0)
    }
}

#[derive(Debug, Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct InputTokensDetails {
    #[serde(default)]
    cached_tokens: usize,
}

#[derive(Debug, Deserialize)]
struct CompletionTokensDetails {
    #[serde(default)]
    reasoning_tokens: usize,
}

#[derive(Debug, Deserialize)]
struct OutputTokensDetails {
    #[serde(default)]
    reasoning_tokens: usize,
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
    prompt_tokens: Option<usize>,
    completion_tokens: Option<usize>,
    input_tokens: Option<usize>,
    output_tokens: Option<usize>,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default)]
    input_tokens_details: Option<InputTokensDetails>,
    #[serde(default)]
    completion_tokens_details: Option<CompletionTokensDetails>,
    #[serde(default)]
    output_tokens_details: Option<OutputTokensDetails>,
}

impl StreamUsage {
    fn observed_token_counts(&self) -> Result<(usize, usize)> {
        let input = self
            .prompt_tokens
            .or(self.input_tokens)
            .context("OpenAI stream usage omitted input token count")?;
        let output = self
            .completion_tokens
            .or(self.output_tokens)
            .context("OpenAI stream usage omitted output token count")?;
        Ok((input, output))
    }

    fn cached_input_tokens(&self) -> usize {
        self.prompt_tokens_details
            .as_ref()
            .and_then(|details| details.cached_tokens)
            .or_else(|| {
                self.input_tokens_details
                    .as_ref()
                    .map(|details| details.cached_tokens)
            })
            .unwrap_or(0)
    }

    fn reasoning_output_tokens(&self) -> usize {
        self.completion_tokens_details
            .as_ref()
            .map(|details| details.reasoning_tokens)
            .or_else(|| {
                self.output_tokens_details
                    .as_ref()
                    .map(|details| details.reasoning_tokens)
            })
            .unwrap_or(0)
    }
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
