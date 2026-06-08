//! Phase 14.8.C — Outbound run-lifecycle webhook dispatcher.
//!
//! Fires fire-and-forget POSTs to a configured URL on every event the
//! `RunEventBus` records. The payload mirrors the SSE envelope used
//! by `/v1/runs/{id}/events/stream` so external consumers can run the
//! same parsing code for both transports.
//!
//! Design constraints:
//!   - Never block the runtime hot path. Posts run on `tokio::spawn`
//!     with a short timeout; failures are logged at `warn!`.
//!   - Only lifecycle-relevant kinds are sent so external consumers
//!     don't drown in tool token deltas. Filtering happens here, not
//!     at the producer, so producers stay oblivious.
//!   - The dispatcher is wrapped in `Option<Arc<…>>` on `AppState`;
//!     when unset (no env var, no per-skill webhook), the field is
//!     simply None and no work happens.

use std::sync::Arc;
use std::time::Duration;

use apxm_core::events::kind;
use apxm_core::events::{ApxmEvent, EventEmitter};
use apxm_driver::ServerWebhookConfig;
use reqwest::Client;
use tracing::warn;

/// Configured destination URL plus the shared HTTP client used to
/// POST events. Cloning is cheap — every field is already `Arc`-able.
#[derive(Clone)]
pub(crate) struct WebhookDispatcher {
    client: Client,
    url: Arc<String>,
}

impl WebhookDispatcher {
    /// Build a dispatcher from layered server config. Returns None when unset
    /// so AppState stays a None.
    pub(crate) fn from_config(config: &ServerWebhookConfig) -> Option<Arc<Self>> {
        config
            .url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .and_then(|url| {
                Self::with_timeout(
                    url.to_string(),
                    Duration::from_secs(config.timeout_secs.max(1)),
                )
                .ok()
                .map(Arc::new)
            })
    }

    #[cfg(test)]
    pub(crate) fn new(url: impl Into<String>) -> Result<Self, reqwest::Error> {
        Self::with_timeout(
            url,
            Duration::from_secs(ServerWebhookConfig::default().timeout_secs),
        )
    }

    fn with_timeout(url: impl Into<String>, timeout: Duration) -> Result<Self, reqwest::Error> {
        let client = Client::builder().timeout(timeout).build()?;
        Ok(Self {
            client,
            url: Arc::new(url.into()),
        })
    }

    /// Fire a single ApxmEvent to the configured URL. Spawns a tokio
    /// task — caller must be inside a tokio runtime.
    pub(crate) fn dispatch(&self, event: ApxmEvent) {
        if !is_lifecycle_relevant(&event) {
            return;
        }
        let client = self.client.clone();
        let url = self.url.clone();
        let body = match serde_json::to_value(&event) {
            Ok(value) => value,
            Err(error) => {
                warn!(%error, "failed to serialize webhook event");
                return;
            }
        };
        tokio::spawn(async move {
            match client.post(url.as_str()).json(&body).send().await {
                Ok(_response) => {}
                Err(error) => {
                    // Failures are non-fatal — external consumers
                    // come and go, and a missed event is recoverable
                    // by replaying via /v1/runs/.../events?since=.
                    warn!(%error, url = %url, "webhook POST failed");
                }
            }
        });
    }
}

/// Filter helper exposed for the run bus: only lifecycle-relevant
/// events get fanned out via webhook. Token-level streams are too
/// noisy for an external consumer to want by default.
pub(crate) fn is_lifecycle_relevant(event: &ApxmEvent) -> bool {
    matches!(
        event.kind(),
        kind::AGENT_SPAWNED
            | kind::COMMUNICATE_DISPATCHED
            | kind::GRAPH_EDGE
            | kind::SESSION_START
            | kind::SESSION_END
            | kind::ERROR
            | kind::EXECUTE_COMPLETE
            | kind::CHECKPOINT_SAVED
            | kind::CHECKPOINT_RESTORED
    ) || matches!(
        event.kind().name(),
        // Server-emitted skill lifecycle (`SKILL_EXECUTE_STARTED` etc.)
        // is defined out-of-crate; match on the wire name.
        "skill_execute_started"
            | "skill_execute_complete"
            | "run_started"
            | "run_completed"
            | "run_failed"
            | "subagent_spawned"
            | "approval_request"
    )
}

/// EventEmitter adapter so the `FanOutEmitter` can hand events to the
/// webhook dispatcher without callers caring about the network side.
pub(crate) struct WebhookEmitter {
    dispatcher: Arc<WebhookDispatcher>,
}

impl WebhookEmitter {
    pub(crate) fn new(dispatcher: Arc<WebhookDispatcher>) -> Self {
        Self { dispatcher }
    }
}

impl EventEmitter for WebhookEmitter {
    fn emit(&self, event: ApxmEvent) {
        self.dispatcher.dispatch(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_dispatcher_requires_nonempty_url() {
        let config = ServerWebhookConfig {
            url: Some(" ".to_string()),
            timeout_secs: 5,
        };

        assert!(WebhookDispatcher::from_config(&config).is_none());
    }

    #[test]
    fn webhook_dispatcher_builds_from_config() {
        let config = ServerWebhookConfig {
            url: Some("http://127.0.0.1:1/notify".to_string()),
            timeout_secs: 0,
        };

        assert!(WebhookDispatcher::from_config(&config).is_some());
    }
}
