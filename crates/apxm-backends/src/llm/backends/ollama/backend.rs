//! Ollama backend implementation (local models).
//!
//! Implements the LLMBackend trait for Ollama's local model API.
//! Uses /api/chat for full support of system prompts, multi-turn conversations,
//! streaming, and tool calling (for models that support it like llama3.1+, qwen2.5).

use crate::llm::backends::traits::StreamChunk;
use crate::llm::backends::{LLMBackend, LLMRequest, LLMResponse, Role};
use anyhow::{Context, Result};
use apxm_core::constants::graph::attrs::{BASE_URL, MODEL};
use apxm_core::types::{FinishReason, ModelCapabilities, ModelInfo, TokenUsage, ToolCall};
use async_trait::async_trait;
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use std::pin::Pin;
use tokio_stream::Stream;

const DEFAULT_BASE_URL: &str = "http://localhost:11434";
const DEFAULT_MODEL: &str = "gpt-oss:120b-cloud";

/// Known Ollama options that should be passed as integers
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

/// Known Ollama options that should be passed as floats
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

/// Ollama LLM backend (local models).
pub struct OllamaBackend {
    model: String,
    base_url: String,
    client: reqwest::Client,
    /// Ollama runtime options (num_ctx, num_gpu, etc.)
    ollama_options: serde_json::Map<String, serde_json::Value>,
}

impl OllamaBackend {
    fn request_model<'a>(&'a self, request: &'a LLMRequest) -> &'a str {
        request.model.as_deref().unwrap_or(&self.model)
    }

    /// Create a new Ollama backend.
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

        // Parse all Ollama options from config
        // These will be passed through to the Ollama API
        let mut ollama_options = serde_json::Map::new();

        if let Some(config_obj) = config.as_ref().and_then(|c| c.as_object()) {
            for (key, value) in config_obj {
                // Skip non-option fields
                if key == MODEL || key == BASE_URL {
                    continue;
                }

                // Convert string values to appropriate types for Ollama
                let converted_value = if let Some(s) = value.as_str() {
                    if INT_OPTIONS.contains(&key.as_str()) {
                        // Parse as integer
                        s.parse::<i64>()
                            .map(serde_json::Value::from)
                            .unwrap_or_else(|_| value.clone())
                    } else if FLOAT_OPTIONS.contains(&key.as_str()) {
                        // Parse as float
                        s.parse::<f64>()
                            .map(serde_json::Value::from)
                            .unwrap_or_else(|_| value.clone())
                    } else if s == "true" || s == "false" {
                        // Parse as bool
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

    /// Convert a structured Message to Ollama JSON format.
    fn message_to_ollama_json(msg: &crate::llm::backends::Message) -> serde_json::Value {
        let role = match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };

        json!({
            "role": role,
            "content": msg.text_content()
        })
    }

    /// Build request body for Ollama /api/chat endpoint.
    fn build_request_body(&self, request: &LLMRequest) -> serde_json::Value {
        let model = self.request_model(request);

        // Convert messages to Ollama format
        let messages: Vec<serde_json::Value> = request
            .resolved_messages()
            .iter()
            .map(Self::message_to_ollama_json)
            .collect();

        // Start with configured Ollama options
        let mut options = serde_json::Value::Object(self.ollama_options.clone());

        // Override with request-specific options
        options["temperature"] = json!(request.temperature);

        if let Some(max_tokens) = request.max_tokens {
            options["num_predict"] = json!(max_tokens);
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

        // Add tools if provided (OpenAI-style format)
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

        tracing::debug!(
            options = %options,
            "Building Ollama /api/chat request"
        );

        body
    }

    /// Parse Ollama /api/chat response.
    fn parse_response(&self, response: OllamaChatResponse, model: &str) -> Result<LLMResponse> {
        // Ollama uses approximate token counts
        let input_tokens = response.prompt_eval_count.unwrap_or(0);
        let output_tokens = response.eval_count.unwrap_or(0);

        let usage = TokenUsage::new(input_tokens, output_tokens);

        // Extract content from message
        let content = response.message.content.clone();

        // Parse tool calls if present
        let tool_calls: Vec<ToolCall> = response
            .message
            .tool_calls
            .as_ref()
            .map(|calls| {
                calls
                    .iter()
                    .enumerate()
                    .map(|(idx, tc)| {
                        // Ollama uses OpenAI-style tool calls
                        let id = format!("call_{}", idx);
                        ToolCall::new(id, tc.function.name.clone(), tc.function.arguments.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Determine finish reason
        let finish_reason = if !tool_calls.is_empty() {
            FinishReason::ToolUse
        } else if response.done {
            FinishReason::Stop
        } else {
            FinishReason::Unknown
        };

        Ok(LLMResponse::new(content, model, usage, finish_reason).with_tool_calls(tool_calls))
    }
}

#[async_trait]
impl LLMBackend for OllamaBackend {
    async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        request.validate()?;

        let model = self.request_model(&request).to_string();
        let body = self.build_request_body(&request);
        let url = format!("{}/api/chat", self.base_url);

        tracing::debug!(
            model = %model,
            url = %url,
            "Sending request to Ollama /api/chat"
        );

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
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

            let url = format!("{}/api/chat", self.base_url);

            tracing::debug!(
                model = %model,
                url = %url,
                "Sending streaming request to Ollama /api/chat"
            );

            let response = self.client
                .post(&url)
                .header("Content-Type", "application/json")
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

                // Process NDJSON lines (each line is a complete JSON object)
                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer.drain(..line_end + 1);

                    if line.is_empty() {
                        continue;
                    }

                    if let Ok(parsed) = serde_json::from_str::<OllamaChatStreamChunk>(&line) {
                        // Update token usage if present
                        if let Some(prompt_count) = parsed.prompt_eval_count {
                            if let Some(eval_count) = parsed.eval_count {
                                last_usage = TokenUsage::new(prompt_count, eval_count);
                                yield StreamChunk::Usage(last_usage.clone());
                            }
                        }

                        // Process message content
                        if let Some(ref content) = parsed.message.content {
                            if !content.is_empty() {
                                full_content.push_str(content);
                                yield StreamChunk::Token(content.to_string());
                            }
                        }

                        // Process tool calls
                        if let Some(ref tc_arr) = parsed.message.tool_calls {
                            for (idx, tc) in tc_arr.iter().enumerate() {
                                // Check if this is a new tool call
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

                        // Check if done
                        if parsed.done {
                            // Build final response
                            let tool_calls: Vec<ToolCall> = tool_calls_map.iter()
                                .map(|(idx, (name, args))| {
                                    ToolCall::new(format!("call_{}", idx), name.clone(), args.clone())
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
                    }
                }
            }

            // Stream ended without done=true -- emit what we have
            let tool_calls: Vec<ToolCall> = tool_calls_map.iter()
                .map(|(idx, (name, args))| {
                    ToolCall::new(format!("call_{}", idx), name.clone(), args.clone())
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
        "ollama"
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn health_check(&self) -> Result<()> {
        let url = format!("{}/api/tags", self.base_url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to connect to Ollama")?;

        if !response.status().is_success() {
            anyhow::bail!("Ollama health check failed: {}", response.status());
        }

        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}/api/tags", self.base_url);

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
                // Models like llama3.1, qwen2.5, etc. support tool calling
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
            batch: false,
            fine_tuning: false,
        }
    }
}

// Ollama /api/chat response types
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
    role: String,
    content: String,
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

// Ollama streaming chunk (NDJSON format)
#[derive(Debug, Deserialize)]
struct OllamaChatStreamChunk {
    message: OllamaStreamMessage,
    done: bool,
    #[serde(default)]
    prompt_eval_count: Option<usize>,
    #[serde(default)]
    eval_count: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct OllamaStreamMessage {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OllamaTags {
    models: Vec<OllamaModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaModel {
    name: String,
}
