//! LLM request types and builders.

use apxm_core::types::AISOperationType;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const DEFAULT_TEMPERATURE: f64 = 0.7;

/// Definition of a tool that can be called by the LLM.
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
    /// The main prompt/input text (backward-compatible single-string field).
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
    /// Nucleus sampling - keep top p probability mass
    pub top_p: Option<f64>,
    /// Frequency penalty - penalize repeated tokens
    pub frequency_penalty: Option<f64>,
    /// Presence penalty - encourage new topics
    pub presence_penalty: Option<f64>,
    /// Stop sequences where generation stops
    pub stop_sequences: Vec<String>,
    /// Custom metadata passed through to provider
    pub metadata: HashMap<String, serde_json::Value>,
    /// Explicitly requested backend (for routing)
    pub backend: Option<String>,
    /// Explicitly requested model (for routing)
    pub model: Option<String>,
    /// AIS operation type used for intelligent routing.
    pub operation_type: Option<AISOperationType>,
    /// Tools available for the LLM to call
    pub tools: Option<Vec<ToolDefinition>>,
    /// How the LLM should use tools
    pub tool_choice: Option<ToolChoice>,
    /// Trace ID for cross-process event correlation.
    pub trace_id: Option<String>,
}

impl LLMRequest {
    /// Create a new request with just a prompt.
    ///
    /// The prompt is stored in the `prompt` field for backward compatibility.
    /// Backends auto-wrap it as a single User message when `messages` is empty.
    pub fn new(prompt: impl Into<String>) -> Self {
        LLMRequest {
            prompt: prompt.into(),
            messages: Vec::new(),
            system_prompt: None,
            temperature: DEFAULT_TEMPERATURE,
            max_tokens: None,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop_sequences: Vec::new(),
            metadata: HashMap::new(),
            backend: None,
            model: None,
            operation_type: None,
            tools: None,
            tool_choice: None,
            trace_id: None,
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
    /// If `messages` is non-empty, returns a clone of it.
    /// Otherwise, synthesizes messages from `prompt` (and optionally `system_prompt`).
    pub fn resolved_messages(&self) -> Vec<Message> {
        if !self.messages.is_empty() {
            return self.messages.clone();
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

    /// Add custom metadata.
    pub fn with_metadata_value(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    /// Set explicit backend for routing.
    pub fn with_backend(mut self, backend: impl Into<String>) -> Self {
        self.backend = Some(backend.into());
        self
    }

    /// Set explicit model for routing.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set operation type for routing.
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

/// Builder pattern helper for complex request construction.
pub struct RequestBuilder {
    request: LLMRequest,
}

impl RequestBuilder {
    /// Create a new builder with the given prompt.
    pub fn new(prompt: impl Into<String>) -> Self {
        RequestBuilder {
            request: LLMRequest::new(prompt),
        }
    }

    /// Add system prompt.
    pub fn system(mut self, system: impl Into<String>) -> Self {
        self.request = self.request.with_system_prompt(system);
        self
    }

    /// Set temperature.
    pub fn temperature(mut self, temp: f64) -> Self {
        self.request = self.request.with_temperature(temp);
        self
    }

    /// Set max tokens.
    pub fn max_tokens(mut self, max: usize) -> Self {
        self.request = self.request.with_max_tokens(max);
        self
    }

    /// Set top_p.
    pub fn top_p(mut self, top_p: f64) -> Self {
        self.request = self.request.with_top_p(top_p);
        self
    }

    /// Add stop sequence.
    pub fn stop(mut self, stop: impl Into<String>) -> Self {
        self.request = self.request.add_stop_sequence(stop);
        self
    }

    /// Set tools available for the LLM.
    pub fn tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.request = self.request.with_tools(tools);
        self
    }

    /// Set how the LLM should use tools.
    pub fn tool_choice(mut self, choice: ToolChoice) -> Self {
        self.request = self.request.with_tool_choice(choice);
        self
    }

    /// Set a trace ID for cross-process event correlation.
    pub fn trace_id(mut self, id: impl Into<String>) -> Self {
        self.request = self.request.with_trace_id(id);
        self
    }

    /// Set structured messages.
    pub fn messages(mut self, messages: Vec<Message>) -> Self {
        self.request = self.request.with_messages(messages);
        self
    }

    /// Build the final request.
    pub fn build(self) -> anyhow::Result<LLMRequest> {
        self.request.validate()?;
        Ok(self.request)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_creation() {
        let req = LLMRequest::new("Hello");
        assert_eq!(req.prompt, "Hello");
        assert_eq!(req.temperature, 0.7);
        assert!(req.messages.is_empty());
    }

    #[test]
    fn test_request_builder() -> Result<(), Box<dyn std::error::Error>> {
        let req = RequestBuilder::new("Test")
            .temperature(0.9)
            .max_tokens(500)
            .build()?;

        assert_eq!(req.temperature, 0.9);
        assert_eq!(req.max_tokens, Some(500));
        Ok(())
    }

    #[test]
    fn test_request_validation() {
        // Empty prompt with no messages should fail
        let req = LLMRequest::new("");
        assert!(req.validate().is_err());

        // Empty prompt but with messages should pass
        let req = LLMRequest::from_messages(vec![Message::text(Role::User, "Hello")]);
        assert!(req.validate().is_ok());

        let req = LLMRequest {
            temperature: 3.0,
            ..LLMRequest::new("test")
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_generation_configs() {
        let balanced = GenerationConfig::balanced();
        assert_eq!(balanced.temperature, 0.7);

        let creative = GenerationConfig::creative();
        assert_eq!(creative.temperature, 1.2);

        let deterministic = GenerationConfig::deterministic();
        assert_eq!(deterministic.temperature, 0.0);
    }

    #[test]
    fn test_tool_definition() {
        let tool = ToolDefinition::new(
            "bash",
            "Execute shell commands",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                },
                "required": ["command"]
            }),
        );
        assert_eq!(tool.name, "bash");
        assert_eq!(tool.description, "Execute shell commands");
    }

    #[test]
    fn test_request_with_tools() {
        let tools = vec![
            ToolDefinition::new("bash", "Execute shell commands", serde_json::json!({})),
            ToolDefinition::new("read", "Read file contents", serde_json::json!({})),
        ];

        let req = LLMRequest::new("Hello").with_tools(tools);
        assert!(req.has_tools());
        assert_eq!(req.tools.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_request_add_tool() {
        let req = LLMRequest::new("Hello")
            .add_tool(ToolDefinition::new(
                "bash",
                "Execute shell",
                serde_json::json!({}),
            ))
            .add_tool(ToolDefinition::new(
                "read",
                "Read file",
                serde_json::json!({}),
            ));

        assert!(req.has_tools());
        assert_eq!(req.tools.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_tool_choice_variants() {
        let auto = ToolChoice::Auto;
        let none = ToolChoice::None;
        let required = ToolChoice::Required;
        let specific = ToolChoice::Specific("bash".to_string());

        assert!(matches!(auto, ToolChoice::Auto));
        assert!(matches!(none, ToolChoice::None));
        assert!(matches!(required, ToolChoice::Required));
        assert!(matches!(specific, ToolChoice::Specific(_)));
    }

    #[test]
    fn test_structured_messages() {
        let messages = vec![
            Message::text(Role::System, "You are helpful"),
            Message::text(Role::User, "Hello"),
        ];

        let req = LLMRequest::from_messages(messages);
        assert!(req.has_messages());
        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.messages[0].role, Role::System);
        assert_eq!(req.messages[1].role, Role::User);
        assert_eq!(req.prompt, "Hello");
    }

    #[test]
    fn test_resolved_messages_from_prompt() {
        let req = LLMRequest::new("Hello").with_system_prompt("Be helpful");
        let msgs = req.resolved_messages();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, Role::System);
        assert_eq!(msgs[0].text_content(), "Be helpful");
        assert_eq!(msgs[1].role, Role::User);
        assert_eq!(msgs[1].text_content(), "Hello");
    }

    #[test]
    fn test_resolved_messages_from_messages() {
        let messages = vec![
            Message::text(Role::User, "First"),
            Message::text(Role::Assistant, "Response"),
            Message::text(Role::User, "Follow-up"),
        ];
        let req = LLMRequest::from_messages(messages);
        let msgs = req.resolved_messages();
        assert_eq!(msgs.len(), 3);
    }

    #[test]
    fn test_message_text_content() {
        let msg = Message::text(Role::User, "Hello world");
        assert_eq!(msg.text_content(), "Hello world");
    }

    #[test]
    fn test_tool_result_message() {
        let msg = Message::tool_result("call_123", "output text");
        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.tool_call_id, Some("call_123".to_string()));
        assert_eq!(msg.text_content(), "output text");
    }

    #[test]
    fn test_message_serialization() {
        let msg = Message::text(Role::User, "Hello");
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["role"], "user");

        let deserialized: Message = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.role, Role::User);
        assert_eq!(deserialized.text_content(), "Hello");
    }

    #[test]
    fn test_trace_id_default_none() {
        let req = LLMRequest::new("Hello");
        assert!(req.trace_id.is_none());
    }

    #[test]
    fn test_with_trace_id() {
        let req = LLMRequest::new("Hello").with_trace_id("trace-abc-123");
        assert_eq!(req.trace_id, Some("trace-abc-123".to_string()));
    }

    #[test]
    fn test_builder_trace_id() {
        let req = RequestBuilder::new("Hello")
            .trace_id("trace-xyz")
            .build()
            .unwrap();
        assert_eq!(req.trace_id, Some("trace-xyz".to_string()));
    }
}
