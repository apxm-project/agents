# Plan: Wire the APXM↔vLLM Bidirectional Integration

## Context

APXM has **excellent infrastructure** for graph-aware vLLM integration that was never actually connected:

- `GraphAwareVllmBackend` (backend.rs) can register graphs, release graphs, and inject hints into requests
- `ApxmGraphHints`, `PinPolicy`, `CompilerHints`, `GraphMetadata`, `NodeSpec` types exist and are tested
- `LLMRequest` has `apxm_hints` and `extra_body` fields with builders
- The OpenAI backend already merges `extra_body` into HTTP bodies

**But the bridge is missing**: the runtime never constructs hints, never calls `register_graph()`, and no compiler pass computes which nodes are critical-path or share prefixes. The types exist; the wiring doesn't.

This plan connects the dots so every LLM request from an APXM graph to a vLLM backend carries full graph context.

## Global Config Integration

APXM already has a powerful routing chain that controls which backend handles each operation:

```toml
# ~/.apxm/config.toml — backend registration
[[backends]]
name = "local-gpu"
type = "local"
protocol = "vllm"                    # ← this triggers GraphAwareVllmBackend
endpoint = "http://localhost:8000"

[[backends.models]]
id = "meta-llama/Llama-4-Maverick-17B-128E"
aliases = ["llama4", "local"]

# Per-operation routing — controls which model/backend each op uses
[chat.routing.operation_routes.ask]
backend = "local-gpu"
model = "meta-llama/Llama-4-Maverick-17B-128E"

[chat.routing.operation_routes.think]
backend = "local-gpu"
model = "meta-llama/Llama-4-Maverick-17B-128E"

[chat.routing.operation_routes.reason]
backend = "amd-gateway"             # ← REASON routes to cloud, not vLLM
model = "claude-sonnet-4-5@20250929"
```

**Routing resolution priority** (from `resolver.rs`):
1. Explicit `backend` attribute on node
2. Model→backend mapping (from model aliases/routes)
3. Operation-type route (`operation_routes.ask.backend`)
4. Global `default_backend`
5. Strategy-based (first healthy)

**Design consequence for hints**: The hint injection must happen AFTER backend resolution, not before. When ASK routes to vLLM but REASON routes to cloud, only ASK requests get `extra_body.apxm`. This means:

- The compiler pass (`vllm_hints()`) annotates ALL LLM nodes — it doesn't know runtime config
- The runtime's `inject_vllm_hints()` checks whether the **resolved backend** is vLLM-aware
- Graph registration includes all LLM nodes (vLLM uses it for prefetch scheduling even for nodes it won't serve)

## The Perfect Flow

```
CONFIG                   COMPILATION              RUNTIME

~/.apxm/config.toml     vllm_hints() pass         build_context() detects vLLM
  backends:              annotates ALL LLM         sets vllm_graph_id/execution_id
    local-gpu (vllm)     nodes with:
    amd-gateway (cloud)   - critical path        → register_graph() sends
  operation_routes:       - downstream deps        GraphMetadata (all LLM nodes)
    ask → local-gpu       - reuse groups
    think → local-gpu     - est. tokens          → per-node LLM handler:
    reason → amd-gateway  - warmup/pipeline        1. resolve backend via registry
                                                    2. IF resolved == vllm-aware:
                         stores as _vllm_*             inject_vllm_hints()
                         node attributes            3. ELSE: skip hints (zero cost)

                                                 → GraphAwareVllmBackend
                                                    inject_hints() → extra_body.apxm

                                                 → release_graph() on complete
```

## Implementation Steps

### Step 1: Add vLLM attribute constants

**File**: `crates/core/apxm-core/src/constants.rs` (after line 127)

```rust
// vLLM graph-awareness hint attributes (set by vllm_hints pass)
pub const VLLM_CRITICAL_PATH: &str = "_vllm_critical_path";
pub const VLLM_DOWNSTREAM_NODES: &str = "_vllm_downstream_nodes";
pub const VLLM_REUSE_GROUP: &str = "_vllm_reuse_group";
pub const VLLM_EST_TOKENS: &str = "_vllm_est_tokens";
pub const VLLM_WARMUP: &str = "_vllm_warmup";
pub const VLLM_PIPELINE: &str = "_vllm_pipeline";
```

Add a `vllm` module at the top level of constants:

```rust
pub mod vllm {
    pub const DEFAULT_PIN_TTL_MS: u32 = 30_000;
    pub const CRITICAL_PATH_LENGTH: &str = "vllm.critical_path_length";
    pub const MAX_PARALLELISM: &str = "vllm.max_parallelism";
    pub const NODE_COUNT: &str = "vllm.node_count";
}
```

---

## COMPILER SIDE

### Step 2: Compiler pass — `vllm_hints()`

**File**: `crates/apxm-graph/src/optimize.rs`

**Signature** (follows existing pass pattern — returns `Result<&mut Self, GraphError>` for chaining):

```rust
pub fn vllm_hints(&mut self) -> Result<&mut Self, GraphError> {
    // ... implementation ...
    Ok(self)
}
```

**Critical design: reuse `compute_parallelism_metrics()`**

`parallelism_analysis()` (line 116) already runs BEFORE `vllm_hints()` and stores results in `self.metadata`. But `vllm_hints()` needs more than just max_parallelism and critical_path_length — it needs per-node critical-path membership and downstream maps. Rather than reading the stored metadata values, `vllm_hints()` should call `compute_parallelism_metrics()` internally (it's a pure function on `&ApxmGraph`, cheap) and extend it with the additional analysis.

Alternatively, refactor `compute_parallelism_metrics` to return richer data (the `longest_path` HashMap) so `vllm_hints` can consume it. This avoids duplicating the BFS. The preferred approach:

```rust
// Refactor the existing helper to expose more data
struct GraphAnalysis {
    max_parallelism: usize,
    critical_path_length: usize,
    longest_path: HashMap<u64, usize>,   // node_id → depth
    incoming: HashMap<u64, Vec<u64>>,
    outgoing: HashMap<u64, Vec<u64>>,
}

fn analyze_graph_topology(graph: &ApxmGraph) -> GraphAnalysis { ... }
```

`parallelism_analysis()` calls `analyze_graph_topology()` (same logic as current `compute_parallelism_metrics`, lines 220-283).
`vllm_hints()` also calls `analyze_graph_topology()` and uses the richer output.

**The 6 analyses `vllm_hints()` performs:**

```rust
pub fn vllm_hints(&mut self) -> Result<&mut Self, GraphError> {
    let analysis = analyze_graph_topology(self);
    let is_llm = |op: &AISOperationType| matches!(op,
        AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason
    );

    // 1. Identify critical-path nodes
    //    A node is on critical path if longest_path[node] == critical_path_length
    //    OR if it's on any longest chain from entry to exit.
    //    Simpler: node is critical if longest_path[node] + longest_remaining[node] == critical_path_length
    //    Build longest_remaining via reverse BFS from exit nodes.
    let critical_set: HashSet<u64> = compute_critical_set(&analysis);

    // 2. Build downstream LLM map (two-phase: immutable read, then mutable write)
    //    For each LLM node, find direct downstream LLM dependents.
    //    Walk outgoing edges, skip non-LLM intermediate nodes.
    let downstream_llm: HashMap<u64, Vec<u64>> = ... ;  // node_id → [downstream LLM ids]

    // 3. Group by system_prompt — reuse group detection
    //    Nodes sharing identical system_prompt get same reuse_group hash.
    //    First node in each group gets warmup: true.
    let mut prompt_groups: HashMap<String, (String, u64)> = HashMap::new();
    //    key: system_prompt text, value: (group_id, first_node_id)
    //    group_id = format!("sp_{:x}", hash(system_prompt)[..8])

    // 4. Estimate tokens per node
    //    est_tokens = (template_str.len() + system_prompt.len()) / 4

    // 5. Detect pipeline chains
    //    A node is pipeline=true if: it's an LLM node, has exactly 1 LLM predecessor,
    //    and that predecessor has exactly 1 LLM successor (linear chain).

    // 6. Annotate each LLM node (two-phase to satisfy borrow checker)
    struct VllmAnnotation {
        node_idx: usize,
        critical_path: bool,
        downstream_nodes: Vec<u64>,
        reuse_group: Option<String>,
        est_tokens: u64,
        warmup: bool,
        pipeline: bool,
    }
    let annotations: Vec<VllmAnnotation> = ... ;  // collected immutably

    // Apply annotations (mutable phase)
    for ann in annotations {
        let node = &mut self.nodes[ann.node_idx];
        node.attributes.insert(attrs::VLLM_CRITICAL_PATH.to_string(), Value::Bool(ann.critical_path));
        node.attributes.insert(attrs::VLLM_DOWNSTREAM_NODES.to_string(),
            Value::Array(ann.downstream_nodes.iter().map(|id| Value::Number((*id as i64).into())).collect()));
        if let Some(group) = ann.reuse_group {
            node.attributes.insert(attrs::VLLM_REUSE_GROUP.to_string(), Value::String(group));
        }
        node.attributes.insert(attrs::VLLM_EST_TOKENS.to_string(), Value::Number((ann.est_tokens as i64).into()));
        node.attributes.insert(attrs::VLLM_WARMUP.to_string(), Value::Bool(ann.warmup));
        node.attributes.insert(attrs::VLLM_PIPELINE.to_string(), Value::Bool(ann.pipeline));
    }

    // 7. Store graph-level metadata
    let llm_count = self.nodes.iter().filter(|n| is_llm(&n.op)).count();
    self.metadata.insert(vllm::CRITICAL_PATH_LENGTH.to_string(),
        Value::Number((analysis.critical_path_length as i64).into()));
    self.metadata.insert(vllm::MAX_PARALLELISM.to_string(),
        Value::Number((analysis.max_parallelism as i64).into()));
    self.metadata.insert(vllm::NODE_COUNT.to_string(),
        Value::Number((llm_count as i64).into()));

    Ok(self)
}
```

**Key invariant**: The pass is **idempotent** and **safe** even when no vLLM backend exists. It only writes `_vllm_*` prefixed attributes that the runtime ignores unless `ctx.vllm_graph_id.is_some()`.

### Step 3: Wire the pass into the pipeline

**File**: `crates/compiler/apxm-compiler/src/api/pipeline.rs` (after line 109)

```rust
run_pass!(graph, vllm_hints, "vLLM graph hints");
```

Pipeline order becomes: `constant_folding` → `prompt_caching` → `memoization_hints` → `parallelism_analysis` → `vllm_hints`

### Step 4: Attribute flow through DAG lowering — what happens automatically

**File**: `crates/apxm-graph/src/lower_dag.rs` line 17

```rust
node.attributes = graph_node.attributes.clone();  // ALL _vllm_* attrs survive
```

**Per-node attributes**: Flow automatically. No changes needed to `lower_dag.rs` for node-level hints.

**Graph-level metadata**: Currently `lower_dag.rs:94-105` only reads `is_entry` from `graph.metadata` and does NOT propagate other metadata keys to `DagMetadata`. We need to propagate `vllm.*` keys.

**File**: `crates/core/apxm-core/src/types/execution/dag.rs` — Add optional fields to `DagMetadata`:

```rust
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DagMetadata {
    pub name: Option<String>,
    #[serde(default)]
    pub is_entry: bool,
    #[serde(default)]
    pub parameters: Vec<FlowParameter>,
    // NEW: vLLM graph-level metadata (set by vllm_hints pass, used by runtime for registration)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vllm_critical_path_length: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vllm_max_parallelism: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vllm_node_count: Option<u32>,
}
```

**File**: `crates/apxm-graph/src/lower_dag.rs` — Propagate in the `DagMetadata` construction (line 94):

```rust
metadata: DagMetadata {
    name: Some(graph.name.clone()),
    is_entry,
    parameters: graph.parameters.iter().map(|p| FlowParameter {
        name: p.name.clone(),
        type_name: p.type_name.clone(),
    }).collect(),
    // Propagate vLLM metadata from graph.metadata if present
    vllm_critical_path_length: graph.metadata.get(vllm::CRITICAL_PATH_LENGTH)
        .and_then(|v| v.as_u64()).map(|v| v as u32),
    vllm_max_parallelism: graph.metadata.get(vllm::MAX_PARALLELISM)
        .and_then(|v| v.as_u64()).map(|v| v as u32),
    vllm_node_count: graph.metadata.get(vllm::NODE_COUNT)
        .and_then(|v| v.as_u64()).map(|v| v as u32),
},
```

---

## RUNTIME SIDE

### Step 5: ExecutionContext — carry vLLM graph/execution IDs

**File**: `crates/runtime/apxm-runtime/src/executor/context.rs` (add after line 104)

```rust
/// vLLM graph ID for this execution (set when a vLLM backend is registered).
/// When Some, the LLM handler injects ApxmGraphHints into every LLM request.
pub vllm_graph_id: Option<String>,
/// vLLM execution ID — unique per run, paired with graph_id.
pub vllm_execution_id: Option<String>,
```

Initialize to `None` in `new()` (line 115). These are set by `build_context()` when a vLLM backend is detected.

### Step 6: Backend detection API

**File**: `crates/runtime/apxm-backends/src/llm/registry/mod.rs`

Add method:
```rust
/// Find the first graph-aware vLLM backend. Returns (name, base_url).
pub fn find_graph_aware_backend(&self) -> Option<(String, String)> {
    for entry in self.backends.iter() {
        let meta = entry.value().metadata();
        if meta.get("backend_type").and_then(|v| v.as_str()) == Some("vllm-graph-aware") {
            let base_url = meta.get("base_url")
                .and_then(|v| v.as_str())
                .unwrap_or("http://localhost:8000")
                .to_string();
            return Some((entry.key().clone(), base_url));
        }
    }
    None
}
```

**File**: `crates/runtime/apxm-backends/src/llm/backends/vllm/backend.rs` line 240

Add `base_url` to `metadata()` return value so the runtime can reach the vLLM server for lifecycle calls without downcasting:

```rust
fn metadata(&self) -> serde_json::Value {
    let mut meta = self.inner.metadata();
    if let serde_json::Value::Object(ref mut map) = meta {
        map.insert("backend_type".to_string(), "vllm-graph-aware".into());
        map.insert("apxm_extensions".to_string(), true.into());
        map.insert("base_url".to_string(), self.base_url.clone().into());  // NEW
    }
    meta
}
```

### Step 7: vLLM graph lifecycle manager

**New file**: `crates/runtime/apxm-runtime/src/vllm_lifecycle.rs`

```rust
use apxm_backends::llm::backends::vllm::graph_meta::{GraphMetadata, NodeSpec};
use apxm_core::constants::{graph::attrs, vllm};
use apxm_core::types::execution::ExecutionDag;
use apxm_core::types::AISOperationType;

pub struct VllmGraphLifecycle {
    graph_id: String,
    execution_id: String,
    base_url: String,
    client: reqwest::Client,
}

impl VllmGraphLifecycle {
    pub fn new(graph_id: String, execution_id: String, base_url: String) -> Self {
        Self { graph_id, execution_id, base_url, client: reqwest::Client::new() }
    }

    /// Register the graph with vLLM. Best-effort — logs warnings on failure.
    pub async fn register(&self, dag: &ExecutionDag) {
        let is_llm = |op: &AISOperationType| matches!(op,
            AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason);

        let node_specs: Vec<NodeSpec> = dag.nodes.iter()
            .filter(|n| is_llm(&n.op_type))
            .map(|n| NodeSpec {
                node_id: n.id as u32,
                node_name: n.metadata.name.clone(),
                estimated_prompt_tokens: n.attributes.get(attrs::VLLM_EST_TOKENS)
                    .and_then(|v| v.as_u64()).map(|v| v as u32),
                downstream_nodes: n.attributes.get(attrs::VLLM_DOWNSTREAM_NODES)
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|i| i as u32)).collect())
                    .unwrap_or_default(),
                priority_class: if n.attributes.get(attrs::VLLM_CRITICAL_PATH)
                    .and_then(|v| v.as_boolean()).unwrap_or(false) {
                    Some("critical_path".to_string())
                } else { None },
                reuse_group: n.attributes.get(attrs::VLLM_REUSE_GROUP)
                    .and_then(|v| v.as_string()).map(|s| s.to_string()),
                is_critical_path: n.attributes.get(attrs::VLLM_CRITICAL_PATH)
                    .and_then(|v| v.as_boolean()).unwrap_or(false),
            })
            .collect();

        let metadata = GraphMetadata::new(&self.graph_id, &self.execution_id)
            .with_pin_ttl(vllm::DEFAULT_PIN_TTL_MS)
            .with_critical_path_length(
                dag.metadata.vllm_critical_path_length.unwrap_or(0)
            )
            .with_nodes(node_specs);

        let url = format!("{}/v1/apxm/graphs/register", self.base_url);
        match self.client.post(&url).json(&metadata).send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!(graph_id = %self.graph_id, "vLLM graph registered");
            }
            Ok(resp) => {
                tracing::warn!(graph_id = %self.graph_id, status = %resp.status(),
                    "vLLM graph registration returned non-success (continuing)");
            }
            Err(e) => {
                tracing::warn!(graph_id = %self.graph_id, error = %e,
                    "vLLM graph registration failed (continuing without hints)");
            }
        }
    }

    /// Release the graph on vLLM. Best-effort.
    pub async fn release(&self) {
        let url = format!("{}/v1/apxm/graphs/{}", self.base_url, self.graph_id);
        if let Err(e) = self.client.delete(&url).send().await {
            tracing::warn!(graph_id = %self.graph_id, error = %e,
                "vLLM graph release failed");
        }
    }
}
```

**Key**: Uses its own `reqwest::Client` + `base_url` from `find_graph_aware_backend()` return value — no downcasting of `Arc<dyn LLMBackend>` needed.

### Step 8: Wire lifecycle into Runtime::execute methods

**File**: `crates/runtime/apxm-runtime/src/runtime.rs`

**In `build_context()` (line 160)** — detect vLLM backend and set context fields:

```rust
fn build_context(&self, ...) -> ExecutionContext {
    let mut ctx = ExecutionContext::new(...);
    // ... existing setup (lines 172-189) ...

    // NEW: detect vLLM backend
    if let Some((_backend_name, _base_url)) = self.llm_registry.find_graph_aware_backend() {
        // graph_id will be set per-execution in execute()/execute_artifact_*
        // Just flag that a vLLM backend exists so the LLM handler knows to inject hints
    }

    ctx
}
```

Actually, `build_context` is called once but `graph_id` depends on the specific DAG. So context vLLM fields are set **in the execute methods** instead:

**In `execute()` (line 264)** — the "leaf" execution method for simple DAGs:

```rust
pub async fn execute(&self, dag: ExecutionDag) -> Result<RuntimeExecutionResult, RuntimeError> {
    let mut context = self.build_context(None, None, None);

    // NEW: vLLM lifecycle — detect backend and register graph
    let vllm_lifecycle = if let Some((_name, base_url)) = self.llm_registry.find_graph_aware_backend() {
        let graph_name = dag.metadata.name.as_deref().unwrap_or("unnamed");
        let graph_id = format!("{}-{}", graph_name, &context.execution_id[..8]);
        let execution_id = context.execution_id.clone();
        context.vllm_graph_id = Some(graph_id.clone());
        context.vllm_execution_id = Some(execution_id.clone());
        let lifecycle = VllmGraphLifecycle::new(graph_id, execution_id, base_url);
        lifecycle.register(&dag).await;
        Some(lifecycle)
    } else {
        None
    };

    let executor = Arc::new(ExecutorEngine::new(context.clone()));
    let result = self.scheduler.execute(dag, executor, context, vec![]).await;

    // NEW: release graph on completion (success or error)
    if let Some(lifecycle) = vllm_lifecycle {
        lifecycle.release().await;
    }

    let (results, stats, scheduler_metrics, all_outputs, node_output_map) = result?;
    // ... existing result construction ...
}
```

**In `execute_artifact_with_session_and_emitter()` (line 354)** — same pattern:

```rust
pub async fn execute_artifact_with_session_and_emitter(&self, ...) -> Result<...> {
    // ... existing setup (lines 362-378) ...
    let context = self.build_context(session_id, event_emitter, session_dir);

    // NEW: vLLM lifecycle
    let vllm_lifecycle = if let Some((_name, base_url)) = self.llm_registry.find_graph_aware_backend() {
        let graph_name = entry_dag.metadata.name.as_deref().unwrap_or("unnamed");
        let graph_id = format!("{}-{}", graph_name, &context.execution_id[..8]);
        let execution_id = context.execution_id.clone();
        // Set on context (need &mut, so move context setup before Arc wrapping)
        context.vllm_graph_id = Some(graph_id.clone());
        context.vllm_execution_id = Some(execution_id.clone());
        let lifecycle = VllmGraphLifecycle::new(graph_id, execution_id, base_url);
        lifecycle.register(&entry_dag).await;
        Some(lifecycle)
    } else {
        None
    };

    let executor = Arc::new(ExecutorEngine::new(context.clone()));
    let result = self.scheduler.execute(entry_dag, executor, context, arg_values).await;

    if let Some(lifecycle) = vllm_lifecycle {
        lifecycle.release().await;
    }

    let (results, stats, scheduler_metrics, all_outputs, node_output_map) = result?;
    // ... existing ...
}
```

**Coverage**: `execute_artifact_auto()` delegates to `execute()`, `execute_artifact_with_args()` delegates to `execute_artifact_with_session_and_emitter()` — so all 4 execute methods are covered by just modifying these 2 "leaf" methods.

### Step 9: Per-request hint injection in LLM handler

**File**: `crates/runtime/apxm-runtime/src/executor/handlers/llm.rs`

**Injection point**: After line 559 (after all existing config: system_prompt, model, tools), before the retry loop at line 561.

```rust
// Line 559: end of existing tools configuration

// NEW: inject vLLM graph hints if a vLLM backend is active
if ctx.vllm_graph_id.is_some() {
    request = inject_vllm_hints(ctx, node, request);
}

// Line 561: existing retry loop starts
```

**New function** `inject_vllm_hints()`:

```rust
fn inject_vllm_hints(ctx: &ExecutionContext, node: &Node, request: LLMRequest) -> LLMRequest {
    use apxm_backends::llm::backends::vllm::graph_meta::{ApxmGraphHints, PinPolicy, CompilerHints};

    let graph_id = match &ctx.vllm_graph_id {
        Some(id) => id.clone(),
        None => return request,  // defensive — should not happen given the guard
    };
    let execution_id = ctx.vllm_execution_id.clone().unwrap_or_default();

    // Read compiler annotations from node attributes
    let is_critical = node.attributes.get(attrs::VLLM_CRITICAL_PATH)
        .and_then(|v| v.as_boolean()).unwrap_or(false);
    let downstream: Vec<u32> = node.attributes.get(attrs::VLLM_DOWNSTREAM_NODES)
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|i| i as u32)).collect())
        .unwrap_or_default();
    let reuse_group = node.attributes.get(attrs::VLLM_REUSE_GROUP)
        .and_then(|v| v.as_string()).map(|s| s.to_string());
    let est_tokens = node.attributes.get(attrs::VLLM_EST_TOKENS)
        .and_then(|v| v.as_u64()).map(|v| v as u32);
    let warmup = node.attributes.get(attrs::VLLM_WARMUP)
        .and_then(|v| v.as_boolean()).unwrap_or(false);
    let pipeline = node.attributes.get(attrs::VLLM_PIPELINE)
        .and_then(|v| v.as_boolean()).unwrap_or(false);

    let hints = ApxmGraphHints {
        schema_version: 1,
        graph_id: Some(graph_id),
        execution_id: Some(execution_id),
        node_id: Some(node.id as u32),
        node_name: node.metadata.name.clone(),
        priority_class: if is_critical { Some("critical_path".to_string()) } else { None },
        downstream_nodes: downstream,
        reuse_group,
        pin_policy: if is_critical && !downstream.is_empty() {
            PinPolicy::prefix(vllm::DEFAULT_PIN_TTL_MS)
        } else {
            PinPolicy::none()
        },
        compiler_hints: CompilerHints {
            shared_prefix_est_tokens: est_tokens,
            warmup_candidate: if warmup { Some(true) } else { None },
            pipeline_candidate: if pipeline { Some(true) } else { None },
        },
    };

    request.with_apxm_hints(hints)
}
```

**Why this is safe**: `LLMRequest.apxm_hints` is `Option<ApxmGraphHints>`. Only `GraphAwareVllmBackend.generate()` (backend.rs:206) calls `inject_hints()` which merges it into `extra_body.apxm`. OpenAI/Anthropic/Google backends never read `apxm_hints` — the field is silently ignored. So attaching hints unconditionally (when any vLLM backend exists in the registry) is safe and zero-overhead for non-vLLM routes.

**How routing interacts**: The node's `model` attribute determines routing via `LLMRegistry::prepare_request()` (registry/mod.rs:251):
- No model → `operation_models[Ask]` → resolved via alias → routes to registered backend → if vLLM → hints used
- `model="smart"` (alias for cloud model) → routes to cloud backend → hints ignored
- `backend="local-gpu"` → explicitly routes to vLLM → hints used

### Step 10: Ensure hints survive tool loop

**File**: `crates/runtime/apxm-runtime/src/executor/handlers/llm.rs` lines 932-946

**Current code** (tool loop request rebuild at line 933):
```rust
current_request = LLMRequest::new(continuation_prompt)
    .with_system_prompt(current_request.system_prompt.clone().unwrap_or_default())
    .with_temperature(current_request.temperature);

// Keeps tools, tool_choice, model — but NOT apxm_hints!
if let Some(tools) = &initial_request.tools { ... }
if let Some(choice) = &initial_request.tool_choice { ... }
if let Some(model) = &initial_request.model { ... }
```

**Fix**: After line 946, add:
```rust
if let Some(ref hints) = initial_request.apxm_hints {
    current_request = current_request.with_apxm_hints(hints.clone());
}
```

This ensures every tool-loop iteration carries the same vLLM hints. The `node_id` stays the same (it's the same logical node), and `downstream_nodes` is still valid (the node hasn't changed, only the prompt grew with tool results).

---

## Data Flow Summary

```
                    COMPILER                              RUNTIME
                    --------                              -------

optimize.rs                                     context.rs
  vllm_hints() pass                               ExecutionContext {
    ↓ writes per-node:                               vllm_graph_id: Some("..."),
    node.attributes[_vllm_critical_path]             vllm_execution_id: Some("..."),
    node.attributes[_vllm_downstream_nodes]        }
    node.attributes[_vllm_reuse_group]
    node.attributes[_vllm_est_tokens]            runtime.rs
    node.attributes[_vllm_warmup]                  execute() {
    node.attributes[_vllm_pipeline]                  detect vllm backend → set ctx fields
    ↓ writes graph-level:                            VllmGraphLifecycle::register(dag)
    graph.metadata[vllm.critical_path_length]        ↓
    graph.metadata[vllm.max_parallelism]             scheduler.execute(dag, ...)
    graph.metadata[vllm.node_count]                  ↓
                                                     VllmGraphLifecycle::release()
lower_dag.rs                                       }
  node.attributes = graph_node.attributes.clone()
  ↑ ALL _vllm_* attrs survive into ExecutionDag  llm.rs
  dag.metadata.vllm_* ← graph.metadata[vllm.*]    execute() {
                                                     build LLMRequest (line 510)
pipeline.rs                                          add system_prompt, model, tools
  constant_folding                                   ↓
  prompt_caching                                     inject_vllm_hints(ctx, node, request)
  memoization_hints                                  ↑ reads node.attributes[_vllm_*]
  parallelism_analysis                               ↑ reads ctx.vllm_graph_id/execution_id
  vllm_hints          ← NEW PASS                    ↑ constructs ApxmGraphHints
                                                     ↓
                                                     execute_llm_request() → registry → backend
                                                     ↓
                                                     GraphAwareVllmBackend.inject_hints()
                                                     ↑ merges apxm_hints → extra_body.apxm
                                                     ↓
                                                     POST /v1/chat/completions + apxm hints
                                                   }

                                                   tool loop (lines 813-947):
                                                     rebuild request for next iteration
                                                     ↓ carries initial_request.apxm_hints
                                                     (Step 10 fix)
```

## Files Changed (Summary)

| File | Change |
|------|--------|
| `crates/core/apxm-core/src/constants.rs` | Add `_vllm_*` attrs (6 consts) + `vllm` module (3 consts + 1 default) |
| `crates/core/apxm-core/src/types/execution/dag.rs` | Add 3 optional vLLM fields to `DagMetadata` |
| `crates/apxm-graph/src/optimize.rs` | Refactor `compute_parallelism_metrics` → `analyze_graph_topology`; new `vllm_hints()` pass (~120 lines) |
| `crates/apxm-graph/src/lower_dag.rs` | Propagate `vllm_*` metadata to `DagMetadata` (3 lines) |
| `crates/compiler/apxm-compiler/src/api/pipeline.rs` | Add `run_pass!(graph, vllm_hints, "vLLM graph hints")` (1 line) |
| `crates/runtime/apxm-backends/src/llm/registry/mod.rs` | Add `find_graph_aware_backend()` method (~15 lines) |
| `crates/runtime/apxm-backends/src/llm/backends/vllm/backend.rs` | Add `base_url` to `metadata()` (1 line) |
| `crates/runtime/apxm-runtime/src/vllm_lifecycle.rs` | **New** — register/release lifecycle (~80 lines) |
| `crates/runtime/apxm-runtime/src/runtime.rs` | Wire lifecycle in `execute()` + `execute_artifact_with_session_and_emitter()` (~30 lines each) |
| `crates/runtime/apxm-runtime/src/executor/context.rs` | Add `vllm_graph_id`, `vllm_execution_id` fields (4 lines) |
| `crates/runtime/apxm-runtime/src/executor/handlers/llm.rs` | Add `inject_vllm_hints()` (~50 lines) + tool loop fix (3 lines) |

## Verification

1. **Unit test the compiler pass**: Create `ApxmGraph` with fan-out pattern (1 ASK → 3 ASK → 1 THINK), run `vllm_hints()`, assert `_vllm_critical_path`, `_vllm_downstream_nodes`, `_vllm_reuse_group`, `_vllm_warmup` are set correctly on each node. Test in `optimize.rs` alongside existing tests (line 285+).
2. **Unit test hint injection**: Create a `Node` with `_vllm_*` attrs, mock `ExecutionContext` with `vllm_graph_id` set, call `inject_vllm_hints()`, assert resulting `LLMRequest.apxm_hints` has correct `graph_id`, `node_id`, `priority_class`, `pin_policy`, `compiler_hints`.
3. **Unit test tool loop hint survival**: Build request with `apxm_hints`, simulate tool loop rebuild, assert hints carried forward.
4. **Unit test lifecycle manager**: Test `VllmGraphLifecycle::register()` builds correct `GraphMetadata` from DAG with `_vllm_*` attrs. Mock HTTP or test serialization.
5. **Integration**: `dekk apxm compile examples/python/patterns/plan-fan-out/plan_fan_out.py -O2`, decompile artifact, verify `_vllm_*` attributes on ASK/THINK nodes.
6. **No vLLM backend**: When no vLLM backend is registered, `ctx.vllm_graph_id` is None → `inject_vllm_hints()` never runs → `LLMRequest.apxm_hints` is None → zero overhead.

---

## Appendix: Complete Worked Example — `plan_fan_out` Through the Entire Pipeline

This traces the `plan_fan_out.py` graph through every stage of the APXM→vLLM pipeline, showing exactly what the data looks like at each point.

### Stage 0: Python Source (User Writes This)

```python
# examples/python/patterns/plan-fan-out/plan_fan_out.py
from apxm.graph import compile, GraphRecorder

@compile()
def plan_then_parallelize(g: GraphRecorder):
    plan_steps = g.ask("plan_steps", "Create a 3-part outline for a blog post...")

    section_concepts = g.ask("section_concepts",
        "Based on this plan:\n{plan_steps}\n\nWrite section 1: Core async concepts...")
    section_tokio = g.ask("section_tokio",
        "Based on this plan:\n{plan_steps}\n\nWrite section 2: Tokio runtime internals...")
    section_pitfalls = g.ask("section_pitfalls",
        "Based on this plan:\n{plan_steps}\n\nWrite section 3: Common pitfalls...")

    assemble = g.think("assemble",
        "SECTION 1:\n{section_concepts}\nSECTION 2:\n{section_tokio}\n"
        "SECTION 3:\n{section_pitfalls}\n\nAssemble into a polished blog post...")

    output = g.print("=== ASSEMBLED BLOG POST ===\n{assemble}")
    g.done(output)
```

**DAG topology:**
```
plan_steps (ASK)
  ├─Data→ section_concepts (ASK)
  ├─Data→ section_tokio    (ASK)
  └─Data→ section_pitfalls (ASK)
                 ↘         ↓         ↙
              assemble (THINK)
                   ↓
              print (PRINT)
                   ↓
              return (RETURN)
```

### Stage 1: MLIR `.air` (After `@compile()` → `to_air()`)

The `@compile()` decorator captures the graph as a Python `ApxmGraph`, then `to_air()` emits valid MLIR:

```mlir
module {
  func.func @plan_then_parallelize() -> !ais.token attributes {ais.entry} {
    %plan_steps = ais.ask "Create a 3-part outline for a blog post about Rust async programming..." : !ais.token
    %section_concepts = ais.ask "Based on this plan:\n{0}\n\nWrite section 1: Core async concepts..." [%plan_steps : !ais.token] : !ais.token
    %section_tokio = ais.ask "Based on this plan:\n{0}\n\nWrite section 2: Tokio runtime internals..." [%plan_steps : !ais.token] : !ais.token
    %section_pitfalls = ais.ask "Based on this plan:\n{0}\n\nWrite section 3: Common pitfalls..." [%plan_steps : !ais.token] : !ais.token
    %assemble = ais.think "SECTION 1:\n{0}\nSECTION 2:\n{1}\nSECTION 3:\n{2}\n\nAssemble into a polished blog post..." [%section_concepts, %section_tokio, %section_pitfalls : !ais.token, !ais.token, !ais.token] : !ais.token
    ais.print "=== ASSEMBLED BLOG POST ===\n{0}" [%assemble : !ais.token]
    func.return %assemble : !ais.token
  }
}
```

**Key observations:**
- SSA form — each `%name` assigned once, data flow is explicit
- `%section_concepts`, `%section_tokio`, `%section_pitfalls` all take `[%plan_steps]` as input → **fan-out**
- `%assemble` takes all three as inputs → **fan-in**
- `Module::parse()` in Rust MLIR reads this directly — no custom parser needed
- The Python `ApxmGraph` intermediate representation (nodes + edges + attributes) is what the Rust compiler's `Pipeline::compile_graph()` receives

### Stage 2: After `vllm_hints()` Compiler Pass (O2)

The `vllm_hints()` pass runs on the Rust `ApxmGraph` struct (parsed from MLIR) inside `Pipeline::run_graph_passes()`, *before* MLIR lowering to artifact. It analyzes the graph topology and annotates each LLM node's `attributes` HashMap.

**Critical path analysis:**
```
Longest path: plan_steps → section_concepts → assemble (3 LLM hops)
                           (section_tokio / section_pitfalls are equivalent alternatives)
Max parallelism: 3 (the three section nodes fire simultaneously)
```

**Reuse group detection:**
- All 4 ASK nodes share the runtime's default system prompt → `reuse_group: "sp_ask_default"`
- Node 5 (THINK) uses a different default system prompt → separate group or none
- Node 1 is the first in its group → `warmup: true` (tells vLLM to eagerly prefill its prefix)

**Per-node attributes inserted by the pass** (shown as Rust `Value` entries):

```rust
// Node 1: plan_steps (ASK) — root of the fan-out
node.attributes = {
    "template_str":           Value::String("Create a 3-part outline..."),
    // ↓ NEW: vLLM hints ↓
    "_vllm_critical_path":    Value::Bool(true),
    "_vllm_downstream_nodes": Value::Array(vec![Value::Number(2), Value::Number(3), Value::Number(4)]),
    "_vllm_reuse_group":      Value::String("sp_ask_default"),
    "_vllm_est_tokens":       Value::Number(42),
    "_vllm_warmup":           Value::Bool(true),   // first in reuse group
    "_vllm_pipeline":         Value::Bool(false),
}

// Nodes 2-4: section_concepts / section_tokio / section_pitfalls (ASK) — parallel fan
node.attributes = {
    "template_str":           Value::String("Based on this plan:\n{0}\n\nWrite section ..."),
    "_vllm_critical_path":    Value::Bool(true),    // all paths are equal length
    "_vllm_downstream_nodes": Value::Array(vec![Value::Number(5)]),
    "_vllm_reuse_group":      Value::String("sp_ask_default"),
    "_vllm_est_tokens":       Value::Number(85),    // (template + system_prompt) / 4
    "_vllm_warmup":           Value::Bool(false),
    "_vllm_pipeline":         Value::Bool(false),
}

// Node 5: assemble (THINK) — fan-in, last LLM node
node.attributes = {
    "template_str":           Value::String("SECTION 1:\n{0}\nSECTION 2:\n{1}..."),
    "_vllm_critical_path":    Value::Bool(true),
    "_vllm_downstream_nodes": Value::Array(vec![]),  // no LLM nodes after this
    "_vllm_est_tokens":       Value::Number(120),
    "_vllm_warmup":           Value::Bool(false),
    "_vllm_pipeline":         Value::Bool(false),
    // no reuse_group — THINK uses a different system prompt
}
```

**Graph-level metadata** stored in `graph.metadata: HashMap<String, Value>`:
```rust
graph.metadata = {
    "vllm.critical_path_length": Value::Number(3),
    "vllm.max_parallelism":      Value::Number(3),
    "vllm.node_count":           Value::Number(5),
}
```

### Stage 3: DAG Lowering (`lower_to_execution_dag`)

The `_vllm_*` attributes do NOT flow through MLIR text — they travel via the Rust `ApxmGraph` → `lower_to_execution_dag()` → `ExecutionDag` path. At `lower_dag.rs:17`:

```rust
node.attributes = graph_node.attributes.clone();  // ALL attributes preserved
```

The compilation pipeline is:
1. Python `ApxmGraph` → `to_air()` → MLIR text (only `template_str` emitted for ASK/THINK)
2. `Module::parse()` → MLIR Module → MLIR passes (CSE, canonicalize, etc.)
3. **In parallel**: `ApxmGraph` → `run_graph_passes()` (includes `vllm_hints()`) → `lower_to_execution_dag()` → `ExecutionDag`
4. The `ExecutionDag` carries all `_vllm_*` attributes into the artifact binary via `WireNode.attributes`

So the `ExecutionDag` for this graph has 8 nodes, each carrying the full attribute map including vLLM hints. The three section nodes (IDs 2, 3, 4) have their `input_tokens` set to the token from node 1, and node 5's `input_tokens` has tokens from all three sections — this is how the scheduler knows what can run in parallel.

### Stage 4: Runtime — Graph Registration (Before First Node Fires)

When `Runtime::execute()` detects a vLLM backend, it creates `VllmGraphLifecycle` and calls `register()`.

**HTTP Request:**
```
POST http://localhost:8000/v1/apxm/graphs/register
Content-Type: application/json

{
  "graph_id": "plan_then_parallelize-a1b2c3d4",
  "execution_id": "exec-1743984000000-0",
  "critical_path_length": 3,
  "node_count": 5,
  "max_parallelism": 3,
  "default_pin_ttl_ms": 30000,
  "nodes": [
    {
      "node_id": 1,
      "node_name": "plan_steps",
      "estimated_prompt_tokens": 42,
      "downstream_nodes": [2, 3, 4],
      "priority_class": "critical_path",
      "reuse_group": "sp_ask_default",
      "is_critical_path": true
    },
    {
      "node_id": 2,
      "node_name": "section_concepts",
      "estimated_prompt_tokens": 85,
      "downstream_nodes": [5],
      "priority_class": "critical_path",
      "reuse_group": "sp_ask_default",
      "is_critical_path": true
    },
    {
      "node_id": 3,
      "node_name": "section_tokio",
      "estimated_prompt_tokens": 90,
      "downstream_nodes": [5],
      "priority_class": "critical_path",
      "reuse_group": "sp_ask_default",
      "is_critical_path": true
    },
    {
      "node_id": 4,
      "node_name": "section_pitfalls",
      "estimated_prompt_tokens": 88,
      "downstream_nodes": [5],
      "priority_class": "critical_path",
      "reuse_group": "sp_ask_default",
      "is_critical_path": true
    },
    {
      "node_id": 5,
      "node_name": "assemble",
      "estimated_prompt_tokens": 120,
      "downstream_nodes": [],
      "is_critical_path": true
    }
  ]
}
```

**vLLM response:**
```json
{
  "object": "apxm.graph.registration",
  "graph_id": "plan_then_parallelize-a1b2c3d4",
  "execution_id": "exec-1743984000000-0",
  "registered_nodes": 5
}
```

**What vLLM now knows:**
- 5 LLM requests are incoming, max 3 in parallel
- Nodes 2/3/4 share the same system prompt (`sp_ask_default`) → pre-pin that KV prefix
- Node 1 is `warmup: true` → can eagerly prefill after registration
- After nodes 2/3/4 complete, expect exactly one more (node 5)

### Stage 5: Runtime — Per-Request Hint Injection (Each LLM Call)

The dataflow scheduler fires `plan_steps` first (no inputs needed).

**`inject_vllm_hints()` reads node attributes and builds:**

```rust
// In llm.rs execute(), after building the LLMRequest:
let hints = ApxmGraphHints {
    schema_version: 1,
    graph_id: Some("plan_then_parallelize-a1b2c3d4"),
    execution_id: Some("exec-1743984000000-0"),
    node_id: Some(1),
    node_name: Some("plan_steps"),
    priority_class: Some("critical_path"),
    downstream_nodes: vec![2, 3, 4],
    reuse_group: Some("sp_ask_default"),
    pin_policy: PinPolicy::prefix(30_000),  // critical_path + has reuse_group
    compiler_hints: CompilerHints {
        shared_prefix_est_tokens: Some(42),
        warmup_candidate: Some(true),
        pipeline_candidate: None,
    },
};
request = request.with_apxm_hints(hints);
```

### Stage 6: Wire — HTTP Request to vLLM (Node 1: plan_steps)

`GraphAwareVllmBackend::generate()` calls `inject_hints()` which merges into `extra_body`:

```
POST http://localhost:8000/v1/chat/completions
Content-Type: application/json

{
  "model": "meta-llama/Llama-4-Maverick-17B-128E",
  "messages": [
    {"role": "system", "content": "You are a helpful AI assistant. Answer concisely."},
    {"role": "user", "content": "Create a 3-part outline for a blog post about Rust async..."}
  ],
  "temperature": 0.7,
  "apxm": {
    "schema_version": 1,
    "graph_id": "plan_then_parallelize-a1b2c3d4",
    "execution_id": "exec-1743984000000-0",
    "node_id": 1,
    "node_name": "plan_steps",
    "priority_class": "critical_path",
    "downstream_nodes": [2, 3, 4],
    "reuse_group": "sp_ask_default",
    "pin_policy": {"mode": "prefix", "ttl_ms": 30000},
    "compiler_hints": {
      "shared_prefix_est_tokens": 42,
      "warmup_candidate": true
    }
  }
}
```

**What vLLM does with this:**
1. Sees `priority_class: "critical_path"` → schedules at high priority
2. Sees `reuse_group: "sp_ask_default"` → after completing, **pins** the system prompt KV blocks
3. Sees `downstream_nodes: [2, 3, 4]` → knows 3 requests are coming that will reuse this prefix
4. Sees `warmup_candidate: true` → confirms this is the first in its cohort

### Stage 7: Wire — Parallel Fan-Out (Nodes 2, 3, 4 Fire Simultaneously)

After `plan_steps` completes, the scheduler marks all three section nodes as ready (all have their single input token fulfilled). They fire in parallel.

**Node 2 (section_concepts):**
```json
{
  "model": "meta-llama/Llama-4-Maverick-17B-128E",
  "messages": [
    {"role": "system", "content": "You are a helpful AI assistant. Answer concisely."},
    {"role": "user", "content": "Based on this plan:\nPART 1: ... PART 2: ... PART 3: ...\n\nWrite section 1: Core async concepts..."}
  ],
  "apxm": {
    "schema_version": 1,
    "graph_id": "plan_then_parallelize-a1b2c3d4",
    "execution_id": "exec-1743984000000-0",
    "node_id": 2,
    "node_name": "section_concepts",
    "priority_class": "critical_path",
    "downstream_nodes": [5],
    "reuse_group": "sp_ask_default",
    "pin_policy": {"mode": "prefix", "ttl_ms": 30000},
    "compiler_hints": {"shared_prefix_est_tokens": 85}
  }
}
```

**Node 3 (section_tokio) — same structure, different node_id/name/est_tokens:**
```json
{ "apxm": { "node_id": 3, "node_name": "section_tokio", ... } }
```

**Node 4 (section_pitfalls) — same:**
```json
{ "apxm": { "node_id": 4, "node_name": "section_pitfalls", ... } }
```

**What vLLM does:**
1. All three arrive with `reuse_group: "sp_ask_default"` → vLLM recognizes the shared system prompt
2. The system prompt KV blocks were **already pinned** from node 1 → **prefix cache HIT** for all 3
3. vLLM only needs to compute KV for the unique user message suffix of each
4. All three have `downstream_nodes: [5]` → vLLM knows one more request is coming
5. Net effect: **3x less KV computation** for the system prompt portion

### Stage 8: Wire — Assembly Node (Node 5: assemble)

After all three sections complete, the scheduler fires `assemble` (THINK mode):

```json
{
  "model": "meta-llama/Llama-4-Maverick-17B-128E",
  "messages": [
    {"role": "system", "content": "You are a helpful AI assistant..."},
    {"role": "user", "content": "SECTION 1:\n[section_concepts output]\nSECTION 2:\n[section_tokio output]\nSECTION 3:\n[section_pitfalls output]\n\nAssemble into a polished blog post..."}
  ],
  "apxm": {
    "schema_version": 1,
    "graph_id": "plan_then_parallelize-a1b2c3d4",
    "execution_id": "exec-1743984000000-0",
    "node_id": 5,
    "node_name": "assemble",
    "priority_class": "critical_path",
    "downstream_nodes": [],
    "pin_policy": {"mode": "none"},
    "compiler_hints": {"shared_prefix_est_tokens": 120}
  }
}
```

**What vLLM does:**
1. `downstream_nodes: []` → this is the last LLM node, no need to pin anything
2. `pin_policy: "none"` → confirms: don't hold KV blocks
3. vLLM can now **release all pinned blocks** for graph `plan_then_parallelize-a1b2c3d4`

### Stage 9: Runtime — Graph Release (After Execution Completes)

```
DELETE http://localhost:8000/v1/apxm/graphs/plan_then_parallelize-a1b2c3d4
```

**Response:**
```json
{
  "object": "apxm.graph.release",
  "graph_id": "plan_then_parallelize-a1b2c3d4",
  "released_handles": 4,
  "released_blocks": 128
}
```

### Stage 10: Mixed Routing — Some Nodes to vLLM, Some to Cloud

Config example where ASK/THINK go to vLLM but the user has a cloud fallback:

```toml
[[backends]]
name = "local-gpu"
protocol = "vllm"
endpoint = "http://localhost:8000"

[[backends]]
name = "cloud"
protocol = "anthropic"
api_key = "env:ANTHROPIC_API_KEY"

[chat.routing.operation_routes.ask]
backend = "local-gpu"
model = "meta-llama/Llama-4-Maverick-17B-128E"

[chat.routing.operation_routes.think]
backend = "local-gpu"
model = "meta-llama/Llama-4-Maverick-17B-128E"
```

For our `plan_fan_out` graph:
- Nodes 1-4 (ASK) → route to `local-gpu` (vLLM) → `apxm_hints` attached → `GraphAwareVllmBackend.inject_hints()` merges into `extra_body.apxm` → vLLM uses hints
- Node 5 (THINK) → also routes to `local-gpu` → same
- If a node had `model="claude-sonnet"`, it would route to `cloud` (Anthropic) → `apxm_hints` still on the request, but `AnthropicBackend.generate()` never reads `apxm_hints` → ignored, zero overhead

### Stage 11: No vLLM Backend At All (Pure Cloud Setup)

If no backend has `protocol = "vllm"`:

1. `build_context()` calls `llm_registry.find_graph_aware_backend()` → returns `None`
2. `ctx.vllm_graph_id` stays `None`
3. No `VllmGraphLifecycle` is created → no registration HTTP call
4. In `llm.rs`, `if ctx.vllm_graph_id.is_some()` is false → `inject_vllm_hints()` never runs
5. `LLMRequest.apxm_hints` stays `None`
6. OpenAI/Anthropic backend sends clean standard request
7. **Zero overhead**: no allocations, no serialization, no HTTP calls
8. The `_vllm_*` node attributes are still in the artifact (from compilation) but never read

---

## Python Example: End-to-End vLLM Integration

### Step 1: Register vLLM Backend (One-Time Setup)

```bash
# Register a local vLLM instance as a backend
dekk apxm backend add local-gpu \
    --type local \
    --protocol vllm \
    --endpoint http://localhost:8000

# Add a model to it
dekk apxm backend add-model local-gpu \
    meta-llama/Llama-4-Maverick-17B-128E \
    --alias llama4 \
    --alias local

# Route ASK and THINK to vLLM by default
# (edit ~/.apxm/config.toml or use CLI)
```

Resulting `~/.apxm/config.toml`:
```toml
[[backends]]
name = "local-gpu"
type = "local"
protocol = "vllm"
endpoint = "http://localhost:8000"

[[backends.models]]
id = "meta-llama/Llama-4-Maverick-17B-128E"
aliases = ["llama4", "local"]

[chat]
default_backend = "local-gpu"
default_model = "meta-llama/Llama-4-Maverick-17B-128E"
```

### Step 2: Write the Graph (Pure Python — No vLLM-Specific Code!)

```python
#!/usr/bin/env python3
"""code_review.py — Fan-out code review with vLLM graph awareness.

Three reviewers analyze the same diff in parallel, then a synthesizer
merges their findings. vLLM sees the full graph and optimizes KV cache
sharing across the three parallel ASK nodes (same system prompt).

Usage:
    python3 -m examples.python.patterns.code_review
    # or:
    dekk apxm execute code_review.apxm
"""

from apxm.graph import compile, GraphRecorder


@compile()
def code_review(g: GraphRecorder):
    """Three-reviewer parallel code review with synthesis."""

    # Stage 1: Get the diff (single ASK, entry point)
    diff = g.ask(
        "diff_summary",
        "Summarize this code change in 3 sentences:\n"
        "- What files changed\n"
        "- What the intent is\n"
        "- What patterns are used"
    )

    # Stage 2: Three parallel reviewers (fan-out)
    # All share the same default system prompt → vLLM detects this as
    # a reuse_group and pins the KV cache after the first reviewer runs.
    security = g.ask(
        "security_review",
        "Review this code change for SECURITY issues:\n{diff}\n\n"
        "Check for: injection, auth bypass, data exposure, SSRF, "
        "path traversal. Rate severity: critical/high/medium/low."
    )

    perf = g.ask(
        "perf_review",
        "Review this code change for PERFORMANCE issues:\n{diff}\n\n"
        "Check for: N+1 queries, unbounded allocations, blocking I/O, "
        "missing indexes, cache misses. Rate impact: critical/high/medium/low."
    )

    style = g.ask(
        "style_review",
        "Review this code change for CODE QUALITY issues:\n{diff}\n\n"
        "Check for: naming, duplication, complexity, missing tests, "
        "error handling gaps. Rate severity: critical/high/medium/low."
    )

    # Stage 3: Synthesize all reviews (fan-in, uses THINK for deeper reasoning)
    synthesis = g.think(
        "synthesis",
        "Three code reviewers analyzed the same diff. Synthesize:\n\n"
        "SECURITY:\n{security}\n\n"
        "PERFORMANCE:\n{perf}\n\n"
        "CODE QUALITY:\n{style}\n\n"
        "Produce a unified review with:\n"
        "1. Blockers (must fix before merge)\n"
        "2. Suggestions (should fix)\n"
        "3. Nits (nice to have)\n"
        "4. Overall verdict: APPROVE / REQUEST_CHANGES / NEEDS_DISCUSSION"
    )

    output = g.print("=== CODE REVIEW ===\n{synthesis}")
    g.done(output)
```

### Step 3: What Happens When You Run It

```bash
dekk apxm execute code_review.apxm --emit-session
```

**Behind the scenes:**

1. **Python `@compile()`** captures the graph → `ApxmGraph` with 7 nodes, 7 edges
2. **`to_air()`** emits MLIR:
   ```mlir
   module {
     func.func @code_review() -> !ais.token attributes {ais.entry} {
       %diff_summary = ais.ask "Summarize this code change..." : !ais.token
       %security_review = ais.ask "Review...SECURITY..." [%diff_summary : !ais.token] : !ais.token
       %perf_review = ais.ask "Review...PERFORMANCE..." [%diff_summary : !ais.token] : !ais.token
       %style_review = ais.ask "Review...CODE QUALITY..." [%diff_summary : !ais.token] : !ais.token
       %synthesis = ais.think "Three code reviewers..." [%security_review, %perf_review, %style_review : !ais.token, !ais.token, !ais.token] : !ais.token
       ais.print "=== CODE REVIEW ===\n{0}" [%synthesis : !ais.token]
       func.return %synthesis : !ais.token
     }
   }
   ```

3. **Rust compiler** (`Pipeline::compile_graph()` at O2):
   - `constant_folding()` — nothing to fold
   - `prompt_caching()` — nodes 2,3,4 share default system prompt → marks nodes 3,4 with `cached_system_prompt: true`
   - `memoization_hints()` — no duplicates
   - `parallelism_analysis()` — `max_parallelism: 3`, critical path length: 3
   - **`vllm_hints()`** — the new pass:
     - Node 1 (`diff_summary`): critical_path, downstream=[2,3,4], reuse_group=`sp_ask_default`, warmup=true
     - Nodes 2,3,4 (`security/perf/style`): critical_path, downstream=[5], reuse_group=`sp_ask_default`
     - Node 5 (`synthesis`): critical_path, downstream=[], no reuse_group (THINK has different system prompt)

4. **Runtime detects** `local-gpu` backend has `protocol=vllm` → sets `ctx.vllm_graph_id`

5. **Graph registration** → POST to `http://localhost:8000/v1/apxm/graphs/register` with 5 LLM nodes

6. **Execution** — scheduler fires nodes as inputs become ready:
   - `diff_summary` fires first → vLLM gets `apxm.warmup_candidate=true`, pins system prompt KV
   - `security_review`, `perf_review`, `style_review` fire in parallel → vLLM **reuses pinned KV prefix** → 3x less prefix computation
   - `synthesis` fires last → different system prompt, no prefix reuse

7. **Graph release** → DELETE cleans up pinned blocks

### What the User Doesn't Need to Know

The beauty of this design: **the Python code has zero vLLM-specific logic**. The user writes a normal graph with `g.ask()` and `g.think()`. The integration happens entirely in:
- `config.toml` → registers vLLM as a backend (`protocol = "vllm"`)
- Compiler → `vllm_hints()` pass annotates nodes automatically
- Runtime → detects vLLM backend, injects hints transparently

The same `code_review.py` graph works identically whether the backend is:
- vLLM local (`protocol = "vllm"`) → full graph-aware scheduling
- OpenAI cloud (`protocol = "openai"`) → standard API calls, zero overhead
- Anthropic (`protocol = "anthropic"`) → same
- Mixed routing (ASK→vLLM, THINK→cloud) → hints only used by vLLM requests

### Per-Node Override (Advanced — Uses Registered Aliases Only)

For users who want to override routing on specific nodes, they reference
**registered model aliases** from `config.toml` — never hardcoded strings:

```python
# These reference aliases registered via:
#   dekk apxm backend add-model local-gpu meta-llama/... --alias local
#   dekk apxm backend add-model amd-gateway claude-sonnet-4-5 --alias smart
#
# The alias → backend mapping lives in config.toml, not in Python code.

fast_check = g.ask("Quick check...", model="local")    # → vLLM (alias "local")
deep_review = g.think("Deep analysis...", model="smart")  # → cloud (alias "smart")
```

**No hardcoded model IDs, no hardcoded provider names, no hardcoded URLs.**
Everything resolves through the registry:
```
"local" → config.toml model_aliases → "meta-llama/Llama-4-Maverick-17B-128E"
       → model_routes → "local-gpu" backend → protocol=vllm → GraphAwareVllmBackend
```

---

### Summary: The Complete Data Flow

```
Python source (@compile decorator)
    ↓ GraphRecorder captures nodes + edges
ApxmGraph (Python dataclass: nodes, edges, attributes, parameters)
    ↓ to_air() emits valid MLIR
MLIR text (.air) — SSA form, Module::parse() in Rust
    ↓ Pipeline::compile_graph()
    ↓   1. run_graph_passes() on Rust ApxmGraph:
    ↓      constant_folding → prompt_caching → memoization_hints
    ↓      → parallelism_analysis → vllm_hints()  ← NEW PASS
    ↓   2. lower_to_execution_dag() — attributes clone'd to Node
    ↓   3. MLIR passes (CSE, canonicalize, etc.)
    ↓   4. Codegen → artifact binary
.apxmobj artifact (WireNode carries _vllm_* attributes)
    ↓ Runtime loads artifact → ExecutionDag
    ↓ build_context() detects vLLM backend
ExecutionContext { vllm_graph_id, vllm_execution_id }
    ↓ VllmGraphLifecycle::register()
POST /v1/apxm/graphs/register → vLLM knows the full DAG
    ↓ DataflowScheduler fires nodes as inputs become ready
    ↓ LLM handler → inject_vllm_hints() reads node._vllm_* attrs
    ↓ LLMRequest.with_apxm_hints(hints)
    ↓ GraphAwareVllmBackend.inject_hints() → extra_body.apxm
POST /v1/chat/completions + apxm hints → vLLM schedules intelligently
    ↓ ... all nodes complete ...
    ↓ VllmGraphLifecycle::release()
DELETE /v1/apxm/graphs/{id} → vLLM frees pinned KV blocks
```
