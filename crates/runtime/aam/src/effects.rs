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

        // LLM / verification
        AISOperationType::Ask | AISOperationType::Think | AISOperationType::Verify => {
            OperationEffects::new().read(Beliefs)
        }
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
        // Tool / Invocation
        AISOperationType::InvCap => OperationEffects::new().read(Capabilities),

        // Operations with no AAM effects. Every variant remains explicit so
        // additions to `AISOperationType` still require an effect declaration.
        AISOperationType::Exc
        | AISOperationType::Print
        // Control Flow
        | AISOperationType::Jump
        | AISOperationType::BranchOnValue
        | AISOperationType::Return
        | AISOperationType::Switch
        // Synchronization -- Fence is a pure ordering barrier, no AAM mutation
        | AISOperationType::Fence
        | AISOperationType::Merge
        | AISOperationType::WaitAll
        | AISOperationType::AwaitInput
        // Error Handling
        | AISOperationType::TryCatch
        | AISOperationType::Err
        // Coordination
        | AISOperationType::Pause
        // Identity Operations
        | AISOperationType::Nop
        | AISOperationType::Identity
        // Literals / Metadata
        | AISOperationType::ConstStr
        | AISOperationType::Agent
        | AISOperationType::Yield => OperationEffects::new(),

        // Control Flow
        AISOperationType::FlowCall
        | AISOperationType::WorkflowSpawn
        // Communication
        | AISOperationType::Communicate
        | AISOperationType::Handoff
        // Multi-agent coordination
        | AISOperationType::Delegate => OperationEffects::new().read(Beliefs).write(Beliefs),

        // Synchronization
        AISOperationType::Checkpoint => OperationEffects::new()
            .read(Beliefs)
            .read(Goals)
            .read(Capabilities)
            .write(ShortTermMemory),

        // Coordination
        AISOperationType::UpdateGoal => OperationEffects::new().read(Goals).write(Goals),
        AISOperationType::Resume => OperationEffects::new().write(ShortTermMemory),

        // Self-Organization
        AISOperationType::SpawnAgent
        | AISOperationType::RegisterCapability
        | AISOperationType::RegisterHook => OperationEffects::new().write(Capabilities),

        // Autonomous Execution
        AISOperationType::Autonomous => OperationEffects::new()
            .read(Beliefs)
            .write(Beliefs)
            .read(Goals)
            .write(Goals),
    }
}
