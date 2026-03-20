//! Capability-state helpers for the Agent Abstract Machine.

pub use apxm_core::CapabilityRecord;
use std::collections::HashMap;

/// Concrete capability storage used by the runtime today.
pub type CapabilityMap = HashMap<String, CapabilityRecord>;

/// Capability-level changes recorded for one transition.
#[derive(Debug, Clone)]
pub enum CapabilityChange {
    Registered {
        name: String,
        metadata: CapabilityRecord,
    },
}

/// Snapshot only a selected subset of capability keys.
pub fn snapshot_subset(capabilities: &CapabilityMap, keys: &[String]) -> CapabilityMap {
    keys.iter()
        .filter_map(|k| capabilities.get(k).map(|v| (k.clone(), v.clone())))
        .collect()
}
