//! HostDispatchGateway — the agents-facing interface for host I/O.
//!
//! Defined in `apxm-core` (`apxm_core::types::host`); this module is the
//! agents-layer re-export surface so callers within `apxm-runtime` import
//! from one place.
//!
//! Agents NEVER hold live Link sockets or access host attach state directly.
//! All host I/O crosses this interface.
//!
//! Named HostDispatchGateway because host I/O crosses a single agents-facing gateway.

pub use apxm_core::types::host::{
    AgentChannelHandle, HostDispatchError, HostDispatchGateway, HostEffectCommit,
    HostEffectOutcome, HostEffectPrepare, HostPromptApproval, HostProxyRequest, HostProxyResult,
    HostToolCall, HostToolError, HostToolResult, SpawnOffer,
};

/// A no-op gateway used in tests or when no host dispatch is configured.
///
/// Every method returns a `HostDispatchError::Transport` indicating that no
/// real gateway is wired up. Use in unit tests that do not exercise host I/O.
pub struct NoOpHostDispatchGateway;

#[async_trait::async_trait]
impl HostDispatchGateway for NoOpHostDispatchGateway {
    async fn call_tool(
        &self,
        _host_id: &str,
        _call: HostToolCall,
    ) -> Result<HostToolResult, HostDispatchError> {
        Err(HostDispatchError::Transport(
            "no host dispatch gateway configured".into(),
        ))
    }

    async fn request_relay_egress(
        &self,
        _host_id: &str,
        _request: HostProxyRequest,
    ) -> Result<HostProxyResult, HostDispatchError> {
        Err(HostDispatchError::Transport(
            "no host dispatch gateway configured".into(),
        ))
    }

    async fn request_permission(
        &self,
        _host_id: &str,
        _prompt: serde_json::Value,
        _timeout_ms: u64,
    ) -> Result<HostPromptApproval, HostDispatchError> {
        Err(HostDispatchError::Transport(
            "no host dispatch gateway configured".into(),
        ))
    }

    async fn open_agent_channel(
        &self,
        _host_id: &str,
        _spawn_offer: SpawnOffer,
    ) -> Result<AgentChannelHandle, HostDispatchError> {
        Err(HostDispatchError::Transport(
            "no host dispatch gateway configured".into(),
        ))
    }

    async fn send_agent_frame(
        &self,
        _channel_id: &str,
        _frame: Vec<u8>,
    ) -> Result<(), HostDispatchError> {
        Err(HostDispatchError::Transport(
            "no host dispatch gateway configured".into(),
        ))
    }

    async fn close_agent_channel(
        &self,
        _channel_id: &str,
        _reason: &str,
    ) -> Result<(), HostDispatchError> {
        Err(HostDispatchError::Transport(
            "no host dispatch gateway configured".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn noop_gateway_call_tool_returns_transport_error() {
        let gw = NoOpHostDispatchGateway;
        let call = HostToolCall {
            call_id: "c1".into(),
            capability_id: "cap1".into(),
            host_op: "op".into(),
            capability_binding: "binding".into(),
            args: serde_json::Value::Null,
            args_digest: "".into(),
            grant_ref: None,
            timeout_ms: 5000,
            idempotency_key: None,
            subject: None,
        };
        let err = gw.call_tool("h1", call).await;
        assert!(matches!(err, Err(HostDispatchError::Transport(_))));
    }

    #[tokio::test]
    async fn noop_gateway_send_frame_returns_transport_error() {
        let gw = NoOpHostDispatchGateway;
        let err = gw.send_agent_frame("ch1", vec![]).await;
        assert!(matches!(err, Err(HostDispatchError::Transport(_))));
    }
}
