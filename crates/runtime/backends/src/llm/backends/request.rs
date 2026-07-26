//! LLM request types and builders.

use apxm_core::types::{AISOperationType, ApxmGraphHints};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const DEFAULT_TEMPERATURE: f64 = 0.7;

/// Definition of an LLM-callable tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Unique name of the tool
    pub name: String,
    /// Human-readable description of what the tool does
    pub description: String,
    /// JSON Schema for the tool's input parameters
    pub parameters: serde_json::Value,
}

impl ToolDefinition {
    /// Create a new tool definition.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

/// Controls how the LLM should use tools.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum ToolChoice {
    /// Let the model decide whether to use tools
    #[default]
    Auto,
    /// Don't use any tools
    None,
    /// Force the model to use a tool
    Required,
    /// Force the model to use a specific tool
    Specific(String),
}

/// Role of a message participant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A single content part within a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    Image { url: String, detail: Option<String> },
    ToolCall { id: String, function: FunctionCall },
}

/// A function call reference inside a tool-call content part.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// A structured message with role and content parts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    /// Create a simple text message with the given role.
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![ContentPart::Text { text: text.into() }],
            tool_call_id: None,
            name: None,
        }
    }

    /// Create a tool-result message.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: vec![ContentPart::Text {
                text: content.into(),
            }],
            tool_call_id: Some(tool_call_id.into()),
            name: None,
        }
    }

    /// Return the concatenated text content of this message.
    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

/// Request to send to an LLM backend.
#[derive(Debug, Clone)]
pub struct LLMRequest {
    /// The main prompt/input text (the canonical single-string field).
    ///
    /// When `messages` is empty, backends use this field as a single User message.
    /// When `messages` is non-empty, backends prefer `messages` and ignore `prompt`.
    pub prompt: String,
    /// Structured conversation messages (preferred over `prompt` when non-empty).
    pub messages: Vec<Message>,
    /// Optional system prompt providing context
    pub system_prompt: Option<String>,
    /// Temperature controls randomness (0.0-2.0, typical: 0.7)
    pub temperature: f64,
    /// Maximum tokens to generate
    pub max_tokens: Option<usize>,
    /// Exact input-token estimate produced by the runtime context planner.
    ///
    /// This remains provider-neutral admission evidence and is never sent as a
    /// provider request field.
    pub context_input_tokens: Option<usize>,
    /// Nucleus sampling - keep top p probability mass
    pub top_p: Option<f64>,
    /// Frequency penalty - penalize repeated tokens
    pub frequency_penalty: Option<f64>,
    /// Presence penalty - encourage new topics
    pub presence_penalty: Option<f64>,
    /// Stop sequences where generation stops
    pub stop_sequences: Vec<String>,
    /// JSON schema that the backend should enforce when supported.
    pub output_schema: Option<serde_json::Value>,
    /// Optional hidden-reasoning budget for backends/models that support it.
    pub thinking_token_budget: Option<u64>,
    /// Optional explicit thinking-mode control for backends/models that support it.
    pub enable_thinking: Option<bool>,
    /// Custom metadata passed through to provider
    pub metadata: HashMap<String, serde_json::Value>,
    /// The exact backend the request names. When set it must be the backend
    /// the model reference is bound to.
    pub backend: Option<String>,
    /// The exact model reference the request names.
    pub model: Option<String>,
    /// AIS operation type this request was issued for.
    pub operation_type: Option<AISOperationType>,
    /// Tools available for the LLM to call
    pub tools: Option<Vec<ToolDefinition>>,
    /// How the LLM should use tools
    pub tool_choice: Option<ToolChoice>,
    /// Trace ID for cross-process event correlation.
    pub trace_id: Option<String>,
    /// APXM graph scheduling metadata. Backends may lower this into their
    /// own request controls when they support graph-aware execution.
    pub apxm_hints: Option<ApxmGraphHints>,
    /// Extra body fields to send to the LLM (provider-specific).
    pub extra_body: Option<serde_json::Value>,
}

impl LLMRequest {
    /// Create a new request with just a prompt.
    ///
    /// The text is stored in the `prompt` field — the dominant executor path.
    /// Backends auto-wrap it as a single User message when `messages` is empty.
    pub fn new(prompt: impl Into<String>) -> Self {
        LLMRequest {
            prompt: prompt.into(),
            messages: Vec::new(),
            system_prompt: None,
            temperature: DEFAULT_TEMPERATURE,
            max_tokens: None,
            context_input_tokens: None,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop_sequences: Vec::new(),
            output_schema: None,
            thinking_token_budget: None,
            enable_thinking: None,
            metadata: HashMap::new(),
            backend: None,
            model: None,
            operation_type: None,
            tools: None,
            tool_choice: None,
            trace_id: None,
            apxm_hints: None,
            extra_body: None,
        }
    }

    /// Create a new request from structured messages.
    pub fn from_messages(messages: Vec<Message>) -> Self {
        let prompt = messages
            .iter()
            .find(|m| m.role == Role::User)
            .map(|m| m.text_content())
            .unwrap_or_default();
        LLMRequest {
            prompt,
            messages,
            ..Self::new("")
        }
    }

    /// Set structured messages on this request.
    pub fn with_messages(mut self, messages: Vec<Message>) -> Self {
        self.messages = messages;
        self
    }

    /// Add a single message to the conversation.
    pub fn add_message(mut self, message: Message) -> Self {
        self.messages.push(message);
        self
    }

    /// Return true when this request carries structured messages.
    pub fn has_messages(&self) -> bool {
        !self.messages.is_empty()
    }

    /// Resolve messages for backend consumption.
    ///
    /// Structured messages do not erase the separately assembled trusted
    /// system/developer context. When both are present the system message is
    /// always first, followed by the ordered structured conversation frames.
    /// Otherwise this synthesizes a user message from `prompt`.
    pub fn resolved_messages(&self) -> Vec<Message> {
        if !self.messages.is_empty() {
            let mut messages =
                Vec::with_capacity(self.messages.len() + usize::from(self.system_prompt.is_some()));
            if let Some(system) = &self.system_prompt {
                messages.push(Message::text(Role::System, system.clone()));
            }
            messages.extend(self.messages.clone());
            return messages;
        }
        let mut msgs = Vec::new();
        if let Some(system) = &self.system_prompt {
            msgs.push(Message::text(Role::System, system.clone()));
        }
        msgs.push(Message::text(Role::User, &self.prompt));
        msgs
    }

    /// Set the system prompt.
    pub fn with_system_prompt(mut self, system: impl Into<String>) -> Self {
        self.system_prompt = Some(system.into());
        self
    }

    /// Set temperature (0.0-2.0).
    pub fn with_temperature(mut self, temp: f64) -> Self {
        self.temperature = temp.clamp(0.0, 2.0);
        self
    }

    /// Set maximum tokens.
    pub fn with_max_tokens(mut self, max: usize) -> Self {
        self.max_tokens = Some(max);
        self
    }

    /// Attach the exact input-token estimate produced by the context planner.
    pub fn with_context_input_tokens(mut self, input_tokens: usize) -> Self {
        self.context_input_tokens = Some(input_tokens);
        self
    }

    /// Set top_p (nucleus sampling).
    pub fn with_top_p(mut self, top_p: f64) -> Self {
        self.top_p = Some(top_p.clamp(0.0, 1.0));
        self
    }

    /// Set frequency penalty.
    pub fn with_frequency_penalty(mut self, penalty: f64) -> Self {
        self.frequency_penalty = Some(penalty);
        self
    }

    /// Set presence penalty.
    pub fn with_presence_penalty(mut self, penalty: f64) -> Self {
        self.presence_penalty = Some(penalty);
        self
    }

    /// Add a stop sequence.
    pub fn add_stop_sequence(mut self, stop: impl Into<String>) -> Self {
        self.stop_sequences.push(stop.into());
        self
    }

    /// Set a structured output schema.
    pub fn with_output_schema(mut self, schema: serde_json::Value) -> Self {
        self.output_schema = Some(schema);
        self
    }

    /// Set a hidden-reasoning token budget.
    pub fn with_thinking_token_budget(mut self, budget: u64) -> Self {
        self.thinking_token_budget = Some(budget);
        self
    }

    /// Set explicit thinking-mode behavior.
    pub fn with_enable_thinking(mut self, enabled: bool) -> Self {
        self.enable_thinking = Some(enabled);
        self
    }

    /// Add custom metadata.
    pub fn with_metadata_value(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    /// Set the exact backend the request names.
    pub fn with_backend(mut self, backend: impl Into<String>) -> Self {
        self.backend = Some(backend.into());
        self
    }

    /// Set APXM graph hints for vLLM scheduling.
    pub fn with_apxm_hints(mut self, hints: ApxmGraphHints) -> Self {
        self.apxm_hints = Some(hints);
        self
    }

    /// Set extra body fields (provider-specific).
    pub fn with_extra_body(mut self, body: serde_json::Value) -> Self {
        self.extra_body = Some(body);
        self
    }

    /// Set the exact model reference the request names.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set the AIS operation type this request was issued for.
    pub fn with_operation_type(mut self, operation: AISOperationType) -> Self {
        self.operation_type = Some(operation);
        self
    }

    /// Set tools available for the LLM to call.
    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Add a single tool to the request.
    pub fn add_tool(mut self, tool: ToolDefinition) -> Self {
        self.tools.get_or_insert_with(Vec::new).push(tool);
        self
    }

    /// Set how the LLM should use tools.
    pub fn with_tool_choice(mut self, choice: ToolChoice) -> Self {
        self.tool_choice = Some(choice);
        self
    }

    /// Set a trace ID for cross-process event correlation.
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    /// Check if this request has tools configured.
    pub fn has_tools(&self) -> bool {
        self.tools.as_ref().is_some_and(|t| !t.is_empty())
    }

    /// Validate request parameters.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.prompt.is_empty() && self.messages.is_empty() {
            return Err(anyhow::anyhow!(
                "Request must have a non-empty prompt or at least one message"
            ));
        }

        // Reject explicitly invalid task payload
        let trimmed_prompt = self.prompt.trim();
        if trimmed_prompt.eq_ignore_ascii_case("INVALID TASK") {
            return Err(anyhow::anyhow!(
                "Task payload explicitly marked as INVALID TASK"
            ));
        }

        // Check messages for invalid task marker
        for msg in &self.messages {
            let text = msg.text_content();
            if text.trim().eq_ignore_ascii_case("INVALID TASK") {
                return Err(anyhow::anyhow!(
                    "Task payload explicitly marked as INVALID TASK"
                ));
            }
        }

        if self.temperature < 0.0 || self.temperature > 2.0 {
            return Err(anyhow::anyhow!(
                "Temperature must be between 0.0 and 2.0, got {}",
                self.temperature
            ));
        }

        if let Some(top_p) = self.top_p
            && !(0.0..=1.0).contains(&top_p)
        {
            return Err(anyhow::anyhow!(
                "top_p must be between 0.0 and 1.0, got {}",
                top_p
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_messages_preserve_host_assembled_system_context() {
        let request = LLMRequest::from_messages(vec![Message::text(Role::User, "review this")])
            .with_system_prompt("platform policy\nselected skill instructions");

        let messages = request.resolved_messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(
            messages[0].text_content(),
            "platform policy\nselected skill instructions"
        );
        assert_eq!(messages[1].role, Role::User);
        assert_eq!(messages[1].text_content(), "review this");
    }
}

/// Reusable generation configuration template.
#[derive(Debug, Clone)]
pub struct GenerationConfig {
    /// Config name for reference
    pub name: String,
    /// Temperature setting
    pub temperature: f64,
    /// Max tokens setting
    pub max_tokens: Option<usize>,
    /// Top P setting
    pub top_p: Option<f64>,
}

impl GenerationConfig {
    /// Create a new config with given name.
    pub fn new(name: impl Into<String>, temperature: f64) -> Self {
        GenerationConfig {
            name: name.into(),
            temperature: temperature.clamp(0.0, 2.0),
            max_tokens: None,
            top_p: None,
        }
    }

    /// Predefined: balanced generation
    pub fn balanced() -> Self {
        GenerationConfig::new("balanced", 0.7)
    }

    /// Predefined: creative generation
    pub fn creative() -> Self {
        GenerationConfig::new("creative", 1.2)
    }

    /// Predefined: deterministic generation
    pub fn deterministic() -> Self {
        GenerationConfig::new("deterministic", 0.0)
    }

    /// Apply config to a request.
    pub fn apply(self, mut request: LLMRequest) -> LLMRequest {
        request.temperature = self.temperature;
        if let Some(max_tokens) = self.max_tokens {
            request.max_tokens = Some(max_tokens);
        }
        if let Some(top_p) = self.top_p {
            request.top_p = Some(top_p);
        }
        request
    }
}
