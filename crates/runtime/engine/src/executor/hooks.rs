//! Scheduler-level execution hooks.
//!
//! These hooks complement dispatcher middleware. Middleware wraps the actual
//! operation handler, while execution hooks observe scheduler lifecycle events
//! such as ready, start, finish, and graph completion.

use std::sync::Arc;

use apxm_core::types::{AISOperationType, NodeId, OpStatus};

/// Observer interface for runtime execution lifecycle events.
pub trait ExecutionHook: Send + Sync {
    /// Stable hook name for diagnostics.
    fn name(&self) -> &str {
        "execution-hook"
    }

    /// Called when a graph starts running.
    fn on_graph_started(&self, _event: &GraphStartedEvent) {}

    /// Called when a node becomes ready and is admitted to the scheduler queue.
    fn on_node_ready(&self, _event: &NodeReadyEvent) {}

    /// Called when a worker starts executing a ready node.
    fn on_node_started(&self, _event: &NodeStartedEvent) {}

    /// Called when a node reaches a terminal scheduler state.
    fn on_node_finished(&self, _event: &NodeFinishedEvent) {}

    /// Called when a graph finishes or fails.
    fn on_graph_finished(&self, _event: &GraphFinishedEvent) {}
}

/// Immutable hook set for one graph execution.
#[derive(Clone, Default)]
pub struct ExecutionHookContext {
    execution_id: String,
    graph_id: String,
    hooks: Vec<Arc<dyn ExecutionHook>>,
}

impl ExecutionHookContext {
    pub fn new(
        execution_id: impl Into<String>,
        graph_id: impl Into<String>,
        hooks: Vec<Arc<dyn ExecutionHook>>,
    ) -> Self {
        Self {
            execution_id: execution_id.into(),
            graph_id: graph_id.into(),
            hooks,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }

    pub fn graph_id(&self) -> &str {
        &self.graph_id
    }

    pub fn emit_graph_started(&self, node_count: usize) {
        if self.is_empty() {
            return;
        }
        let event = GraphStartedEvent {
            execution_id: self.execution_id.clone(),
            graph_id: self.graph_id.clone(),
            node_count,
        };
        for hook in &self.hooks {
            hook.on_graph_started(&event);
        }
    }

    pub fn emit_node_ready(&self, event: NodeReadyEvent) {
        if self.is_empty() {
            return;
        }
        for hook in &self.hooks {
            hook.on_node_ready(&event);
        }
    }

    pub fn emit_node_started(&self, event: NodeStartedEvent) {
        if self.is_empty() {
            return;
        }
        for hook in &self.hooks {
            hook.on_node_started(&event);
        }
    }

    pub fn emit_node_finished(&self, event: NodeFinishedEvent) {
        if self.is_empty() {
            return;
        }
        for hook in &self.hooks {
            hook.on_node_finished(&event);
        }
    }

    pub fn emit_graph_finished(
        &self,
        executed_nodes: usize,
        failed_nodes: usize,
        duration_ms: u128,
        success: bool,
    ) {
        if self.is_empty() {
            return;
        }
        let event = GraphFinishedEvent {
            execution_id: self.execution_id.clone(),
            graph_id: self.graph_id.clone(),
            executed_nodes,
            failed_nodes,
            duration_ms,
            success,
        };
        for hook in &self.hooks {
            hook.on_graph_finished(&event);
        }
    }
}

#[derive(Debug, Clone)]
pub struct GraphStartedEvent {
    pub execution_id: String,
    pub graph_id: String,
    pub node_count: usize,
}

#[derive(Debug, Clone)]
pub struct NodeReadyEvent {
    pub execution_id: String,
    pub graph_id: String,
    pub node_id: NodeId,
    pub op_type: AISOperationType,
    pub priority: String,
    pub ready_at_ms: u128,
}

#[derive(Debug, Clone)]
pub struct NodeStartedEvent {
    pub execution_id: String,
    pub graph_id: String,
    pub node_id: NodeId,
    pub op_type: AISOperationType,
    pub priority: String,
    pub worker_id: Option<usize>,
    pub ready_at_ms: Option<u128>,
    pub started_at_ms: u128,
    pub queue_wait_ms: Option<u128>,
}

#[derive(Debug, Clone)]
pub struct NodeFinishedEvent {
    pub execution_id: String,
    pub graph_id: String,
    pub node_id: NodeId,
    pub op_type: AISOperationType,
    pub status: OpStatus,
    pub attempts: u32,
    pub started_at_ms: Option<u128>,
    pub finished_at_ms: u128,
    pub duration_ms: Option<u128>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GraphFinishedEvent {
    pub execution_id: String,
    pub graph_id: String,
    pub executed_nodes: usize,
    pub failed_nodes: usize,
    pub duration_ms: u128,
    pub success: bool,
}

// ===========================================================================
// Program-authored lifecycle hooks (the `@hook` mechanism).
//
// Distinct from the `ExecutionHook` observer trait above: these are
// program-authored bindings (from the artifact's hooks sidecar / `REGISTER_HOOK`
// op) that drive the SAME Python handler path as `@tool` (constitution #4). The
// registry lives on `ExecutionContext` (lifetime = the python tool bridge) and
// is inherited by child contexts. Drivers (the interceptor/middleware/async
// pre-step) consume it.
// ===========================================================================

/// Conversational lifecycle events a `@hook` can bind to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart,
    PreTurn,
    PostTurn,
    PreAsk,
    PostAsk,
    PreCap,
    PostCap,
}

impl HookEvent {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "session_start" => Some(Self::SessionStart),
            "pre_turn" => Some(Self::PreTurn),
            "post_turn" => Some(Self::PostTurn),
            "pre_ask" => Some(Self::PreAsk),
            "post_ask" => Some(Self::PostAsk),
            "pre_cap" => Some(Self::PreCap),
            "post_cap" => Some(Self::PostCap),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStart => "session_start",
            Self::PreTurn => "pre_turn",
            Self::PostTurn => "post_turn",
            Self::PreAsk => "pre_ask",
            Self::PostAsk => "post_ask",
            Self::PreCap => "pre_cap",
            Self::PostCap => "post_cap",
        }
    }

    /// Pre-execution events are the only ones that may `gate` (allow/deny/edit).
    pub fn is_pre(&self) -> bool {
        matches!(
            self,
            Self::SessionStart | Self::PreTurn | Self::PreAsk | Self::PreCap
        )
    }
}

/// Whether a hook may only observe or may gate (control) the guarded action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookMode {
    Observe,
    Gate,
}

impl HookMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "observe" => Some(Self::Observe),
            "gate" => Some(Self::Gate),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Gate => "gate",
        }
    }
}

/// One author hook binding: a Python handler bound to a lifecycle event.
#[derive(Debug, Clone)]
pub struct HookBinding {
    /// Python handler id, dispatched via the shared `PythonHandlerBridge`.
    pub handler_id: String,
    pub event: HookEvent,
    /// Glob over tool/op name this hook applies to (`*` = all).
    pub match_glob: String,
    pub mode: HookMode,
}

impl HookBinding {
    /// Does this hook apply to a tool/op named `name`? Supports `*` wildcards.
    pub fn matches(&self, name: &str) -> bool {
        glob_match(&self.match_glob, name)
    }
}

/// Per-artifact registry of author hook bindings, resolved from `REGISTER_HOOK`
/// nodes at runtime (and/or the hooks sidecar at load). Threaded as
/// `Arc<HookRegistry>` on `ExecutionContext` (lifetime = the python tool bridge)
/// and inherited by child contexts, so registration by a `REGISTER_HOOK` node in
/// the entry flow is visible to later turn/tool nodes. Interior mutability lets
/// the shared (`Arc`) registry be populated through `&self`.
#[derive(Debug, Default)]
pub struct HookRegistry {
    bindings: std::sync::Mutex<Vec<HookBinding>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings
            .lock()
            .expect("hook registry poisoned")
            .is_empty()
    }

    pub fn len(&self) -> usize {
        self.bindings.lock().expect("hook registry poisoned").len()
    }

    pub fn register(&self, binding: HookBinding) {
        self.bindings
            .lock()
            .expect("hook registry poisoned")
            .push(binding);
    }

    /// All bindings for `event`, in registration order (owned clones so the
    /// lock is not held by the caller).
    pub fn for_event(&self, event: HookEvent) -> Vec<HookBinding> {
        self.bindings
            .lock()
            .expect("hook registry poisoned")
            .iter()
            .filter(|b| b.event == event)
            .cloned()
            .collect()
    }

    /// Bindings for `event` whose `match` glob applies to `name`.
    pub fn matching(&self, event: HookEvent, name: &str) -> Vec<HookBinding> {
        self.bindings
            .lock()
            .expect("hook registry poisoned")
            .iter()
            .filter(|b| b.event == event && b.matches(name))
            .cloned()
            .collect()
    }
}

/// Minimal glob match supporting `*` wildcards (anywhere in the pattern). An
/// empty pattern or `*` matches everything.
fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern.is_empty() || pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == name;
    }
    // Split on `*` and match the literal segments in order, anchoring the first
    // and last segments to the start/end unless the pattern begins/ends with `*`.
    let segments: Vec<&str> = pattern.split('*').collect();
    let mut pos = 0usize;
    for (i, seg) in segments.iter().enumerate() {
        if seg.is_empty() {
            continue;
        }
        // `name.get(pos..)` returns None if `pos` is not a char boundary, so a
        // non-ASCII name can never panic on a byte slice (returns no-match).
        let rest = match name.get(pos..) {
            Some(r) => r,
            None => return false,
        };
        if i == 0 {
            if !rest.starts_with(seg) {
                return false;
            }
            pos += seg.len();
        } else if i == segments.len() - 1 {
            if !rest.ends_with(seg) {
                return false;
            }
        } else if let Some(found) = rest.find(seg) {
            pos += found + seg.len();
        } else {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod hook_registry_tests {
    use super::*;

    #[test]
    fn event_round_trips() {
        for s in [
            "session_start",
            "pre_turn",
            "post_turn",
            "pre_ask",
            "post_ask",
            "pre_cap",
            "post_cap",
        ] {
            assert_eq!(HookEvent::parse(s).unwrap().as_str(), s);
        }
        assert!(HookEvent::parse("nope").is_none());
    }

    #[test]
    fn only_pre_events_gate() {
        assert!(HookEvent::PreCap.is_pre());
        assert!(HookEvent::SessionStart.is_pre());
        assert!(!HookEvent::PostCap.is_pre());
        assert!(!HookEvent::PostAsk.is_pre());
    }

    #[test]
    fn glob_matching() {
        assert!(glob_match("*", "lookup"));
        assert!(glob_match("", "lookup"));
        assert!(glob_match("lookup", "lookup"));
        assert!(!glob_match("lookup", "search"));
        assert!(glob_match("look*", "lookup"));
        assert!(glob_match("*up", "lookup"));
        assert!(glob_match("l*p", "lookup"));
        assert!(!glob_match("x*y", "lookup"));
        // Non-ASCII names must not panic on byte slicing (m2).
        assert!(glob_match("*", "café_tool"));
        assert!(glob_match("café*", "café_tool"));
        assert!(!glob_match("a*z", "café"));
        assert!(glob_match("c*é", "café"));
    }

    /// The turn/ask lifecycle events the turn drivers consume (`pre_turn`,
    /// `post_turn`, `post_ask`) must be registrable and discoverable via
    /// `for_event` — i.e. not dead surface.
    #[test]
    fn lifecycle_turn_events_are_queryable() {
        let reg = HookRegistry::new();
        for ev in [HookEvent::PreTurn, HookEvent::PostTurn, HookEvent::PostAsk] {
            reg.register(HookBinding {
                handler_id: format!("h-{}", ev.as_str()),
                event: ev,
                match_glob: "*".into(),
                mode: HookMode::Observe,
            });
        }
        assert_eq!(reg.for_event(HookEvent::PreTurn).len(), 1);
        assert_eq!(reg.for_event(HookEvent::PostTurn).len(), 1);
        assert_eq!(reg.for_event(HookEvent::PostAsk).len(), 1);
        // post_turn carries the reply (per contract); event round-trips.
        assert_eq!(
            reg.for_event(HookEvent::PostTurn)[0].handler_id,
            "h-post_turn"
        );
    }

    #[test]
    fn registry_filters_by_event_and_match() {
        let reg = HookRegistry::new();
        reg.register(HookBinding {
            handler_id: "h1".into(),
            event: HookEvent::PreCap,
            match_glob: "lookup".into(),
            mode: HookMode::Gate,
        });
        reg.register(HookBinding {
            handler_id: "h2".into(),
            event: HookEvent::PreCap,
            match_glob: "*".into(),
            mode: HookMode::Observe,
        });
        reg.register(HookBinding {
            handler_id: "h3".into(),
            event: HookEvent::PostCap,
            match_glob: "*".into(),
            mode: HookMode::Observe,
        });
        assert_eq!(reg.len(), 3);
        let m = reg.matching(HookEvent::PreCap, "lookup");
        assert_eq!(m.len(), 2);
        let m2 = reg.matching(HookEvent::PreCap, "search");
        assert_eq!(m2.len(), 1);
        assert_eq!(m2[0].handler_id, "h2");
    }
}
