use crate::constants::{fields, methods};
use crate::session::AcpSession;
use crate::AcpError;

/// Session control operations (mode, model, cancel).
///
/// These send JSON-RPC requests to the agent to configure session behavior
/// before or during a prompt.
pub struct SessionControls;

impl SessionControls {
    /// Set the agent's operating mode (e.g., "architect", "code").
    pub async fn set_mode(session: &mut AcpSession, mode_id: &str) -> Result<(), AcpError> {
        let params = serde_json::json!({
            fields::SESSION_ID: session.agent_session_id(),
            fields::MODE_ID: mode_id,
        });
        session
            .send_request_no_reverse(methods::SESSION_SET_MODE, Some(params))
            .await?;
        Ok(())
    }

    /// Set the agent's model (e.g., "claude-sonnet-4").
    pub async fn set_model(session: &mut AcpSession, model_id: &str) -> Result<(), AcpError> {
        let params = serde_json::json!({
            fields::SESSION_ID: session.agent_session_id(),
            "model": model_id,
        });
        session
            .send_request_no_reverse(methods::UNSTABLE_SET_SESSION_MODEL, Some(params))
            .await?;
        Ok(())
    }

    /// Set a configuration option on the agent.
    pub async fn set_config_option(
        session: &mut AcpSession,
        config_id: &str,
        value: &str,
    ) -> Result<serde_json::Value, AcpError> {
        let params = serde_json::json!({
            fields::SESSION_ID: session.agent_session_id(),
            fields::CONFIG_ID: config_id,
            "value": value,
        });
        session
            .send_request_no_reverse(methods::SESSION_SET_CONFIG_OPTION, Some(params))
            .await
    }

    /// Cancel the current operation on the session.
    pub async fn cancel(session: &mut AcpSession) -> Result<(), AcpError> {
        let params = serde_json::json!({
            fields::SESSION_ID: session.agent_session_id(),
        });
        session
            .send_request_no_reverse(methods::SESSION_CANCEL, Some(params))
            .await?;
        Ok(())
    }
}
