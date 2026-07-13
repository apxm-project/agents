//! Dependency-driven readiness scheduling for `.apxmw` workflow execution.
//!
//! The queue owns workflow-level dependency counters and terminal-state
//! propagation. Callers retain graph execution, session, and authority
//! ownership while using this queue to choose which workflow step may start.

use anyhow::{Result, bail};
use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap, VecDeque};

use super::{
    StepStatus, WorkflowCriticalPathEvidence, WorkflowPlan, WorkflowPlanStep, WorkflowStep,
};

/// Workflow scheduling policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkflowSchedulerMode {
    /// Start a step as soon as its own dependencies succeed.
    #[default]
    Ready,
    /// Retain whole topological-phase waiting for paired baseline comparisons.
    Phased,
}

/// Critical-path priority derived from compiler legality summaries.
///
/// The source omits a step when its artifact lacks complete, versioned
/// reorderability and latency evidence. Such steps remain dependency-ready,
/// but their selection order stays deterministic rather than claiming an
/// optimization the runtime cannot prove.
#[derive(Debug, Clone)]
pub struct WorkflowLegalityPrioritySource {
    weighted_remaining_paths: HashMap<String, u64>,
}

impl WorkflowLegalityPrioritySource {
    /// Build legal weighted remaining paths for one authored workflow.
    pub fn from_critical_path_evidence(
        steps: &[WorkflowStep],
        evidence: &WorkflowCriticalPathEvidence,
    ) -> Result<Self> {
        let plan = WorkflowPlan::build(steps)?;
        Ok(Self::from_plan(&plan, evidence))
    }

    fn from_plan(plan: &WorkflowPlan, evidence: &WorkflowCriticalPathEvidence) -> Self {
        let direct_weights = plan
            .steps()
            .iter()
            .filter_map(|step| {
                evidence
                    .weighted_critical_path_ms(step.id())
                    .map(|weight| (step.declaration_index(), weight))
            })
            .collect::<HashMap<_, _>>();
        let mut weighted_remaining_paths = HashMap::with_capacity(direct_weights.len());

        for phase in plan.phases().iter().rev() {
            for &index in phase.iter().rev() {
                let Some(weight) = direct_weights.get(&index).copied() else {
                    continue;
                };
                let downstream = plan
                    .step(index)
                    .dependents()
                    .iter()
                    .filter_map(|dependent| {
                        weighted_remaining_paths.get(plan.step(*dependent).id())
                    })
                    .copied()
                    .max()
                    .unwrap_or(0);
                weighted_remaining_paths.insert(
                    plan.step(index).id().to_string(),
                    weight.saturating_add(downstream),
                );
            }
        }

        Self {
            weighted_remaining_paths,
        }
    }
}

impl WorkflowLegalityPrioritySource {
    fn weighted_remaining_path(&self, step: &WorkflowPlanStep) -> Option<u64> {
        self.weighted_remaining_paths.get(step.id()).copied()
    }
}

/// Configuration for one workflow ready queue.
#[derive(Clone, Default)]
pub struct WorkflowSchedulerOptions {
    /// Select dependency-ready scheduling or the legacy phased baseline.
    pub mode: WorkflowSchedulerMode,
    /// Bound the number of workflow steps that may be in flight.
    pub max_concurrency: Option<usize>,
    /// Versioned compiler evidence used to derive legal critical-path priority.
    pub critical_path_evidence: Option<WorkflowCriticalPathEvidence>,
    /// Legacy caller-provided latency observations.
    ///
    /// Measurements alone do not prove reordering is legal, so this field does
    /// not enable critical-path priority. Callers must provide
    /// `critical_path_evidence`.
    pub profile_latency_ms: Option<HashMap<String, u64>>,
}

/// A workflow step selected for execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyWorkflowStep {
    /// The workflow-local step identifier.
    pub id: String,
    /// The step's declaration-order index.
    pub index: usize,
}

/// Tracks workflow dependency completion and exposes newly runnable work.
pub struct WorkflowReadyQueue {
    plan: WorkflowPlan,
    states: Vec<WorkflowQueueState>,
    ready: BTreeSet<ReadyKey>,
    terminal_count: usize,
    mode: WorkflowSchedulerMode,
    active_phase: usize,
    max_concurrency: usize,
    priority_source: Option<WorkflowLegalityPrioritySource>,
}

/// Internal lifecycle state for a workflow step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowQueueState {
    /// The step still waits on one or more dependencies.
    Pending {
        remaining_dependencies: usize,
        blocked_by_failed_dependency: bool,
    },
    /// The step can be selected for execution.
    Ready,
    /// The caller has selected the step and it is executing.
    Running,
    /// The step has a final observable result.
    Finished(StepStatus),
}

/// Stable ordering key for ready workflow steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ReadyKey {
    weighted_remaining_path: Reverse<u64>,
    declaration_index: usize,
}

impl WorkflowReadyQueue {
    /// Build the default dependency-ready scheduler from workflow steps.
    pub fn new(steps: &[WorkflowStep]) -> Result<Self> {
        Self::with_options(steps, WorkflowSchedulerOptions::default())
    }

    /// Build a scheduler with an explicit mode, limit, or priority adapter.
    pub fn with_options(steps: &[WorkflowStep], options: WorkflowSchedulerOptions) -> Result<Self> {
        Self::from_plan(WorkflowPlan::build(steps)?, options)
    }

    /// Build a scheduler from an already constructed workflow plan.
    pub fn from_plan(plan: WorkflowPlan, options: WorkflowSchedulerOptions) -> Result<Self> {
        if options.max_concurrency == Some(0) {
            bail!("Workflow max_concurrency must be greater than zero");
        }

        let priority_source = options
            .critical_path_evidence
            .as_ref()
            .map(|evidence| WorkflowLegalityPrioritySource::from_plan(&plan, evidence));

        let mut queue = Self {
            states: plan
                .steps()
                .iter()
                .map(|step| WorkflowQueueState::Pending {
                    remaining_dependencies: step.dependencies().len(),
                    blocked_by_failed_dependency: false,
                })
                .collect(),
            plan,
            ready: BTreeSet::new(),
            terminal_count: 0,
            mode: options.mode,
            active_phase: 0,
            max_concurrency: options.max_concurrency.unwrap_or(usize::MAX),
            priority_source,
        };

        for index in 0..queue.states.len() {
            if matches!(
                queue.states[index],
                WorkflowQueueState::Pending {
                    remaining_dependencies: 0,
                    ..
                }
            ) {
                queue.enqueue_ready(index);
            }
        }

        Ok(queue)
    }

    /// Return the configured workflow-level in-flight limit.
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// Return the immutable plan that backs this queue.
    pub fn plan(&self) -> &WorkflowPlan {
        &self.plan
    }

    /// Return the next eligible workflow step in deterministic order.
    pub fn take_ready(&mut self) -> Option<ReadyWorkflowStep> {
        let key = self.next_ready_key()?;
        Some(self.take_key(key))
    }

    /// Record a terminal step result and return descendants skipped by it.
    pub fn complete(
        &mut self,
        step_id: &str,
        status: StepStatus,
    ) -> Result<Vec<ReadyWorkflowStep>> {
        let index = self.index(step_id)?;
        if !matches!(self.states[index], WorkflowQueueState::Running) {
            bail!("Workflow step '{step_id}' is not running");
        }

        self.states[index] = WorkflowQueueState::Finished(status);
        self.terminal_count += 1;

        let mut skipped = Vec::new();
        let mut terminal_steps = VecDeque::from([(index, status)]);
        while let Some((terminal_index, terminal_status)) = terminal_steps.pop_front() {
            let dependent_indices = self.plan.step(terminal_index).dependents().to_vec();
            for dependent_index in dependent_indices {
                let should_skip = {
                    let state = self
                        .states
                        .get_mut(dependent_index)
                        .expect("planner dependent index is valid");
                    let WorkflowQueueState::Pending {
                        remaining_dependencies,
                        blocked_by_failed_dependency,
                    } = state
                    else {
                        continue;
                    };

                    *remaining_dependencies = remaining_dependencies.saturating_sub(1);
                    *blocked_by_failed_dependency |= terminal_status != StepStatus::Success;
                    *remaining_dependencies == 0 && *blocked_by_failed_dependency
                };

                if should_skip {
                    self.states[dependent_index] =
                        WorkflowQueueState::Finished(StepStatus::Skipped);
                    self.terminal_count += 1;
                    skipped.push(self.step_at(dependent_index));
                    terminal_steps.push_back((dependent_index, StepStatus::Skipped));
                    continue;
                }

                if matches!(
                    self.states[dependent_index],
                    WorkflowQueueState::Pending {
                        remaining_dependencies: 0,
                        blocked_by_failed_dependency: false,
                    }
                ) {
                    self.enqueue_ready(dependent_index);
                }
            }
        }

        Ok(skipped)
    }

    /// Return whether every declared step reached a terminal state.
    pub fn is_complete(&self) -> bool {
        self.terminal_count == self.states.len()
    }

    /// Return the number of steps that have not reached a terminal state.
    pub fn remaining_count(&self) -> usize {
        self.states.len().saturating_sub(self.terminal_count)
    }

    /// Resolve the authored step identifier to its declaration-order index.
    fn index(&self, step_id: &str) -> Result<usize> {
        self.plan
            .index_of(step_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown workflow step '{step_id}'"))
    }

    /// Return the next ready key while honoring the selected scheduling mode.
    fn next_ready_key(&mut self) -> Option<ReadyKey> {
        loop {
            let next = match self.mode {
                WorkflowSchedulerMode::Ready => self.ready.first().copied(),
                WorkflowSchedulerMode::Phased => self
                    .ready
                    .iter()
                    .find(|key| self.plan.step(key.declaration_index).phase() == self.active_phase)
                    .copied(),
            };
            if next.is_some() || self.mode == WorkflowSchedulerMode::Ready {
                return next;
            }
            if !self.phase_is_terminal(self.active_phase) {
                return None;
            }
            self.active_phase += 1;
        }
    }

    /// Return whether the active baseline phase has no pending or running work.
    fn phase_is_terminal(&self, phase_index: usize) -> bool {
        let Some(phase) = self.plan.phases().get(phase_index) else {
            return true;
        };
        phase
            .iter()
            .all(|&index| matches!(self.states[index], WorkflowQueueState::Finished(_)))
    }

    /// Return one ready-step payload by declaration index.
    fn step_at(&self, index: usize) -> ReadyWorkflowStep {
        ReadyWorkflowStep {
            id: self.plan.step(index).id().to_string(),
            index,
        }
    }

    /// Move one selected ready key into its running state.
    fn take_key(&mut self, key: ReadyKey) -> ReadyWorkflowStep {
        self.ready.remove(&key);
        self.states[key.declaration_index] = WorkflowQueueState::Running;
        self.step_at(key.declaration_index)
    }

    /// Mark one workflow step ready and add it to the deterministic queue.
    fn enqueue_ready(&mut self, index: usize) {
        self.states[index] = WorkflowQueueState::Ready;
        let step = self.plan.step(index);
        let weighted_remaining_path = self
            .priority_source
            .as_ref()
            .and_then(|source| source.weighted_remaining_path(step))
            .unwrap_or(0);
        self.ready.insert(ReadyKey {
            weighted_remaining_path: Reverse(weighted_remaining_path),
            declaration_index: step.declaration_index(),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Workflow readiness tests cover timing, terminal propagation, and limits.

    use std::collections::{BTreeMap, HashMap};
    use std::sync::Arc;

    use apxm_core::types::NodeId;
    use apxm_core::types::compiler::{
        DagOptimizationSummaryV1, OperationOptimizationSummaryV1, OptimizationSummaryV1,
    };
    use futures::stream::{FuturesUnordered, StreamExt};
    use tokio::sync::{Semaphore, mpsc};

    use super::*;

    /// One test-controlled workflow step.
    #[derive(Clone)]
    struct ControlledStep {
        release: Arc<Semaphore>,
        status: StepStatus,
    }

    /// Observable workflow scheduling event.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Event {
        Started(String),
        Finished(String, StepStatus),
        Skipped(String),
    }

    /// Build a compact authored workflow step for scheduler tests.
    fn step(id: &str, depends_on: &[&str]) -> WorkflowStep {
        WorkflowStep {
            id: id.to_string(),
            path: format!("{id}.air"),
            depends_on: depends_on
                .iter()
                .map(|dependency| (*dependency).to_string())
                .collect(),
            params: HashMap::new(),
        }
    }

    /// Build one compiler summary with a legal or rejected scheduling weight.
    fn critical_path_summary(
        weighted_critical_path_ms: u64,
        may_reorder: bool,
    ) -> OptimizationSummaryV1 {
        let mut operation = OperationOptimizationSummaryV1 {
            node_id: NodeId::default(),
            operation: "nop".to_string(),
            effect_authority: Default::default(),
            prompt: Default::default(),
            cost: Default::default(),
            backend: Default::default(),
            legality: Default::default(),
            decisions: Vec::new(),
            data_inputs: Vec::new(),
            effect_inputs: Vec::new(),
            control_inputs: Vec::new(),
        };
        operation.legality.may_reorder = may_reorder;
        OptimizationSummaryV1::new(vec![DagOptimizationSummaryV1 {
            weighted_critical_path_ms,
            nodes: vec![operation],
            ..Default::default()
        }])
    }

    /// Build a test-controlled successful workflow step.
    fn succeeds() -> ControlledStep {
        ControlledStep {
            release: Arc::new(Semaphore::new(0)),
            status: StepStatus::Success,
        }
    }

    /// Build a test-controlled failed workflow step.
    fn fails() -> ControlledStep {
        ControlledStep {
            release: Arc::new(Semaphore::new(0)),
            status: StepStatus::Failed,
        }
    }

    /// Run a controlled workflow until every step reaches a terminal state.
    async fn run_controlled(
        steps: Vec<WorkflowStep>,
        options: WorkflowSchedulerOptions,
        controls: Vec<ControlledStep>,
        events: mpsc::UnboundedSender<Event>,
    ) {
        let mut queue = WorkflowReadyQueue::with_options(&steps, options).expect("valid queue");
        let mut in_flight = FuturesUnordered::new();

        while !queue.is_complete() {
            while in_flight.len() < queue.max_concurrency() {
                let Some(scheduled) = queue.take_ready() else {
                    break;
                };
                let index = scheduled.index;
                let step_id = scheduled.id;
                let control = controls[index].clone();
                events
                    .send(Event::Started(step_id.clone()))
                    .expect("event receiver remains open");
                in_flight.push(async move {
                    let permit = control
                        .release
                        .clone()
                        .acquire_owned()
                        .await
                        .expect("test gate remains open");
                    drop(permit);
                    (index, step_id, control.status)
                });
            }

            let Some((index, step_id, status)) = in_flight.next().await else {
                panic!("scheduler stalled before all workflow steps became terminal");
            };
            events
                .send(Event::Finished(step_id.clone(), status))
                .expect("event receiver remains open");
            for skipped in queue
                .complete(&step_id, status)
                .expect("completion accepted")
            {
                events
                    .send(Event::Skipped(skipped.id))
                    .expect("event receiver remains open");
            }
            assert_eq!(queue.states[index], WorkflowQueueState::Finished(status));
        }
    }

    #[tokio::test]
    async fn ready_mode_starts_fast_branch_dependent_before_slow_sibling_finishes() {
        let controls = vec![succeeds(), succeeds(), succeeds()];
        let fast = controls[0].clone();
        let slow = controls[1].clone();
        let after_fast = controls[2].clone();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let runner = tokio::spawn(run_controlled(
            vec![
                step("fast", &[]),
                step("slow", &[]),
                step("after_fast", &["fast"]),
            ],
            WorkflowSchedulerOptions::default(),
            controls,
            sender,
        ));

        assert_eq!(
            receiver.recv().await,
            Some(Event::Started("fast".to_string()))
        );
        assert_eq!(
            receiver.recv().await,
            Some(Event::Started("slow".to_string()))
        );
        fast.release.add_permits(1);
        assert_eq!(
            receiver.recv().await,
            Some(Event::Finished("fast".to_string(), StepStatus::Success))
        );
        assert_eq!(
            receiver.recv().await,
            Some(Event::Started("after_fast".to_string()))
        );
        assert!(receiver.try_recv().is_err());

        after_fast.release.add_permits(1);
        slow.release.add_permits(1);
        runner.await.expect("runner joins");
    }

    #[tokio::test]
    async fn phased_mode_retains_whole_phase_waiting_for_paired_baselines() {
        let controls = vec![succeeds(), succeeds(), succeeds()];
        let fast = controls[0].clone();
        let slow = controls[1].clone();
        let after_fast = controls[2].clone();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let runner = tokio::spawn(run_controlled(
            vec![
                step("fast", &[]),
                step("slow", &[]),
                step("after_fast", &["fast"]),
            ],
            WorkflowSchedulerOptions {
                mode: WorkflowSchedulerMode::Phased,
                ..Default::default()
            },
            controls,
            sender,
        ));

        assert_eq!(
            receiver.recv().await,
            Some(Event::Started("fast".to_string()))
        );
        assert_eq!(
            receiver.recv().await,
            Some(Event::Started("slow".to_string()))
        );
        fast.release.add_permits(1);
        assert_eq!(
            receiver.recv().await,
            Some(Event::Finished("fast".to_string(), StepStatus::Success))
        );
        assert!(receiver.try_recv().is_err());

        slow.release.add_permits(1);
        assert_eq!(
            receiver.recv().await,
            Some(Event::Finished("slow".to_string(), StepStatus::Success))
        );
        assert_eq!(
            receiver.recv().await,
            Some(Event::Started("after_fast".to_string()))
        );
        after_fast.release.add_permits(1);
        runner.await.expect("runner joins");
    }

    #[tokio::test]
    async fn failure_skips_transitive_dependents_without_blocking_independent_work() {
        let controls = vec![fails(), succeeds(), succeeds(), succeeds()];
        let failing = controls[0].clone();
        let independent = controls[3].clone();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let runner = tokio::spawn(run_controlled(
            vec![
                step("failing", &[]),
                step("blocked", &["failing"]),
                step("blocked_child", &["blocked"]),
                step("independent", &[]),
            ],
            WorkflowSchedulerOptions::default(),
            controls,
            sender,
        ));

        assert_eq!(
            receiver.recv().await,
            Some(Event::Started("failing".to_string()))
        );
        assert_eq!(
            receiver.recv().await,
            Some(Event::Started("independent".to_string()))
        );
        failing.release.add_permits(1);
        assert_eq!(
            receiver.recv().await,
            Some(Event::Finished("failing".to_string(), StepStatus::Failed))
        );
        assert_eq!(
            receiver.recv().await,
            Some(Event::Skipped("blocked".to_string()))
        );
        assert_eq!(
            receiver.recv().await,
            Some(Event::Skipped("blocked_child".to_string()))
        );
        assert!(receiver.try_recv().is_err());

        independent.release.add_permits(1);
        runner.await.expect("runner joins");
    }

    #[tokio::test]
    async fn explicit_concurrency_limit_keeps_ready_work_bounded_and_ordered() {
        let controls = vec![succeeds(), succeeds()];
        let first = controls[0].clone();
        let second = controls[1].clone();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let runner = tokio::spawn(run_controlled(
            vec![step("first", &[]), step("second", &[])],
            WorkflowSchedulerOptions {
                max_concurrency: Some(1),
                ..Default::default()
            },
            controls,
            sender,
        ));

        assert_eq!(
            receiver.recv().await,
            Some(Event::Started("first".to_string()))
        );
        assert!(receiver.try_recv().is_err());
        first.release.add_permits(1);
        assert_eq!(
            receiver.recv().await,
            Some(Event::Finished("first".to_string(), StepStatus::Success))
        );
        assert_eq!(
            receiver.recv().await,
            Some(Event::Started("second".to_string()))
        );
        second.release.add_permits(1);
        runner.await.expect("runner joins");
    }

    #[test]
    fn failed_dependency_waits_for_all_prerequisites_before_skip_propagation() {
        let steps = vec![
            step("fails", &[]),
            step("succeeds", &[]),
            step("join", &["fails", "succeeds"]),
        ];
        let mut queue = WorkflowReadyQueue::new(&steps).expect("valid queue");

        assert_eq!(queue.take_ready().expect("failing root").id, "fails");
        assert_eq!(queue.take_ready().expect("successful root").id, "succeeds");
        assert!(
            queue
                .complete("fails", StepStatus::Failed)
                .expect("failure completion")
                .is_empty()
        );
        assert!(!queue.is_complete());

        assert_eq!(
            queue
                .complete("succeeds", StepStatus::Success)
                .expect("success completion")
                .into_iter()
                .map(|step| step.id)
                .collect::<Vec<_>>(),
            vec!["join"]
        );
    }

    #[test]
    fn compiler_legality_evidence_prioritizes_the_longest_remaining_critical_path() {
        let steps = vec![
            step("short", &[]),
            step("critical", &[]),
            step("critical_tail", &["critical"]),
        ];
        let mut queue = WorkflowReadyQueue::with_options(
            &steps,
            WorkflowSchedulerOptions {
                critical_path_evidence: Some(WorkflowCriticalPathEvidence {
                    artifacts: BTreeMap::from([
                        ("short".to_string(), critical_path_summary(1, true)),
                        ("critical".to_string(), critical_path_summary(20, true)),
                        ("critical_tail".to_string(), critical_path_summary(30, true)),
                    ]),
                }),
                ..Default::default()
            },
        )
        .expect("valid queue");

        assert_eq!(queue.take_ready().expect("critical root").id, "critical");
        assert_eq!(queue.take_ready().expect("short root").id, "short");
    }

    #[test]
    fn unproven_critical_paths_preserve_declaration_order() {
        let steps = vec![step("first", &[]), step("unproven", &[])];
        let mut queue = WorkflowReadyQueue::with_options(
            &steps,
            WorkflowSchedulerOptions {
                critical_path_evidence: Some(WorkflowCriticalPathEvidence {
                    artifacts: BTreeMap::from([
                        ("first".to_string(), critical_path_summary(1, true)),
                        ("unproven".to_string(), critical_path_summary(100, false)),
                    ]),
                }),
                profile_latency_ms: Some(HashMap::from([("unproven".to_string(), 100)])),
                ..Default::default()
            },
        )
        .expect("valid queue");

        assert_eq!(queue.take_ready().expect("first root").id, "first");
        assert_eq!(queue.take_ready().expect("unproven root").id, "unproven");
    }
}
