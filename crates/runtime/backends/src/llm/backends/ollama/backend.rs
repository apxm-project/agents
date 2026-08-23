//! Ollama backend implementation (local models).

use crate::llm::ProviderProtocol;
use crate::llm::backends::http::{
    llm_http_client, read_provider_error_body, read_provider_json, require_provider_success,
};
use crate::llm::backends::openai::backend::validate_provider_dispatch;
use crate::llm::backends::traits::StreamChunk;
use crate::llm::backends::{
    ConfiguredModelCapabilities, configured_model_capabilities, required_config_string,
};
use crate::llm::backends::{LLMBackend, LLMRequest, LLMResponse, Role};
use crate::llm::wire::{
    api_paths, config_keys, message_keys, ollama as ollama_keys, response_metadata,
};
use anyhow::{Context, Result};
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::types::{FinishReason, ModelCapabilities, ModelInfo, TokenUsage, ToolCall};
use async_trait::async_trait;
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::pin::Pin;
use tokio_stream::Stream;

const PROTOCOL: ProviderProtocol = ProviderProtocol::Ollama;

const INT_OPTIONS: &[&str] = &[
    "num_ctx",
    "num_gpu",
    "num_thread",
    "num_keep",
    "num_predict",
    "num_batch",
    "main_gpu",
    "seed",
    "mirostat",
];

const FLOAT_OPTIONS: &[&str] = &[
    "temperature",
    "top_p",
    "top_k",
    "tfs_z",
    "typical_p",
    "repeat_penalty",
    "presence_penalty",
    "frequency_penalty",
    "mirostat_tau",
    "mirostat_eta",
];

pub struct OllamaBackend {
    model: String,
    base_url: String,
    client: reqwest::Client,
    ollama_options: serde_json::Map<String, serde_json::Value>,
    /// Registered per-model capability evidence.
    model_capabilities: HashMap<String, ConfiguredModelCapabilities>,
    model_supports_thinking: HashMap<String, bool>,
}

impl OllamaBackend {
    fn configured_capabilities(&self, model: &str) -> ConfiguredModelCapabilities {
        self.model_capabilities
            .get(model)
            .copied()
            .unwrap_or_default()
    }

    fn request_model<'a>(&'a self, request: &'a LLMRequest) -> &'a str {
        request.model.as_deref().unwrap_or(&self.model)
    }

    pub async fn new(_api_key: &str, config: Option<serde_json::Value>) -> Result<Self> {
        let model = required_config_string(config.as_ref(), PROTOCOL, MODEL)?;
        let base_url = required_config_string(config.as_ref(), PROTOCOL, BASE_URL)?;

        let mut ollama_options = serde_json::Map::new();
        let model_capabilities = configured_model_capabilities(config.as_ref());
        let mut model_supports_thinking = HashMap::new();

        if let Some(models) = config
            .as_ref()
            .and_then(|c| c.get(config_keys::MODELS))
            .and_then(|models| models.as_array())
        {
            for entry in models {
                let Some(model_id) = entry.get(config_keys::ID).and_then(|value| value.as_str())
                else {
                    continue;
                };
                let supports = entry
                    .get(config_keys::SUPPORTS_THINKING)
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                model_supports_thinking.insert(model_id.to_string(), supports);
            }
        }

        if let Some(config_obj) = config.as_ref().and_then(|c| c.as_object()) {
            for (key, value) in config_obj {
                if key == MODEL || key == BASE_URL {
                    continue;
                }

                let converted_value = if let Some(s) = value.as_str() {
                    if INT_OPTIONS.contains(&key.as_str()) {
                        s.parse::<i64>()
                            .map_or_else(|_| value.clone(), serde_json::Value::from)
                    } else if FLOAT_OPTIONS.contains(&key.as_str()) {
                        s.parse::<f64>()
                            .map_or_else(|_| value.clone(), serde_json::Value::from)
                    } else if s == "true" || s == "false" {
                        serde_json::Value::Bool(s == "true")
                    } else {
                        value.clone()
                    }
                } else {
                    value.clone()
                };

                ollama_options.insert(key.clone(), converted_value);
            }
        }

        tracing::debug!(
            model = %model,
            base_url = %base_url,
            ollama_options = ?ollama_options,
            "Creating Ollama backend"
        );

        Ok(OllamaBackend {
            model,
            base_url,
            client: llm_http_client(),
            ollama_options,
            model_capabilities,
            model_supports_thinking,
        })
    }

    fn should_think(&self, request: &LLMRequest) -> bool {
        match request.enable_thinking {
            Some(true) => true,
            Some(false) => false,
            None => self
                .model_supports_thinking
                .get(self.request_model(request))
                .copied()
                .unwrap_or(false),
        }
    }

    fn build_request_body(&self, request: &LLMRequest) -> serde_json::Value {
        let model = self.request_model(request);

        let messages: Vec<serde_json::Value> = request
            .resolved_messages()
            .iter()
            .map(|msg| {
                let role = match msg.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };
                json!({ "role": role, "content": msg.text_content() })
            })
            .collect();

        let mut options = serde_json::Value::Object(self.ollama_options.clone());
        options["temperature"] = json!(request.temperature);

        if let Some(max_tokens) = request.max_tokens {
            options[ollama_keys::NUM_PREDICT] = json!(max_tokens);
        }

        if let Some(top_p) = request.top_p {
            options["top_p"] = json!(top_p);
        }

        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "options": options
        });

        if let Some(tools) = &request.tools
            && !tools.is_empty()
        {
            let ollama_tools: Vec<serde_json::Value> = tools
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
            body["tools"] = json!(ollama_tools);
        }

        if self.should_think(request) {
            body[ollama_keys::THINK] = json!(true);
        }

        body
    }

    fn collect_tool_calls(
        tool_calls_map: &std::collections::HashMap<usize, (String, serde_json::Value)>,
    ) -> (Vec<ToolCall>, FinishReason) {
        let tool_calls: Vec<ToolCall> = tool_calls_map
            .iter()
            .map(|(idx, (name, args))| {
                ToolCall::new(format!("call_{}", idx), name.clone(), args.clone())
            })
            .collect();

        let finish_reason = if tool_calls.is_empty() {
            FinishReason::Stop
        } else {
            FinishReason::ToolUse
        };

        (tool_calls, finish_reason)
    }
}

#[async_trait]
impl LLMBackend for OllamaBackend {
    async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        validate_provider_dispatch(&request)?;

        let model = self.request_model(&request).to_string();
        let body = self.build_request_body(&request);
        let url = format!("{}{}", self.base_url, api_paths::API_CHAT);

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("Failed to send request to Ollama")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = read_provider_error_body(response, &[]).await;
            anyhow::bail!("Ollama API error (status {}): {}", status, error_text);
        }

        let api_response: OllamaChatResponse =
            read_provider_json(response, "Failed to parse Ollama response", &[]).await?;

        let usage = TokenUsage::new(
            api_response
                .prompt_eval_count
                .context("Ollama response omitted observed prompt_eval_count")?,
            api_response
                .eval_count
                .context("Ollama response omitted observed eval_count")?,
        );

        let content = api_response.message.content.unwrap_or_default();

        let tool_calls: Vec<ToolCall> = api_response
            .message
            .tool_calls
            .as_ref()
            .map(|calls| {
                calls
                    .iter()
                    .enumerate()
                    .map(|(idx, tc)| {
                        ToolCall::new(
                            format!("call_{}", idx),
                            tc.function.name.clone(),
                            tc.function.arguments.clone(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        let finish_reason = if !tool_calls.is_empty() {
            FinishReason::ToolUse
        } else if api_response.done {
            FinishReason::Stop
        } else {
            FinishReason::Unknown
        };

        let mut response =
            LLMResponse::new(content, &model, usage, finish_reason).with_tool_calls(tool_calls);
        if let Some(reasoning) = api_response
            .message
            .thinking
            .filter(|text| !text.is_empty())
        {
            response =
                response.with_metadata(response_metadata::REASONING.to_string(), json!(reasoning));
        }
        Ok(response)
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

            let url = format!("{}{}", self.base_url, api_paths::API_CHAT);

            let response = self.client
                .post(&url)
                .json(&body)
                .send()
                .await
                .context("Failed to send streaming request to Ollama")?;
            let response = require_provider_success(response, "Ollama", &[]).await?;

            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut full_content = String::new();
            let mut full_reasoning = String::new();
            let mut last_usage = None;
            let mut tool_calls_map: std::collections::HashMap<usize, (String, serde_json::Value)> =
                std::collections::HashMap::new();

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result.context("Stream read error")?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer.drain(..=line_end);

                    if line.is_empty() {
                        continue;
                    }

                    if let Ok(parsed) = serde_json::from_str::<OllamaChatResponse>(&line) {
                        if let Some(prompt_count) = parsed.prompt_eval_count
                            && let Some(eval_count) = parsed.eval_count {
                                let usage = TokenUsage::new(prompt_count, eval_count);
                                last_usage = Some(usage.clone());
                                yield StreamChunk::Usage(usage);
                            }

                        if let Some(ref thinking) = parsed.message.thinking
                            && !thinking.is_empty() {
                                full_reasoning.push_str(thinking);
                                yield StreamChunk::Thought(thinking.clone());
                            }

                        if let Some(ref content) = parsed.message.content
                            && !content.is_empty() {
                                full_content.push_str(content);
                                yield StreamChunk::Token(content.clone());
                            }

                        if let Some(ref tc_arr) = parsed.message.tool_calls {
                            for (idx, tc) in tc_arr.iter().enumerate() {
                                if let std::collections::hash_map::Entry::Vacant(e) = tool_calls_map.entry(idx) {
                                    e.insert((tc.function.name.clone(), tc.function.arguments.clone()));
                                    yield StreamChunk::ToolCallStart {
                                        id: format!("call_{}", idx),
                                        name: tc.function.name.clone(),
                                    };
                                }
                            }
                        }

                        if parsed.done {
                            let (tool_calls, finish_reason) = Self::collect_tool_calls(&tool_calls_map);
                            let usage = last_usage.context("Ollama stream completed without observed usage")?;
                            let mut resp = LLMResponse::new(
                                full_content.clone(),
                                &model,
                                usage,
                                finish_reason,
                            ).with_tool_calls(tool_calls);
                            if let Some(reasoning) = (!full_reasoning.is_empty())
                                .then(|| full_reasoning.clone())
                                .or_else(|| {
                                    parsed
                                        .message
                                        .thinking
                                        .filter(|text| !text.is_empty())
                                })
                            {
                                resp = resp.with_metadata(
                                    response_metadata::REASONING.to_string(),
                                    json!(reasoning),
                                );
                            }
                            yield StreamChunk::Done(resp);
                            return;
                        }
                    }
                }
            }

            Err(anyhow::anyhow!(
                "Ollama stream ended before done=true terminal message"
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
        let url = format!("{}{}", self.base_url, api_paths::API_TAGS);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to connect to Ollama")?;
        require_provider_success(response, "Ollama health check", &[]).await?;
        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}{}", self.base_url, api_paths::API_TAGS);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to list Ollama models")?;

        let response = require_provider_success(response, "Ollama model listing", &[]).await?;
        let tags: OllamaTags =
            read_provider_json(response, "Failed to parse Ollama model listing", &[]).await?;

        let models = tags
            .models
            .into_iter()
            .map(|m| {
                let capabilities = self.configured_capabilities(&m.name);

                ModelInfo {
                    id: m.name.clone(),
                    name: m.name,
                    context_window: capabilities.context_window,
                    supports_vision: capabilities.supports_vision,
                    supports_functions: capabilities.supports_functions,
                }
            })
            .collect();

        Ok(models)
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

    fn response_memoization_policy(
        &self,
    ) -> crate::llm::backends::traits::ResponseMemoizationPolicy {
        crate::llm::backends::traits::ResponseMemoizationPolicy::BackendPrefix
    }
}

// Unified response type for both streaming chunks and full responses.
// Ollama NDJSON streaming uses the same shape with optional fields.
#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaMessage,
    done: bool,
    #[serde(default)]
    prompt_eval_count: Option<usize>,
    #[serde(default)]
    eval_count: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct OllamaMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OllamaToolCall {
    function: OllamaFunction,
}

#[derive(Debug, Deserialize)]
struct OllamaFunction {
    name: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OllamaTags {
    models: Vec<OllamaModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaModel {
    name: String,
}
