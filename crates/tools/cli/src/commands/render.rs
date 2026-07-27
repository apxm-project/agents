//!.F — shared monospace render contract for `apxm watch` and
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
    /// Wire kind name → count, for every event `apply` saw but had no
    /// dedicated projection for (most Layer-2 kinds today:
    /// `subagent_spawn_begin`/`tool_call_begin`/…). `render_tree` surfaces
    /// this as a visible "unrecognized" line instead of the previous
    /// silent drop — never crashing or hanging the tree view on a kind it
    /// doesn't know how to project yet.
    unrecognized_kinds: BTreeMap<String, u64>,
}

impl RunSnapshot {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            ..Self::default()
        }
    }

    /// Fold one event into the snapshot. Event kinds the renderer doesn't
    /// have a dedicated agent/tool/op projection for are no longer
    /// silently dropped — they're counted in `unrecognized_kinds` so
    /// `render_tree` can surface a visible line instead (most Layer-2
    /// kinds fall here today; this is additive and never blocks the
    /// canonical shapes above).
    pub fn apply(&mut self, event: &ApxmEvent) {
        let mut recognized = false;
        if let Some(payload) = event.payload.downcast_ref::<AgentSpawnedPayload>() {
            recognized = true;
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
            recognized = true;
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
        if let Some(payload) = event.payload.downcast_ref::<OperationEndPayload>() {
            recognized = true;
            if let Some(code) = self.node_to_agent.get(&payload.node_id).cloned()
                && let Some(entry) = self.agents.get_mut(&code)
            {
                entry.status = if payload.success {
                    AgentStatus::Done
                } else {
                    AgentStatus::Failed
                };
            }
        }
        if let Some(payload) = event.payload.downcast_ref::<ToolStartPayload>() {
            recognized = true;
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
            recognized = true;
            let _ = payload; // Tool end is informational; status flips via OperationEnd.
        }
        // Token accounting comes off NodeMetrics events; project node_id
        // back to the owning agent via the spawn-time reverse map.
        if let Some(payload) = event.payload.downcast_ref::<NodeMetricsPayload>() {
            recognized = true;
            if let Some(code) = self.node_to_agent.get(&payload.node_id).cloned()
                && let Some(entry) = self.agents.get_mut(&code)
            {
                entry.tokens += payload.metrics.processes.totals.input_tokens as u64
                    + payload.metrics.processes.totals.output_tokens as u64;
            }
        }

        if !recognized {
            *self
                .unrecognized_kinds
                .entry(event.kind().name().to_string())
                .or_insert(0) += 1;
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
/// Layout shape:
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
    if !snapshot.unrecognized_kinds.is_empty() {
        let kinds = snapshot
            .unrecognized_kinds
            .iter()
            .map(|(kind, count)| {
                if *count > 1 {
                    format!("{kind} x{count}")
                } else {
                    kind.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("   ⚠ unrecognized events: {kinds}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::events::EventSource;
    use apxm_core::events::payload::{
        AgentMessagePayload, SubagentSpawnBeginPayload, ToolCallBeginPayload,
    };

    /// `watch_tree_reflects_layer2_events` — `subagent_spawn_begin`,
    /// `tool_call_begin`, and `agent_message` each change `render_tree`'s
    /// output. `RunSnapshot::apply` must fold every one of these in rather
    /// than silently dropping it, which would leave `render_tree` output
    /// byte-identical.
    #[test]
    fn watch_tree_reflects_layer2_events() {
        let mut snapshot = RunSnapshot::new("thread-1");
        let before = render_tree(&snapshot);

        snapshot.apply(&ApxmEvent::root(
            SubagentSpawnBeginPayload {
                agent_code: "agent-1".to_string(),
                agent_name: None,
                agent_type: None,
                module_key: None,
                autonomy_policy: None,
                parent_span_id: None,
            },
            EventSource::Runtime,
            "trace-1",
        ));
        let after_spawn_begin = render_tree(&snapshot);
        assert_ne!(
            before, after_spawn_begin,
            "subagent_spawn_begin must change render_tree output"
        );

        snapshot.apply(&ApxmEvent::root(
            ToolCallBeginPayload {
                agent_code: "agent-1".to_string(),
                tool_name: "web_search".to_string(),
                argument_keys: vec!["q".to_string()],
                tool_call_correlation: None,
            },
            EventSource::Runtime,
            "trace-1",
        ));
        let after_tool_call_begin = render_tree(&snapshot);
        assert_ne!(
            after_spawn_begin, after_tool_call_begin,
            "tool_call_begin must change render_tree output"
        );

        snapshot.apply(&ApxmEvent::root(
            AgentMessagePayload {
                text: "done".to_string(),
                item_id: None,
                response_id: None,
                usage: None,
            },
            EventSource::Runtime,
            "trace-1",
        ));
        let after_agent_message = render_tree(&snapshot);
        assert_ne!(
            after_tool_call_begin, after_agent_message,
            "agent_message must change render_tree output"
        );

        assert!(
            after_agent_message.contains("agent_message")
                || after_agent_message.contains("subagent_spawn_begin")
                || after_agent_message.contains("tool_call_begin"),
            "unrecognized Layer-2 kinds must be visibly surfaced, not silently dropped: {after_agent_message}"
        );
    }
}
