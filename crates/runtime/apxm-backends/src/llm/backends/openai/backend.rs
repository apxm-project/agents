//! OpenAI backend implementation.
//!
//! Implements the LLMBackend trait for OpenAI's API, supporting modern OpenAI
//! model identifiers (gpt-4o, gpt-4o-mini, gpt-4-turbo, gpt-4, gpt-3.5-turbo, etc.).
//!
//! This file updates the provider default model and the list of known models
//! surfaced by `list_models()` to reflect more recent model names.

use crate::llm::ProviderProtocol;
use crate::llm::backends::http::llm_http_client;
use crate::llm::backends::traits::StreamChunk;
use crate::llm::backends::{ContentPart, LLMBackend, LLMRequest, LLMResponse, Role, ToolChoice};
use crate::llm::catalog::{default_model_for_protocol, models_for_protocol};
use crate::llm::wire::{
    api_paths, config_keys, headers, message_keys, openai as openai_keys, roles, sse, tool_keys,
};
use anyhow::{Context, Result};
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::constants::llm::apxm::{APXM_FIELDS_HONORED_HEADER, FIELDS_HONORED_RECORD_KEY};
use apxm_core::types::{FinishReason, ModelCapabilities, ModelInfo, TokenUsage, ToolCall};
use async_trait::async_trait;
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::pin::Pin;
use tokio_stream::Stream;

/// Parse the APXM `x-apxm-fields-honored` response header, if present.
///
/// The APXM vLLM fork emits this header on every per-request response,
/// listing the `vllm_xargs.apxm` fields the scheduler
/// actually consumed in scheduling decisions (e.g.
/// `x-apxm-fields-honored: priority,prefix_cohorts,pin_release`). The
/// APXM runtime stores this in `LLMResponse.metadata["fields_honored"]`
/// so per-execution aggregation can distinguish "the backend declares
/// it CAN honor this field" (capability table) from "the backend
/// reported it DID honor this field on this request" (runtime evidence).
///
/// Vanilla OpenAI and non-fork backends do not emit this header; the
/// function returns `None`, and `fields_honored` is absent from metadata
/// — which is the honest signal that no runtime evidence is available.
fn parse_apxm_fields_honored_header(response: &reqwest::Response) -> Option<Vec<String>> {
    let raw = response
        .headers()
        .get(APXM_FIELDS_HONORED_HEADER)?
        .to_str()
        .ok()?;
    parse_apxm_fields_honored_value(raw)
}

/// Pure parser for the comma-separated `x-apxm-fields-honored` value.
/// Empty strings, missing entries, and whitespace-only entries all
/// collapse to `None` rather than `Some(vec![])` — an empty list and
/// "header was absent" are equivalent honesty signals (the backend
/// reported nothing was honored), so collapsing them keeps the
/// `LLMResponse.metadata` shape consistent.
fn parse_apxm_fields_honored_value(raw: &str) -> Option<Vec<String>> {
    let fields: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    (!fields.is_empty()).then_some(fields)
}

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const PROTOCOL: ProviderProtocol = ProviderProtocol::OpenAI;
const DEFAULT_MODEL: &str = "gpt-4o-mini";
/// Output-budget key for reasoning-family models (the classic key is `max_tokens`).
const MAX_COMPLETION_TOKENS: &str = "max_completion_tokens";

/// Reasoning-family OpenAI/Azure models (gpt-5*, o1/o3/o4*) require
/// `max_completion_tokens` and reject custom sampling parameters. Matched by id
/// prefix so new point releases (e.g. `gpt-5.6`, `o5`) are covered without a list.
fn is_reasoning_model(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.starts_with("gpt-5")
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
        || m.starts_with("o5")
}

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
    /// Model IDs declared with `supports_custom_temperature = false` in
    /// backend registration config. These models reject an explicit custom
    /// `temperature` field, so the request must rely on the provider default.
    fixed_temperature_models: HashSet<String>,
    /// Whether this OpenAI-compatible endpoint accepts structured-output
    /// request fields. Runtime schema validation still applies when disabled.
    structured_outputs_supported: bool,
    client: reqwest::Client,
}

impl OpenAIBackend {
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
            .unwrap_or_else(|| default_model_for_protocol(PROTOCOL).unwrap_or(DEFAULT_MODEL))
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

        // Parse per-model capability entries forwarded by BackendRegistration.
        // The OpenAI-compatible adapter only consults protocol-level request
        // shaping flags here; provider-specific body shaping belongs in the
        // concrete backend specialization.
        let fixed_temperature_models: HashSet<String> = config
            .as_ref()
            .and_then(|c| c.get(config_keys::MODELS))
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|entry| {
                        let id = entry.get(config_keys::ID).and_then(|v| v.as_str())?;
                        let supports = entry
                            .get(config_keys::SUPPORTS_CUSTOM_TEMPERATURE)
                            .and_then(|v| v.as_bool())?;
                        if supports { None } else { Some(id.to_string()) }
                    })
                    .collect()
            })
            .unwrap_or_default();

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
            fixed_temperature_models,
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

        // Reasoning-family models (gpt-5*, o1/o3/o4*) on the OpenAI / Azure API use
        // `max_completion_tokens` instead of `max_tokens` and reject custom
        // `temperature`, `top_p`, and penalties — only provider defaults are
        // allowed. Older chat models (gpt-4*, gpt-4o*, and OpenAI-compatible
        // gateways like Kimi/DeepSeek) keep the classic parameters.
        let reasoning = is_reasoning_model(model);

        if !reasoning && !self.fixed_temperature_models.contains(model) {
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

        if self.structured_outputs_supported
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

        let input_tokens = response.usage.input_token_count();
        let output_tokens = response.usage.output_token_count();
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
        request.validate()?;

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

        // Capture per-request runtime honor evidence before consuming
        // the body. The APXM vLLM fork emits this header from its
        // scheduler; vanilla OpenAI / non-fork backends
        // return None and `fields_honored` remains absent in metadata.
        let fields_honored = parse_apxm_fields_honored_header(&response);

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
            let normalized = serde_json::json!({
                "choices": [{"message": {"content": text}, "finish_reason": "stop"}],
                "usage": raw.get("usage").cloned().unwrap_or(serde_json::json!({"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}))
            });
            serde_json::from_value::<OpenAIResponse>(normalized)
                .context("Failed to normalize OpenAI-compatible gateway response")
        } else if let Some(content) = raw
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
        {
            // Already correct but nested differently
            let normalized = serde_json::json!({
                "choices": [{"message": {"content": content}, "finish_reason": "stop"}],
                "usage": raw.get("usage").cloned().unwrap_or(serde_json::json!({"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}))
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

        let mut llm_response = self.parse_response(api_response, &model)?;
        if let Some(honored) = fields_honored {
            llm_response.metadata.insert(
                FIELDS_HONORED_RECORD_KEY.to_owned(),
                serde_json::Value::Array(
                    honored.into_iter().map(serde_json::Value::String).collect(),
                ),
            );
        }
        Ok(llm_response)
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
                                let input = usage_obj.input_token_count();
                                let output = usage_obj.output_token_count();
                                if input > 0 || output > 0 {
                                    last_usage = TokenUsage::new(input, output).with_details(
                                        usage_obj.cached_input_tokens(),
                                        usage_obj.reasoning_output_tokens(),
                                    );
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
        Ok(models_for_protocol(PROTOCOL)
            .map(|m| ModelInfo {
                id: m.id.to_string(),
                name: m.id.to_string(),
                context_window: 128_000,
                supports_vision: m.id.contains("4o")
                    || m.id.contains("5")
                    || m.id.contains("turbo"),
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
            structured_outputs: self.structured_outputs_supported,
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
    #[serde(default)]
    prompt_tokens: usize,
    #[serde(default)]
    completion_tokens: Option<usize>,
    #[serde(default)]
    input_tokens: usize,
    #[serde(default)]
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
    fn input_token_count(&self) -> usize {
        if self.prompt_tokens > 0 {
            self.prompt_tokens
        } else {
            self.input_tokens
        }
    }

    fn output_token_count(&self) -> usize {
        self.completion_tokens
            .or(self.output_tokens)
            .unwrap_or_default()
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
    #[serde(default)]
    prompt_tokens: usize,
    #[serde(default)]
    completion_tokens: Option<usize>,
    #[serde(default)]
    input_tokens: usize,
    #[serde(default)]
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
    fn input_token_count(&self) -> usize {
        if self.prompt_tokens > 0 {
            self.prompt_tokens
        } else {
            self.input_tokens
        }
    }

    fn output_token_count(&self) -> usize {
        self.completion_tokens
            .or(self.output_tokens)
            .unwrap_or_default()
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
