use std::sync::Arc;

use apxm_runtime::ExecutionEventEmitter;

use crate::config::HookConfig;

pub mod multi;
pub mod subprocess;

pub fn emitter_from_config(hooks: &[HookConfig]) -> Option<Arc<dyn ExecutionEventEmitter>> {
    if hooks.is_empty() {
        None
    } else {
        Some(Arc::new(subprocess::SubprocessHookEmitter::new(
            hooks.to_vec(),
        )))
    }
}

pub fn compose_emitters(
    primary: Option<Arc<dyn ExecutionEventEmitter>>,
    secondary: Option<Arc<dyn ExecutionEventEmitter>>,
) -> Option<Arc<dyn ExecutionEventEmitter>> {
    match (primary, secondary) {
        (Some(primary), Some(secondary)) => {
            Some(Arc::new(multi::MultiEmitter::new(vec![primary, secondary])))
        }
        (Some(emitter), None) | (None, Some(emitter)) => Some(emitter),
        (None, None) => None,
    }
}
