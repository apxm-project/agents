//! Fleet observability (`/v1/observability/fleet`).
//!
//! A single read-only aggregate for the studio Fleet view: per-program and
//! per-goal run history with the supervision-style fields a fleet dashboard
//! needs — restart count, last status, last-stop reason, and uptime.
//!
//! It joins three in-server sources:
//!   - the execution store (skill/program run records) → per-program rollups,
//!   - the goal-run registry (goal-convergence passes)  → per-goal rollups,
//!   - the agent registry (COMMUNICATE/A2A peers)        → registered-agent uptime.
//!
//! Nothing here spawns or supervises processes (that is apxm-os's job); this
//! endpoint surfaces what THIS server already knows so the studio Fleet/
//! observability views have one place to read run health from.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::executions::{ExecutionRecord, ExecutionStatus};
use crate::goal_runs::{GoalRunRecord, GoalRunStatus};
use crate::helpers::now_ms;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub(crate) struct FleetResponse {
    pub(crate) object: &'static str,
    /// Per-program (skill id) run rollups.
    pub(crate) programs: Vec<ProgramFleetEntry>,
    /// Per-goal run rollups.
    pub(crate) goals: Vec<GoalFleetEntry>,
    /// Registered COMMUNICATE/A2A agents with their registration uptime.
    pub(crate) agents: Vec<AgentFleetEntry>,
    /// Wall-clock the response was computed (ms epoch), so a client can age the
    /// uptime fields locally without a second call.
    pub(crate) observed_at_ms: u64,
}

/// Run history rolled up for one program identity (`skill_id`).
#[derive(Debug, Serialize)]
pub(crate) struct ProgramFleetEntry {
    pub(crate) skill_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) skill_version: Option<String>,
    /// Total runs observed — the "restart count" for a program that is launched
    /// repeatedly.
    pub(crate) run_count: usize,
    pub(crate) succeeded: usize,
    pub(crate) failed: usize,
    pub(crate) running: usize,
    /// Status of the most recently started run.
    pub(crate) last_status: ExecutionStatus,
    pub(crate) last_started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_completed_at_ms: Option<u64>,
    /// Error message of the most recent failed run, if any — the "last stop
    /// reason" for a fleet view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_stop_reason: Option<String>,
    /// Duration of the most recent completed run (ms), or elapsed-so-far for a
    /// still-running one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_uptime_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_execution_id: Option<String>,
}

/// Run history rolled up for one goal run.
#[derive(Debug, Serialize)]
pub(crate) struct GoalFleetEntry {
    pub(crate) goal_id: String,
    pub(crate) status: GoalRunStatus,
    /// Convergence passes executed so far — the goal's effective restart count.
    pub(crate) restart_count: usize,
    pub(crate) iteration: usize,
    pub(crate) max_iterations: usize,
    pub(crate) started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) completed_at_ms: Option<u64>,
    /// Uptime: total run duration for a settled goal, else elapsed-so-far.
    pub(crate) uptime_ms: u64,
    /// Last-stop reason: the goal's error for a failed run, else its terminal
    /// status name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_execution_id: Option<String>,
}

/// A registered peer agent with its registration uptime.
#[derive(Debug, Serialize)]
pub(crate) struct AgentFleetEntry {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) registered_at_ms: u64,
    pub(crate) uptime_ms: u64,
    pub(crate) flows: usize,
    pub(crate) capabilities: usize,
}

/// `GET /v1/observability/fleet` — the fleet rollup.
pub(crate) async fn get_fleet(State(state): State<AppState>) -> Json<FleetResponse> {
    let now = now_ms();
    let programs = rollup_programs(state.execution_store.list(), now);
    let goals = state
        .goal_runs
        .list()
        .into_iter()
        .map(|record| goal_entry(record, now))
        .collect();
    let agents = state
        .agent_registry
        .iter()
        .map(|entry| agent_entry(entry.value(), now))
        .collect();

    Json(FleetResponse {
        object: "fleet",
        programs,
        goals,
        agents,
        observed_at_ms: now,
    })
}

/// Group run records by `skill_id` and reduce each group to a fleet entry. The
/// "last" fields track the most recently started run in the group.
fn rollup_programs(records: Vec<ExecutionRecord>, now: u64) -> Vec<ProgramFleetEntry> {
    let mut by_program: BTreeMap<String, ProgramAccumulator> = BTreeMap::new();
    for record in records {
        if record.skill_id.trim().is_empty() {
            continue;
        }
        by_program
            .entry(record.skill_id.clone())
            .or_default()
            .observe(record, now);
    }
    let mut entries: Vec<ProgramFleetEntry> =
        by_program.into_values().map(ProgramAccumulator::finish).collect();
    // Most recently active program first.
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.last_started_at_ms));
    entries
}

#[derive(Default)]
struct ProgramAccumulator {
    skill_id: String,
    skill_version: Option<String>,
    run_count: usize,
    succeeded: usize,
    failed: usize,
    running: usize,
    last: Option<ExecutionRecord>,
}

impl ProgramAccumulator {
    fn observe(&mut self, record: ExecutionRecord, _now: u64) {
        self.skill_id.clone_from(&record.skill_id);
        if self.skill_version.is_none() && !record.skill_version.trim().is_empty() {
            self.skill_version = Some(record.skill_version.clone());
        }
        self.run_count += 1;
        match record.status {
            ExecutionStatus::Succeeded => self.succeeded += 1,
            ExecutionStatus::Failed => self.failed += 1,
            ExecutionStatus::Running => self.running += 1,
        }
        let is_newer = self
            .last
            .as_ref()
            .is_none_or(|cur| record.started_at_ms >= cur.started_at_ms);
        if is_newer {
            self.last = Some(record);
        }
    }

    fn finish(self) -> ProgramFleetEntry {
        // `run_count >= 1` always holds here (an accumulator only exists once a
        // record was observed), so `last` is present.
        let last = self.last.expect("program accumulator has at least one record");
        let last_uptime_ms = last
            .completed_at_ms
            .map(|done| done.saturating_sub(last.started_at_ms));
        ProgramFleetEntry {
            skill_id: self.skill_id,
            skill_version: self.skill_version,
            run_count: self.run_count,
            succeeded: self.succeeded,
            failed: self.failed,
            running: self.running,
            last_status: last.status,
            last_started_at_ms: last.started_at_ms,
            last_completed_at_ms: last.completed_at_ms,
            last_stop_reason: last.error,
            last_uptime_ms,
            last_execution_id: Some(last.execution_id),
        }
    }
}

fn goal_entry(record: GoalRunRecord, now: u64) -> GoalFleetEntry {
    let uptime_ms = record
        .completed_at_ms
        .unwrap_or(now)
        .saturating_sub(record.started_at_ms);
    let last_stop_reason = if record.status.is_terminal() {
        record
            .error
            .clone()
            .or_else(|| Some(terminal_reason(&record.status)))
    } else {
        None
    };
    GoalFleetEntry {
        goal_id: record.goal_id,
        status: record.status,
        restart_count: record.pass_execution_ids.len(),
        iteration: record.iteration,
        max_iterations: record.max_iterations,
        started_at_ms: record.started_at_ms,
        completed_at_ms: record.completed_at_ms,
        uptime_ms,
        last_stop_reason,
        current_execution_id: record.current_execution_id,
    }
}

fn terminal_reason(status: &GoalRunStatus) -> String {
    match status {
        GoalRunStatus::Succeeded => "succeeded",
        GoalRunStatus::Failed => "failed",
        GoalRunStatus::Cancelled => "cancelled",
        GoalRunStatus::Running => "running",
    }
    .to_string()
}

fn agent_entry(agent: &crate::agent::AgentRegistration, now: u64) -> AgentFleetEntry {
    AgentFleetEntry {
        name: agent.name.clone(),
        url: agent.url.clone(),
        registered_at_ms: agent.registered_at,
        uptime_ms: now.saturating_sub(agent.registered_at),
        flows: agent.flows.len(),
        capabilities: agent.capabilities.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execute::ExecuteResponse;
    use crate::types::responses::{ExecutionStats, LlmUsageSummary};

    fn record(
        execution_id: &str,
        skill_id: &str,
        status: ExecutionStatus,
        started: u64,
        completed: Option<u64>,
        error: Option<&str>,
    ) -> ExecutionRecord {
        ExecutionRecord {
            execution_id: execution_id.to_string(),
            skill_id: skill_id.to_string(),
            skill_version: "1.0.0".to_string(),
            entry_flow: None,
            source_hash: None,
            air_hash: None,
            artifact_hash: None,
            parent_execution_id: None,
            parent_skill_id: None,
            parent_skill_version: None,
            scope_id: None,
            session_id: "s".to_string(),
            session_dir: "/tmp/s".to_string(),
            status,
            started_at_ms: started,
            completed_at_ms: completed,
            result: completed.map(|_| ExecuteResponse {
                results: Default::default(),
                content: None,
                session_dir: None,
                stats: ExecutionStats {
                    executed_nodes: 0,
                    failed_nodes: 0,
                    duration_ms: 0,
                },
                llm_usage: LlmUsageSummary {
                    input_tokens: 0,
                    output_tokens: 0,
                    total_requests: 0,
                },
                tool_call_counts: Default::default(),
            }),
            error: error.map(str::to_string),
            node_outputs: Vec::new(),
            node_metrics: Vec::new(),
            token_values: std::collections::HashMap::new(),
            goal: None,
        }
    }

    #[test]
    fn rollup_groups_runs_and_tracks_latest_and_stop_reason() {
        let records = vec![
            record("e1", "writer", ExecutionStatus::Succeeded, 100, Some(150), None),
            record(
                "e2",
                "writer",
                ExecutionStatus::Failed,
                200,
                Some(260),
                Some("model timeout"),
            ),
            record("e3", "reader", ExecutionStatus::Running, 300, None, None),
        ];
        let entries = rollup_programs(records, 400);

        // Most-recently-started program first: "reader" started at 300.
        assert_eq!(entries[0].skill_id, "reader");
        assert_eq!(entries[0].run_count, 1);
        assert_eq!(entries[0].running, 1);
        assert_eq!(entries[0].last_status, ExecutionStatus::Running);

        let writer = entries
            .iter()
            .find(|e| e.skill_id == "writer")
            .expect("writer rolled up");
        assert_eq!(writer.run_count, 2, "restart count = number of runs");
        assert_eq!(writer.succeeded, 1);
        assert_eq!(writer.failed, 1);
        // The latest run (e2, started 200) was a failure — its error is the last
        // stop reason and its duration is the last uptime.
        assert_eq!(writer.last_status, ExecutionStatus::Failed);
        assert_eq!(writer.last_stop_reason.as_deref(), Some("model timeout"));
        assert_eq!(writer.last_uptime_ms, Some(60));
        assert_eq!(writer.last_execution_id.as_deref(), Some("e2"));
    }

    #[test]
    fn raw_runs_with_empty_skill_id_are_skipped() {
        let records = vec![record("e1", "", ExecutionStatus::Succeeded, 100, Some(150), None)];
        assert!(rollup_programs(records, 200).is_empty());
    }
}
