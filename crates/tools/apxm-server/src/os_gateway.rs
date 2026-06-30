use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use apxm_core::types::host::{
    AgentChannelHandle, HostDispatchError, HostDispatchGateway, HostProxyRequest, HostProxyResult,
    HostToolCall, HostToolResult, SpawnOffer,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value as JsonValue;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tracing::debug;

/// HostDispatchGateway implementation that proxies calls through the apxm-os
/// HTTP relay API. Lives in apxm-server; calls os over HTTP/WS.
pub struct OsGatewayClient {
    os_base_url: String,
    client: reqwest::Client,
    pending_channels: Arc<Mutex<HashMap<String, (mpsc::Sender<JsonValue>, mpsc::Receiver<JsonValue>)>>>,
}

impl OsGatewayClient {
    pub fn new(os_base_url: impl Into<String>) -> Self {
        Self {
            os_base_url: os_base_url.into(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("reqwest client"),
            pending_channels: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl HostDispatchGateway for OsGatewayClient {
    async fn call_tool(
        &self,
        host_id: &str,
        call: HostToolCall,
    ) -> Result<HostToolResult, HostDispatchError> {
        let url = format!("{}/v1/relay/tools/call", self.os_base_url);
        let body = serde_json::json!({
            "host_id": host_id,
            "call_id": &call.call_id,
            "capability_id": &call.capability_id,
            "host_op": &call.host_op,
            "tool_binding": &call.tool_binding,
            "args": &call.args,
            "timeout_ms": call.timeout_ms,
        });
        let resp = self.client.post(&url)
            .json(&body)
            .timeout(Duration::from_millis(call.timeout_ms + 5000))
            .send()
            .await
            .map_err(|e| HostDispatchError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            let msg = resp.text().await.unwrap_or_default();
            return Err(HostDispatchError::HostError { message: msg });
        }
        resp.json::<HostToolResult>()
            .await
            .map_err(|e| HostDispatchError::Transport(e.to_string()))
    }

    async fn request_relay_egress(
        &self,
        _host_id: &str,
        _request: HostProxyRequest,
    ) -> Result<HostProxyResult, HostDispatchError> {
        Err(HostDispatchError::Transport("relay egress not implemented".to_string()))
    }

    async fn open_agent_channel(
        &self,
        host_id: &str,
        spawn_offer: SpawnOffer,
    ) -> Result<AgentChannelHandle, HostDispatchError> {
        let url = format!("{}/v1/relay/agent/spawn", self.os_base_url);
        let resp = self.client.post(&url)
            .json(&serde_json::json!({
                "host_id": host_id,
                "channel_id": &spawn_offer.channel_id,
                "lease_id": &spawn_offer.lease_id,
                "profile": &spawn_offer.profile,
                "mode": &spawn_offer.mode,
                "model": &spawn_offer.model,
            }))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| HostDispatchError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            let msg = resp.text().await.unwrap_or_default();
            return Err(HostDispatchError::HostError { message: msg });
        }
        let body: JsonValue = resp.json().await
            .map_err(|e| HostDispatchError::Transport(e.to_string()))?;
        let channel_id = body["channel_id"].as_str()
            .unwrap_or(&spawn_offer.channel_id)
            .to_string();

        // Connect WebSocket to the relay channel endpoint
        let ws_url = format!("{}/v1/relay/channels/{channel_id}",
            self.os_base_url.replace("http://", "ws://").replace("https://", "wss://"));

        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .map_err(|e| HostDispatchError::Transport(format!("ws connect: {e}")))?;

        let (mut ws_sink, mut ws_src) = ws_stream.split();
        let (to_host_tx, mut to_host_rx) = mpsc::channel::<JsonValue>(64);
        let (from_host_tx, from_host_rx) = mpsc::channel::<JsonValue>(64);

        // Bridge WS ↔ mpsc channels
        let channel_id2 = channel_id.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = to_host_rx.recv() => {
                        let Some(v) = msg else { break; };
                        let text = serde_json::to_string(&v).unwrap_or_default();
                        if ws_sink.send(Message::Text(text.into())).await.is_err() { break; }
                    }
                    frame = ws_src.next() => {
                        let Some(Ok(Message::Text(t))) = frame else { break; };
                        let v: JsonValue = serde_json::from_str(&t).unwrap_or(JsonValue::Null);
                        if from_host_tx.send(v).await.is_err() { break; }
                    }
                }
            }
            debug!(channel_id = %channel_id2, "relay channel WS bridge closed");
        });

        self.pending_channels.lock().await.insert(channel_id.clone(), (to_host_tx, from_host_rx));

        Ok(AgentChannelHandle { channel_id, host_id: host_id.to_string() })
    }

    async fn send_agent_frame(
        &self,
        channel_id: &str,
        frame: Vec<u8>,
    ) -> Result<(), HostDispatchError> {
        let v: JsonValue = serde_json::from_slice(&frame)
            .map_err(|e| HostDispatchError::Transport(e.to_string()))?;
        let guard = self.pending_channels.lock().await;
        if let Some((tx, _)) = guard.get(channel_id) {
            tx.send(v).await.map_err(|_| HostDispatchError::Transport("channel closed".to_string()))
        } else {
            Err(HostDispatchError::NotAttached { host_id: channel_id.to_string() })
        }
    }

    async fn close_agent_channel(
        &self,
        channel_id: &str,
        _reason: &str,
    ) -> Result<(), HostDispatchError> {
        self.pending_channels.lock().await.remove(channel_id);
        Ok(())
    }

    async fn take_relay_channel(
        &self,
        channel_id: &str,
    ) -> Option<(mpsc::Sender<JsonValue>, mpsc::Receiver<JsonValue>)> {
        self.pending_channels.lock().await.remove(channel_id)
    }
}
