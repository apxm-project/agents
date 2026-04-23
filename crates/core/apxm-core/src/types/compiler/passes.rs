//! Runtime-facing pass registry types.
//!
//! These local types are used for FFI/registry reporting. Canonical compiler
//! pass metadata lives in [`super::metadata`], which re-exports the AIS pass
//! catalog without duplicating definitions.

/// Pass category for filtering and introspection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PassCategory {
    Analysis = 0,
    Transform = 1,
    Optimization = 2,
    Lowering = 3,
}

impl From<u32> for PassCategory {
    fn from(value: u32) -> Self {
        match value {
            0 => PassCategory::Analysis,
            1 => PassCategory::Transform,
            2 => PassCategory::Optimization,
            3 => PassCategory::Lowering,
            _ => PassCategory::Transform,
        }
    }
}

/// Information about a registered pass
#[derive(Debug, Clone)]
pub struct PassInfo {
    pub name: String,
    pub description: String,
    pub category: PassCategory,
}
