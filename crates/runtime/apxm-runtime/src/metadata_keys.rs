//! Execution metadata keys produced and consumed by the runtime.

pub const PARENT_EXECUTION_ID: &str = "parent_execution_id";
pub const SCOPE_ID: &str = "scope_id";
pub const PARENT_SCOPE_ID: &str = "parent_scope_id";
pub const SESSION_DIR: &str = "session_dir";
pub const SESSION_ROOT: &str = "session_root";
pub const DELEGATE_TASK_SPEC: &str = "delegate_task_spec";
pub const DELEGATE_TARGET: &str = "delegate_target";
pub const NEGOTIATE_PROPOSAL: &str = "negotiate_proposal";
pub const NEGOTIATE_ROUND: &str = "negotiate_round";
pub const NEGOTIATE_PARTY: &str = "negotiate_party";
pub const COMMUNICATE_SENDER: &str = "communicate_sender";
pub const COMMUNICATE_RECIPIENT: &str = "communicate_recipient";
pub const COMMUNICATE_MODE: &str = "communicate_mode";
pub const FLOW_CALL_DEPTH: &str = "flow_call_depth";
pub const CALL_SKILL_DEPTH: &str = "call_skill_depth";
/// The effective side-effect policy granted to the current execution (wire
/// form of `apxm_skill::CapabilityPolicy`). Seeded at the top-level execution
/// from the launching skill manifest and propagated to children so CALL_SKILL
/// admission can enforce `child ⊆ parent`.
pub const SIDE_EFFECT_POLICY: &str = "side_effect_policy";
pub const TARGET_AGENT: &str = "target_agent";
pub const TARGET_FLOW: &str = "target_flow";
pub const TARGET_SKILL_ID: &str = "target_skill_id";
pub const TARGET_SKILL_VERSION: &str = "target_skill_version";
