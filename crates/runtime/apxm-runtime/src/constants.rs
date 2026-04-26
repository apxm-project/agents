//! Runtime-owned constants that are not part of the cross-crate APXM contract.

pub(crate) mod env {
    /// Per-node workspace for context files produced by the runtime.
    pub(crate) const APXM_NODE_WORKSPACE: &str = "APXM_NODE_WORKSPACE";
}
