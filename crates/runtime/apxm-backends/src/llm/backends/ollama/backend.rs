//! Ollama backend implementation (local models).

use crate::llm::backends::traits::StreamChunk;
use crate::llm::backends::{LLMBackend, LLMRequest, LLMResponse, Role};
use anyhow::{Context, Result};
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::constants::llm::{api_paths, ollama as ollama_keys};
use apxm_core::types::{FinishReason, ModelCapabilities, ModelInfo, TokenUsage, ToolCall};
use async_trait::async_trait;
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use std::pin::Pin;
use tokio_stream::Stream;

const DEFAULT_BASE_URL: &str = "http://localhost:11434";
const DEFAULT_MODEL: &str = "gpt-oss:120b-cloud";

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
}

impl OllamaBackend {
    fn request_model<'a>(&'a self, request: &'a LLMRequest) -> &'a str {
        request.model.as_deref().unwrap_or(&self.model)
    }

    pub async fn new(_api_key: &str, config: Option<serde_json::Value>) -> Result<Self> {
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

        let mut ollama_options = serde_json::Map::new();

        if let Some(config_obj) = config.as_ref().and_then(|c| c.as_object()) {
            for (key, value) in config_obj {
                if key == MODEL || key == BASE_URL {
                    continue;
                }

                let converted_value = if let Some(s) = value.as_str() {
                    if INT_OPTIONS.contains(&key.as_str()) {
                        s.parse::<i64>()
                            .map(serde_json::Value::from)
                            .unwrap_or_else(|_| value.clone())
                    } else if FLOAT_OPTIONS.contains(&key.as_str()) {
                        s.parse::<f64>()
                            .map(serde_json::Value::from)
                            .unwrap_or_else(|_| value.clone())
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
            client: reqwest::Client::new(),
            ollama_options,
        })
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

        let finish_reason = if !tool_calls.is_empty() {
            FinishReason::ToolUse
        } else {
            FinishReason::Stop
        };

        (tool_calls, finish_reason)
    }
}

#[async_trait]
impl LLMBackend for OllamaBackend {
    async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        request.validate()?;

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
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("Ollama API error (status {}): {}", status, error_text);
        }

        let api_response: OllamaChatResponse = response
            .json()
            .await
            .context("Failed to parse Ollama response")?;

        let usage = TokenUsage::new(
            api_response.prompt_eval_count.unwrap_or(0),
            api_response.eval_count.unwrap_or(0),
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

        Ok(LLMResponse::new(content, &model, usage, finish_reason).with_tool_calls(tool_calls))
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

            let url = format!("{}{}", self.base_url, api_paths::API_CHAT);

            let response = self.client
                .post(&url)
                .json(&body)
                .send()
                .await
                .context("Failed to send streaming request to Ollama")?
                .error_for_status()
                .map_err(|e| anyhow::anyhow!("Ollama API error: {}", e))?;

            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut full_content = String::new();
            let mut last_usage = TokenUsage::new(0, 0);
            let mut tool_calls_map: std::collections::HashMap<usize, (String, serde_json::Value)> =
                std::collections::HashMap::new();

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result.context("Stream read error")?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer.drain(..line_end + 1);

                    if line.is_empty() {
                        continue;
                    }

                    if let Ok(parsed) = serde_json::from_str::<OllamaChatResponse>(&line) {
                        if let Some(prompt_count) = parsed.prompt_eval_count {
                            if let Some(eval_count) = parsed.eval_count {
                                last_usage = TokenUsage::new(prompt_count, eval_count);
                                yield StreamChunk::Usage(last_usage.clone());
                            }
                        }

                        if let Some(ref content) = parsed.message.content {
                            if !content.is_empty() {
                                full_content.push_str(content);
                                yield StreamChunk::Token(content.to_string());
                            }
                        }

                        if let Some(ref tc_arr) = parsed.message.tool_calls {
                            for (idx, tc) in tc_arr.iter().enumerate() {
                                if !tool_calls_map.contains_key(&idx) {
                                    tool_calls_map.insert(
                                        idx,
                                        (tc.function.name.clone(), tc.function.arguments.clone()),
                                    );
                                    yield StreamChunk::ToolCallStart {
                                        id: format!("call_{}", idx),
                                        name: tc.function.name.clone(),
                                    };
                                }
                            }
                        }

                        if parsed.done {
                            let (tool_calls, finish_reason) = Self::collect_tool_calls(&tool_calls_map);
                            let resp = LLMResponse::new(
                                full_content.clone(),
                                &model,
                                last_usage.clone(),
                                finish_reason,
                            ).with_tool_calls(tool_calls);
                            yield StreamChunk::Done(resp);
                            return;
                        }
                    }
                }
            }

            // Stream ended without done=true -- emit what we have
            let (tool_calls, finish_reason) = Self::collect_tool_calls(&tool_calls_map);
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
        "ollama"
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn health_check(&self) -> Result<()> {
        let url = format!("{}{}", self.base_url, api_paths::API_TAGS);
        self.client
            .get(&url)
            .send()
            .await
            .context("Failed to connect to Ollama")?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("Ollama health check failed: {}", e))?;
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

        if !response.status().is_success() {
            anyhow::bail!("Failed to list models: {}", response.status());
        }

        let tags: OllamaTags = response.json().await?;

        let models = tags
            .models
            .into_iter()
            .map(|m| {
                let supports_functions = m.name.contains("llama3.1")
                    || m.name.contains("llama3.2")
                    || m.name.contains("llama3.3")
                    || m.name.contains("qwen2.5")
                    || m.name.contains("mistral");

                ModelInfo {
                    id: m.name.clone(),
                    name: m.name,
                    context_window: 128_000,
                    supports_vision: false,
                    supports_functions,
                }
            })
            .collect();

        Ok(models)
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            streaming: true,
            vision: false,
            functions: true,
            structured_outputs: false,
            batch: false,
            fine_tuning: false,
        }
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
