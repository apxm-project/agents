//! MockHostDispatchGateway — records calls and returns configured responses.
//!
//! Used by conformance tests that need to inspect which gateway methods were
//! called and what arguments were passed, without wiring a real Link socket.

use std::collections::HashMap;
use std::sync::Arc;

use apxm_core::types::host::{
    AgentChannelHandle, HostDispatchError, HostDispatchGateway, HostProxyRequest, HostProxyResult,
    HostToolCall, HostToolResult, SpawnOffer,
};
use tokio::sync::{Mutex, mpsc};

/// Recorded call from `call_tool`.
#[derive(Debug, Clone)]
pub struct RecordedToolCall {
    pub host_id: String,
    pub call: HostToolCall,
}

/// Recorded call from `open_agent_channel`.
#[derive(Debug, Clone)]
pub struct RecordedChannelOpen {
    pub host_id: String,
    pub spawn_offer: SpawnOffer,
}

/// Recorded call from `send_agent_frame`.
#[derive(Debug, Clone)]
pub struct RecordedFrame {
    pub channel_id: String,
    pub frame: Vec<u8>,
}

/// Recorded call from `close_agent_channel`.
#[derive(Debug, Clone)]
pub struct RecordedChannelClose {
    pub channel_id: String,
    pub reason: String,
}

/// Configured response for `call_tool` calls.
#[allow(dead_code)]
pub enum CallToolResponse {
    Success(HostToolResult),
    Error(HostDispatchError),
}

/// A mock gateway that records all calls and returns configured responses.
///
/// Use `MockHostDispatchGateway::builder()` to configure responses, then
/// call `.build()` to get an `Arc`-wrapped instance for injection.
pub struct MockHostDispatchGateway {
    /// Recorded `call_tool` invocations (appended in order).
    pub tool_calls: Mutex<Vec<RecordedToolCall>>,
    /// Recorded `open_agent_channel` invocations.
    pub channel_opens: Mutex<Vec<RecordedChannelOpen>>,
    /// Recorded `send_agent_frame` invocations.
    pub frames_sent: Mutex<Vec<RecordedFrame>>,
    /// Recorded `close_agent_channel` invocations.
    pub channel_closes: Mutex<Vec<RecordedChannelClose>>,

    /// Canned response for every `call_tool` call (default: success with `ok: true`).
    pub call_tool_result: HostToolResult,
    /// Canned `channel_id` returned by `open_agent_channel`.
    pub channel_id: String,

    /// Pre-seeded relay channels keyed by channel_id.
    /// `take_relay_channel` drains these.
    relay_channels: Mutex<
        HashMap<String, (mpsc::Sender<serde_json::Value>, mpsc::Receiver<serde_json::Value>)>,
    >,
}

impl Default for MockHostDispatchGateway {
    fn default() -> Self {
        Self {
            tool_calls: Mutex::new(Vec::new()),
            channel_opens: Mutex::new(Vec::new()),
            frames_sent: Mutex::new(Vec::new()),
            channel_closes: Mutex::new(Vec::new()),
            call_tool_result: HostToolResult {
                ok: true,
                value: Some(serde_json::json!({ "result": "mock-ok" })),
                error: None,
                result_ref: None,
            },
            channel_id: "mock-channel-001".to_string(),
            relay_channels: Mutex::new(HashMap::new()),
        }
    }
}

impl MockHostDispatchGateway {
    /// Construct a default mock (all calls succeed with canned values).
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Seed a relay channel pair so `take_relay_channel` returns it.
    pub async fn seed_relay_channel(
        self: &Arc<Self>,
        channel_id: &str,
    ) -> (mpsc::Sender<serde_json::Value>, mpsc::Receiver<serde_json::Value>) {
        let (tx_to_host, rx_from_apxm) = mpsc::channel::<serde_json::Value>(16);
        let (tx_to_apxm, rx_from_host) = mpsc::channel::<serde_json::Value>(16);
        self.relay_channels
            .lock()
            .await
            .insert(channel_id.to_string(), (tx_to_host, rx_from_host));
        (tx_to_apxm, rx_from_apxm)
    }

    /// Return the number of `call_tool` invocations recorded.
    pub async fn tool_call_count(&self) -> usize {
        self.tool_calls.lock().await.len()
    }

    /// Return the number of `open_agent_channel` invocations recorded.
    pub async fn channel_open_count(&self) -> usize {
        self.channel_opens.lock().await.len()
    }

    /// Return all recorded spawn offers (cloned).
    pub async fn spawn_offers(&self) -> Vec<SpawnOffer> {
        self.channel_opens
            .lock()
            .await
            .iter()
            .map(|r| r.spawn_offer.clone())
            .collect()
    }
}

#[async_trait::async_trait]
impl HostDispatchGateway for MockHostDispatchGateway {
    async fn call_tool(
        &self,
        host_id: &str,
        call: HostToolCall,
    ) -> Result<HostToolResult, HostDispatchError> {
        self.tool_calls.lock().await.push(RecordedToolCall {
            host_id: host_id.to_string(),
            call,
        });
        Ok(self.call_tool_result.clone())
    }

    async fn request_relay_egress(
        &self,
        _host_id: &str,
        _request: HostProxyRequest,
    ) -> Result<HostProxyResult, HostDispatchError> {
        Ok(HostProxyResult {
            ok: true,
            status: Some(200),
            body_ref: None,
            error: None,
        })
    }

    async fn open_agent_channel(
        &self,
        host_id: &str,
        spawn_offer: SpawnOffer,
    ) -> Result<AgentChannelHandle, HostDispatchError> {
        self.channel_opens.lock().await.push(RecordedChannelOpen {
            host_id: host_id.to_string(),
            spawn_offer,
        });
        Ok(AgentChannelHandle {
            channel_id: self.channel_id.clone(),
            host_id: host_id.to_string(),
        })
    }

    async fn send_agent_frame(
        &self,
        channel_id: &str,
        frame: Vec<u8>,
    ) -> Result<(), HostDispatchError> {
        self.frames_sent.lock().await.push(RecordedFrame {
            channel_id: channel_id.to_string(),
            frame,
        });
        Ok(())
    }

    async fn close_agent_channel(
        &self,
        channel_id: &str,
        reason: &str,
    ) -> Result<(), HostDispatchError> {
        self.channel_closes.lock().await.push(RecordedChannelClose {
            channel_id: channel_id.to_string(),
            reason: reason.to_string(),
        });
        Ok(())
    }

    async fn take_relay_channel(
        &self,
        channel_id: &str,
    ) -> Option<(mpsc::Sender<serde_json::Value>, mpsc::Receiver<serde_json::Value>)> {
        self.relay_channels.lock().await.remove(channel_id)
    }
}
