//! Phase 14.8.F — shared monospace render contract for `apxm watch` and
//! `apxm rollout replay`.
//!
//! The chat panel and the TUI both consume the same per-specialist event
//! shape (`agent_spawned`, `tool_call_begin`, `operation_start`, …) — this
//! module is the *single* place we project an event stream into the TUI's
//! line shape so the two presentations stay in sync.

use std::collections::BTreeMap;

use apxm_core::events::ApxmEvent;
use apxm_core::events::payload::{
    AgentSpawnedPayload, NodeMetricsPayload, OperationEndPayload, OperationStartPayload,
    ToolEndPayload, ToolStartPayload,
};

/// Per-agent rolling state derived from the event stream. Public so the
/// `replay` and `watch` paths can render identically off the same struct.
#[derive(Debug, Clone, Default)]
pub struct AgentRow {
    pub agent_code: String,
    pub status: AgentStatus,
    pub tool_calls: u64,
    pub tokens: u64,
    pub last_tool: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentStatus {
    #[default]
    Pending,
    Spawning,
    Running,
    Done,
    Failed,
}

impl AgentStatus {
    /// Status glyphs match the chat panel mapping: pending → ○, spawning →
    /// ◐, running → ◑, done → ●, failed → ✕.
    pub fn glyph(self) -> &'static str {
        match self {
            AgentStatus::Pending => "○",
            AgentStatus::Spawning => "◐",
            AgentStatus::Running => "◑",
            AgentStatus::Done => "●",
            AgentStatus::Failed => "✕",
        }
    }
}

/// Aggregate snapshot for the whole run.
#[derive(Debug, Clone, Default)]
pub struct RunSnapshot {
    pub thread_id: String,
    pub root_agent: Option<String>,
    pub agents: BTreeMap<String, AgentRow>,
    /// node_id → agent_code reverse map, filled by AgentSpawned and used
    /// by OperationEnd/NodeMetrics to project per-node events onto the
    /// right row.
    node_to_agent: std::collections::HashMap<u64, String>,
}

impl RunSnapshot {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            ..Self::default()
        }
    }

    /// Fold one event into the snapshot. Unknown event payloads are
    /// ignored — the renderer only knows the canonical agent/tool/op
    /// shapes the chat panel already consumes.
    pub fn apply(&mut self, event: &ApxmEvent) {
        if let Some(payload) = event.payload.downcast_ref::<AgentSpawnedPayload>() {
            let entry = self
                .agents
                .entry(payload.agent_code.clone())
                .or_insert_with(|| AgentRow {
                    agent_code: payload.agent_code.clone(),
                    ..AgentRow::default()
                });
            // First spawn wins as the root_agent unless one is already
            // pinned by a downstream event.
            if self.root_agent.is_none() {
                self.root_agent = Some(payload.agent_code.clone());
            }
            entry.status = AgentStatus::Spawning;
            self.node_to_agent
                .insert(payload.node_id, payload.agent_code.clone());
        }
        if let Some(payload) = event.payload.downcast_ref::<OperationStartPayload>() {
            // The runtime stamps agent_code on the context for SPAWN_AGENT
            // and the COMMUNICATE op; flip the matching row to Running.
            let agent_code = payload
                .context
                .as_ref()
                .and_then(|c| c.get("agent_code"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| self.node_to_agent.get(&payload.node_id).cloned());
            if let Some(code) = agent_code {
                let entry = self.agents.entry(code.clone()).or_insert_with(|| AgentRow {
                    agent_code: code.clone(),
                    ..AgentRow::default()
                });
                entry.status = AgentStatus::Running;
            }
        }
        if let Some(payload) = event.payload.downcast_ref::<OperationEndPayload>()
            && let Some(code) = self.node_to_agent.get(&payload.node_id).cloned()
            && let Some(entry) = self.agents.get_mut(&code)
        {
            entry.status = if payload.success {
                AgentStatus::Done
            } else {
                AgentStatus::Failed
            };
        }
        if let Some(payload) = event.payload.downcast_ref::<ToolStartPayload>() {
            // Tool events don't always carry the calling agent's code; fall
            // back to the most recently spawned agent so the counters still
            // attach to something visible.
            if let Some(code) = tool_agent_code(payload, &self.agents) {
                let entry = self.agents.entry(code.clone()).or_insert_with(|| AgentRow {
                    agent_code: code.clone(),
                    ..AgentRow::default()
                });
                entry.tool_calls += 1;
                entry.last_tool = Some(payload.name.clone());
                if entry.status == AgentStatus::Pending {
                    entry.status = AgentStatus::Running;
                }
            }
        }
        if let Some(payload) = event.payload.downcast_ref::<ToolEndPayload>() {
            let _ = payload; // Tool end is informational; status flips via OperationEnd.
        }
        // Token accounting comes off NodeMetrics events; project node_id
        // back to the owning agent via the spawn-time reverse map.
        if let Some(payload) = event.payload.downcast_ref::<NodeMetricsPayload>()
            && let Some(code) = self.node_to_agent.get(&payload.node_id).cloned()
            && let Some(entry) = self.agents.get_mut(&code)
        {
            entry.tokens += payload.metrics.processes.totals.input_tokens as u64
                + payload.metrics.processes.totals.output_tokens as u64;
        }
    }

    /// Returns true while at least one agent is non-terminal. Drives the
    /// "Synthesizing…" line on the watch view.
    pub fn any_running(&self) -> bool {
        self.agents
            .values()
            .any(|a| matches!(a.status, AgentStatus::Spawning | AgentStatus::Running))
    }
}

fn tool_agent_code(
    payload: &ToolStartPayload,
    agents: &BTreeMap<String, AgentRow>,
) -> Option<String> {
    // The runtime doesn't stamp agent_code on ToolStart directly; the args
    // can carry it as a debugging hint when the dispatcher fans out via
    // SPAWN_AGENT. Falling back to "the last agent we've seen running"
    // keeps the per-agent counter sane even without that hint.
    if let Some(code) = payload
        .args
        .get("_agent_code")
        .and_then(serde_json::Value::as_str)
    {
        return Some(code.to_string());
    }
    agents
        .values()
        .rev()
        .find(|a| matches!(a.status, AgentStatus::Running | AgentStatus::Spawning))
        .map(|a| a.agent_code.clone())
}

/// Render `snapshot` to a multi-line monospace string.
///
/// Layout matches the design in the plan:
///
/// ```text
/// ⏺ Cleo · 3 specialists running       [thread_id: ...]
///    ├ ◑ module.knowledge   · 3 tools · 2.5k tokens
///    ├ ● module.crm         · 1 tool                done
///    └ ◐ module.revenue                              spawning…
///
///    ◑ Synthesizing answer from 2 agents…
/// ```
pub fn render_tree(snapshot: &RunSnapshot) -> String {
    let mut lines = Vec::new();
    let running_count = snapshot
        .agents
        .values()
        .filter(|a| matches!(a.status, AgentStatus::Spawning | AgentStatus::Running))
        .count();
    let header_agent = snapshot.root_agent.as_deref().unwrap_or("Cleo");
    lines.push(format!(
        "{glyph} {agent}  {count} specialist{plural} running          [thread_id: {tid}]",
        glyph = if snapshot.any_running() { "◑" } else { "●" },
        agent = header_agent,
        count = running_count,
        plural = if running_count == 1 { "" } else { "s" },
        tid = snapshot.thread_id,
    ));
    let rows: Vec<&AgentRow> = snapshot.agents.values().collect();
    let last = rows.len().saturating_sub(1);
    for (idx, row) in rows.iter().enumerate() {
        let branch = if idx == last { "   └" } else { "   ├" };
        let tool_word = if row.tool_calls == 1 { "tool" } else { "tools" };
        let tokens_part = if row.tokens > 0 {
            format!(" · {:.1}k tokens", row.tokens as f64 / 1000.0)
        } else {
            String::new()
        };
        let trailing = match row.status {
            AgentStatus::Done => "  done".to_string(),
            AgentStatus::Failed => "  failed".to_string(),
            AgentStatus::Spawning => "  spawning…".to_string(),
            _ => String::new(),
        };
        lines.push(format!(
            "{branch} {glyph} {agent:<22} · {n} {tool_word}{tokens_part}{trailing}",
            glyph = row.status.glyph(),
            agent = row.agent_code,
            n = row.tool_calls,
        ));
        if matches!(row.status, AgentStatus::Running)
            && let Some(tool) = &row.last_tool
        {
            lines.push(format!("       ⎿  calling {tool}…"));
        }
    }
    if snapshot.any_running() && rows.len() > 1 {
        lines.push(String::new());
        lines.push(format!(
            "   ◑ Synthesizing answer from {} agents…",
            rows.len()
        ));
    }
    lines.join("\n")
}
