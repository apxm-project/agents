//! Effect metadata for AIS operations interacting with the AAM.

use apxm_core::types::operations::AISOperationType;
use std::collections::HashSet;

/// Logical component of the Agent Abstract Machine/memory touched by an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AamComponent {
    Beliefs,
    Goals,
    Capabilities,
    ShortTermMemory,
    LongTermMemory,
    Episodic,
}

/// Read/write set for an operation.
#[derive(Debug, Clone, Default)]
pub struct OperationEffects {
    pub reads: HashSet<AamComponent>,
    pub writes: HashSet<AamComponent>,
    pub has_side_effects: bool,
}

impl OperationEffects {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read(mut self, component: AamComponent) -> Self {
        self.reads.insert(component);
        self
    }

    pub fn write(mut self, component: AamComponent) -> Self {
        self.writes.insert(component);
        self.has_side_effects = true;
        self
    }

    pub fn can_reorder_with(&self, other: &OperationEffects) -> bool {
        self.reads.is_disjoint(&other.writes)
            && self.writes.is_disjoint(&other.reads)
            && self.writes.is_disjoint(&other.writes)
    }
}

/// Effect declarations for every AIS operation.
///
/// Every variant is listed explicitly (no wildcard) so that adding a new
/// `AISOperationType` forces a compilation error here, ensuring the
/// declaration stays in sync with the enum.
pub fn operation_effects(op: &AISOperationType) -> OperationEffects {
    use AamComponent::*;

    match op {
        // Memory
        AISOperationType::QMem => OperationEffects::new().read(Beliefs).read(ShortTermMemory),
        AISOperationType::UMem => OperationEffects::new()
            .write(Beliefs)
            .write(ShortTermMemory),

        // LLM
        AISOperationType::Ask => OperationEffects::new().read(Beliefs),
        AISOperationType::Think => OperationEffects::new().read(Beliefs),
        AISOperationType::Reason => OperationEffects::new()
            .read(Beliefs)
            .write(Beliefs)
            .write(Goals),

        // Planning & Analysis
        AISOperationType::Plan => OperationEffects::new()
            .read(Goals)
            .write(Goals)
            .write(Beliefs),
        AISOperationType::Reflect => OperationEffects::new().read(Episodic).write(Episodic),
        AISOperationType::Verify => OperationEffects::new().read(Beliefs),

        // Tool / Invocation
        AISOperationType::InvCap => OperationEffects::new().read(Capabilities),
        AISOperationType::Exc => OperationEffects::new(),
        AISOperationType::Print => OperationEffects::new(),

        // Control Flow
        AISOperationType::Jump => OperationEffects::new(),
        AISOperationType::BranchOnValue => OperationEffects::new(),
        AISOperationType::Return => OperationEffects::new(),
        AISOperationType::Switch => OperationEffects::new(),
        AISOperationType::FlowCall => OperationEffects::new().read(Beliefs).write(Beliefs),
        AISOperationType::WorkflowSpawn => OperationEffects::new().read(Beliefs).write(Beliefs),
        AISOperationType::CallSkill => OperationEffects::new().read(Beliefs).write(Beliefs),

        // Synchronization -- Fence is a pure ordering barrier, no AAM mutation
        AISOperationType::Fence => OperationEffects::new(),
        AISOperationType::Merge => OperationEffects::new(),
        AISOperationType::WaitAll => OperationEffects::new(),
        AISOperationType::Checkpoint => OperationEffects::new()
            .read(Beliefs)
            .read(Goals)
            .read(Capabilities)
            .write(ShortTermMemory),

        // Error Handling
        AISOperationType::TryCatch => OperationEffects::new(),
        AISOperationType::Err => OperationEffects::new(),

        // Communication
        AISOperationType::Communicate => OperationEffects::new().read(Beliefs).write(Beliefs),
        AISOperationType::Handoff => OperationEffects::new().read(Beliefs).write(Beliefs),

        // Coordination
        AISOperationType::UpdateGoal => OperationEffects::new().read(Goals).write(Goals),
        AISOperationType::Pause => OperationEffects::new(),
        AISOperationType::Resume => OperationEffects::new().write(ShortTermMemory),

        // Multi-agent coordination
        AISOperationType::Delegate => OperationEffects::new().read(Beliefs).write(Beliefs),

        // Identity Operations
        AISOperationType::Nop => OperationEffects::new(),
        AISOperationType::Identity => OperationEffects::new(),

        // Self-Organization
        AISOperationType::SpawnAgent => OperationEffects::new().write(Capabilities),
        AISOperationType::RegisterCapability => OperationEffects::new().write(Capabilities),
        AISOperationType::RegisterHook => OperationEffects::new().write(Capabilities),

        // Autonomous Execution
        AISOperationType::Autonomous => OperationEffects::new()
            .read(Beliefs)
            .write(Beliefs)
            .read(Goals)
            .write(Goals),

        // Literals / Metadata
        AISOperationType::ConstStr => OperationEffects::new(),
        AISOperationType::Agent => OperationEffects::new(),
        AISOperationType::Yield => OperationEffects::new(),
    }
}
