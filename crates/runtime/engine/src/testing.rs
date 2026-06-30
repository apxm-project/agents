//! Test support for runtime-owned mocks.
//!
//! These mocks live outside operation handlers so tests exercise runtime
//! behavior without depending on real ACP agents, external binaries, or live
//! backend services.

use std::sync::{Arc, Mutex};

use apxm_core::error::RuntimeError;

use crate::process::AgentProcess;
use crate::process_table::{AgentPromptResponse, AgentPrompter};

pub const MOCK_AGENT_NAME: &str = "mock-agent";
pub const MOCK_AGENT_NAME_ALT: &str = "mock-agent-alt";
pub const MOCK_AGENT_PROFILE: &str = "mock-profile";
pub const MOCK_AGENT_PROFILE_ALT: &str = "mock-profile-alt";
pub const MOCK_BACKEND_KIND: &str = "mock-backend-kind";
pub const MOCK_BACKEND_NAME: &str = "mock-backend";
pub const MOCK_BACKEND_NAME_ALT: &str = "mock-backend-alt";
pub const MOCK_MODEL_NAME: &str = "mock-model";
pub const MOCK_MODEL_NAME_ALT: &str = "mock-model-alt";
pub const MOCK_PROVIDER_NAME: &str = "mock-provider";
pub const MOCK_PROVIDER_NAME_ALT: &str = "mock-provider-alt";
pub const MOCK_SESSION_ID: &str = "mock-session";
pub const MOCK_STOP_REASON: &str = "mock-stop";

/// Agent prompter that records every prompt and echoes the prompt text.
pub struct RecordingAgentPrompter {
    prompts: Arc<Mutex<Vec<String>>>,
}

impl RecordingAgentPrompter {
    pub fn new(prompts: Arc<Mutex<Vec<String>>>) -> Self {
        Self { prompts }
    }
}

#[async_trait::async_trait]
impl AgentPrompter for RecordingAgentPrompter {
    async fn prompt(
        &self,
        _process: &AgentProcess,
        message: &str,
    ) -> Result<AgentPromptResponse, RuntimeError> {
        self.prompts
            .lock()
            .expect("prompt lock")
            .push(message.to_string());
        Ok(AgentPromptResponse::text(message.to_string()))
    }
}

/// Agent prompter that returns a fixed response with deterministic usage.
pub struct MockUsageAgentPrompter {
    response_prefix: String,
    session_id: Option<String>,
    turn: Option<u64>,
    model: Option<String>,
    stop_reason: Option<String>,
    input_tokens: Option<usize>,
    output_tokens: Option<usize>,
}

impl MockUsageAgentPrompter {
    pub fn new(response_prefix: impl Into<String>) -> Self {
        Self {
            response_prefix: response_prefix.into(),
            session_id: None,
            turn: None,
            model: None,
            stop_reason: None,
            input_tokens: None,
            output_tokens: None,
        }
    }

    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_turn(mut self, turn: u64) -> Self {
        self.turn = Some(turn);
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_stop_reason(mut self, stop_reason: impl Into<String>) -> Self {
        self.stop_reason = Some(stop_reason.into());
        self
    }

    pub fn with_token_usage(
        mut self,
        input_tokens: Option<usize>,
        output_tokens: Option<usize>,
    ) -> Self {
        self.input_tokens = input_tokens;
        self.output_tokens = output_tokens;
        self
    }
}

#[async_trait::async_trait]
impl AgentPrompter for MockUsageAgentPrompter {
    async fn prompt(
        &self,
        _process: &AgentProcess,
        message: &str,
    ) -> Result<AgentPromptResponse, RuntimeError> {
        let mut response =
            AgentPromptResponse::text(format!("{}{}", self.response_prefix, message))
                .with_token_usage(self.input_tokens, self.output_tokens);
        if let Some(session_id) = &self.session_id {
            response = response.with_session_id(session_id.clone());
        }
        if let Some(turn) = self.turn {
            response = response.with_turn(turn);
        }
        if let Some(model) = &self.model {
            response = response.with_model(model.clone());
        }
        if let Some(stop_reason) = &self.stop_reason {
            response = response.with_stop_reason(stop_reason.clone());
        }
        Ok(response)
    }
}
