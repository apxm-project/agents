# vLLM Graph-Aware Hint Pipeline

End-to-end trace of how APXM workflows compile to vLLM scheduling hints,
from Python source through `.air` (MLIR) compilation to the HTTP request
body that reaches the vLLM server.

---

## 1. Python Source

Workflows are authored in Python using the `@compile()` decorator and
`GraphRecorder` API. Here is the `shared_prefix_fanout` benchmark
(`examples/python/benchmarks/shared_prefix_fanout.py`), which exercises
the exact compiler passes that produce vLLM hints:

```python
from apxm import compile, GraphRecorder

@compile()
def shared_prefix_fanout(g: GraphRecorder):
    """One large context shared by 4 parallel review nodes."""

    context_text = """You are reviewing a large codebase. Here is the complete context:

        MODULE: Authentication System
        ================================
        The authentication system handles user login, session management,
        and JWT token validation. ...
        """

    # Parallel review tasks -- all share the same large context
    review_security = g.ask(
        "review_security",
        context_text + "\n\nFocus on SECURITY aspects:\n"
        "1. Are there any authentication vulnerabilities?\n"
        "2. Is the JWT implementation secure?\n"
        "3. Are rate limits sufficient?\n"
        "Provide a security assessment in 3-4 sentences."
    )

    review_performance = g.ask(
        "review_performance",
        context_text + "\n\nFocus on PERFORMANCE aspects:\n..."
    )

    review_reliability = g.ask(
        "review_reliability",
        context_text + "\n\nFocus on RELIABILITY aspects:\n..."
    )

    review_scalability = g.ask(
        "review_scalability",
        context_text + "\n\nFocus on SCALABILITY aspects:\n..."
    )

    final_report = g.merge(
        "final_report",
        review_security, review_performance,
        review_reliability, review_scalability
    )

    output = g.print(
        "=== COMPLETE CODE REVIEW ===\n\n"
        "Security Review:\n{review_security}\n\n"
        "Performance Review:\n{review_performance}\n\n"
        "Reliability Review:\n{review_reliability}\n\n"
        "Scalability Review:\n{review_scalability}"
    )

    output >> final_report
    g.done(final_report)
```

At this stage there are **no vLLM hints**. The four ASK nodes share a
common prefix (`context_text`) but the compiler has not analyzed this yet.

Key features of the Python API used here:

- `g.ask(name, template)` creates an ASK node with `template_str` attribute
- `g.merge(name, *inputs)` creates a MERGE node with data edges from inputs
- `g.print(message)` references node variables by name (e.g. `{review_security}`)
  which auto-wires data edges and stamps `input_names` so the runtime can
  look up each value by source name (no positional placeholders).
- `output >> final_report` creates a Control edge (sequencing)
- `g.done(node)` creates a RETURN node

---

## 2. AIR Generation (Python to MLIR)

The Python frontend's `ApxmGraph.to_air()` method
(`crates/compiler/apxm-frontend/python/apxm/ir.py`) topologically sorts nodes,
resolves data edges to SSA operands, and emits valid MLIR text in the
AIS dialect:

```mlir
module {
  func.func @shared_prefix_fanout() -> !ais.token attributes {ais.entry} {
    %review_security = ais.ask "You are reviewing a large codebase...\n\nFocus on SECURITY..." : !ais.token
    %review_performance = ais.ask "You are reviewing a large codebase...\n\nFocus on PERFORMANCE..." : !ais.token
    %review_reliability = ais.ask "You are reviewing a large codebase...\n\nFocus on RELIABILITY..." : !ais.token
    %review_scalability = ais.ask "You are reviewing a large codebase...\n\nFocus on SCALABILITY..." : !ais.token
    %final_report = ais.merge %review_security, %review_performance, %review_reliability, %review_scalability : !ais.token, !ais.token, !ais.token, !ais.token -> !ais.token
    ais.print "=== COMPLETE CODE REVIEW ===\n\n..." [%review_security, %review_performance, %review_reliability, %review_scalability : !ais.token, !ais.token, !ais.token, !ais.token]
    func.return %final_report : !ais.token
  }
}
```

Each AIS op lives in the `ais.` dialect namespace. User-supplied
attributes (`template_str`) are carried through as-is. The four ASK
nodes have no input operands — they're independent and can execute in
parallel.

### Compilation entry points

The CLI accepts `.air` text directly (`apxm compile graph.air`).
Internally, `Module::parse()` (C++ FFI) parses the MLIR text into
an in-memory module for optimization passes.

---

## 3. Compiler Passes

The pass pipeline runs at O1/O2/O3 and annotates ops with scheduling
metadata. Two passes are directly relevant to vLLM hints:

### 3a. PromptCanonicalization (O2+ only)

**File:** `crates/compiler/apxm-compiler/mlir/lib/Dialect/AIS/Transforms/PromptCanonicalization.cpp`

Groups LLM ops that share identical upstream context operands. For each
group it:

1. Rewrites templates so the shared prefix comes first, followed by an
   op-specific suffix.
2. Sets three attributes on each grouped op:

| Attribute | Type | Value |
|-----------|------|-------|
| `ais.shared_prefix_group` | String | Group id, e.g. `"shared_prefix_0"` |
| `ais.shared_prefix_est_tokens` | i64 | `512 * context_operand_count` |
| `ais.warmup_candidate` | bool | `true` on the **first** op in the group only |

After this pass, the four ASK ops above would all carry
`ais.shared_prefix_group = "shared_prefix_0"` and the first one would
also carry `ais.warmup_candidate = true`.

### 3b. AssignPriority (O1+)

**File:** `crates/compiler/apxm-compiler/mlir/lib/Dialect/AIS/Transforms/AssignPriority.cpp`

Performs critical-path analysis on the DAG:

1. Computes longest-path depth from each op to a sink node.
2. Assigns a numeric `priority` (90 = critical, 70 = high fan-out,
   30 = normal).
3. Collects the set of downstream consumer op IDs.

| Attribute | Type | Value |
|-----------|------|-------|
| `priority` | i32 | 90, 70, or 30 |
| `ais.downstream_nodes` | ArrayAttr\<i32\> | e.g. `[4]` |

### 3c. Other scheduling passes

**CapabilityScheduling** adds `ais.tier`, `ais.intent`, `ais.latency`,
`ais.estimated_cost`, and `ais.parallel_safe` — used by the dataflow
scheduler but not forwarded to vLLM.

### Pass ordering

```
O1:  normalize -> build-prompt -> dspy-optimize -> assign-priority
     -> scheduling -> fuse-ask-ops -> canonicalizer -> cse -> symbol-dce

O2:  normalize -> build-prompt -> dspy-optimize -> assign-priority
     -> prompt-canonicalization -> template-specialization
     -> scheduling -> fuse-ask-ops -> condense-ops
     -> dead-context-elimination -> canonicalizer -> cse -> symbol-dce
```

`assign-priority` runs at all optimization levels. `prompt-canonicalization`
runs only at O2+.

---

## 4. Artifact Emission

**File:** `crates/compiler/apxm-compiler/mlir/lib/Dialect/AIS/Conversion/Artifact/ArtifactEmitter.cpp`

`emitNode()` walks each MLIR op and collects its attributes into an
`ArtifactNode`. Two transformations happen:

### 4a. Semantic renaming

A handful of attribute names are remapped to match runtime expectations:

| MLIR name | Artifact name |
|-----------|---------------|
| `parameters` | `params` |
| `space` | `memory_tier` |
| `child_agent` | `agent_name` |
| `payload` | `message` |

### 4b. Dialect prefix stripping

After the semantic remapping, any remaining `ais.` prefix is stripped:

```cpp
if (emitKey.starts_with("ais."))
    emitKey = emitKey.drop_front(4);
```

This converts compiler-set attributes to their runtime names:

| MLIR attribute | Artifact key |
|----------------|-------------|
| `ais.shared_prefix_group` | `shared_prefix_group` |
| `ais.shared_prefix_est_tokens` | `shared_prefix_est_tokens` |
| `ais.warmup_candidate` | `warmup_candidate` |
| `ais.downstream_nodes` | `downstream_nodes` |
| `ais.tier` | `tier` |

### 4c. Value conversion

`convertAttribute()` recursively converts MLIR attribute types:

- `IntegerAttr(1-bit)` -> bool
- `IntegerAttr(N-bit)` -> i64
- `FloatAttr` -> f64
- `StringAttr` -> string
- `ArrayAttr` -> array (recursive)
- `DictionaryAttr` -> object (recursive)

An `ArrayAttr<IntegerAttr>` like `[4]` (from `downstream_nodes`)
becomes a JSON-like `[4]` array of integers in the wire format.

### 4d. Binary format

The artifact is serialized as a little-endian binary (wire format v3),
wrapped in a container with BLAKE3 integrity hash:

```
[MAGIC "APXM"][VERSION 1][PAYLOAD_LEN][BLAKE3 HASH][FLAGS][PAYLOAD]
  +-- [WIRE_VERSION 3][DAG_COUNT][DAG_1 ... DAG_N]
       +-- per DAG: module_name, is_entry, params, nodes, edges, entry/exit
            +-- per node: id, op_index, attributes (HashMap<String, Value>),
                         input_tokens, output_tokens, metadata {priority, ...}
```

---

## 5. Runtime: Reading Attributes

When the runtime deserializes the artifact, each `Node` has a
`HashMap<String, Value>` of attributes with **bare keys** (no `ais.` prefix).

### 5a. Rust constants

`crates/core/apxm-core/src/constants.rs` defines the lookup keys:

```rust
pub const WARMUP_CANDIDATE: &str = "warmup_candidate";
pub const SHARED_PREFIX_EST_TOKENS: &str = "shared_prefix_est_tokens";
pub const DOWNSTREAM_NODES: &str = "downstream_nodes";
pub const REUSE_GROUP: &str = "shared_prefix_group";  // matches stripped MLIR name
```

### 5b. Graph registration (engine.rs)

Before DAG execution begins, `ExecutorEngine::execute_dag()` builds a
`GraphMetadata` object from the DAG nodes and broadcasts it to all
backends:

```rust
// crates/runtime/apxm-runtime/src/executor/engine.rs
let graph_id = self.context.execution_id.clone();
self.register_graph_metadata(&dag, &graph_id).await;    // POST /v1/apxm/graphs/register
let result = self.execute_dag_inner(dag).await;
self.context.llm_registry.release_graph_all(&graph_id).await;  // DELETE /v1/apxm/graphs/{id}
```

`register_graph_metadata()` reads each node's attributes and constructs
a `NodeSpec`:

```rust
NodeSpec {
    node_id: node.id as u32,
    node_name: node.metadata.name.clone(),
    estimated_prompt_tokens: attrs[SHARED_PREFIX_EST_TOKENS],
    downstream_nodes: attrs[DOWNSTREAM_NODES],  // Vec<u32>
    priority_class: match node.metadata.priority {
        90.. => "critical_path",
        60..=89 => "normal",
        _ => "speculative",
    },
    reuse_group: attrs[REUSE_GROUP],
    is_critical_path: priority >= 90,
}
```

The resulting JSON is sent to `POST /v1/apxm/graphs/register`:

```json
{
  "graph_id": "exec-abc-123",
  "execution_id": "exec-abc-123",
  "node_count": 4,
  "nodes": [
    {
      "node_id": 0,
      "node_name": "review_security",
      "estimated_prompt_tokens": 512,
      "downstream_nodes": [4],
      "priority_class": "normal",
      "reuse_group": "shared_prefix_0",
      "is_critical_path": false
    }
  ]
}
```

### 5c. Per-request hints (llm.rs)

When each LLM node executes, the handler reads the same attributes and
builds `ApxmGraphHints`:

```rust
// crates/runtime/apxm-runtime/src/executor/handlers/llm.rs
let warmup_candidate     = attrs[WARMUP_CANDIDATE].as_bool();
let shared_prefix_tokens = attrs[SHARED_PREFIX_EST_TOKENS].as_u64().map(|u| u as u32);
let reuse_group          = attrs[REUSE_GROUP].as_string();
let downstream_nodes     = get_u32_array_attribute(node, DOWNSTREAM_NODES);

let pin_policy = if reuse_group.is_some() {
    PinPolicy::prefix_default()   // mode="prefix", ttl_ms=None (use graph default)
} else {
    PinPolicy::none()
};

let hints = ApxmGraphHints {
    schema_version: 1,
    graph_id: Some(execution_id),
    execution_id: Some(execution_id),
    node_id: Some(node.id as u32),
    node_name: Some(name),
    priority_class: Some(priority_class),  // "critical_path" | "normal" | "speculative"
    downstream_nodes,
    reuse_group,
    pin_policy,
    compiler_hints: CompilerHints {
        shared_prefix_est_tokens: shared_prefix_tokens,
        warmup_candidate,
        pipeline_candidate: None,
    },
};

request = request.with_apxm_hints(hints);
```

### 5d. Warmup handler

`warmup.rs` reads the same attributes to decide whether to fire a
prefix-cache warmup request **before** the actual LLM call:

```
warmup fires when ALL of:
  1. warmup_candidate = true
  2. shared_prefix_est_tokens >= min_prefix_tokens (default 512)
  3. downstream_nodes.len() >= min_fanout (default 2)
  4. optimization target != "cost"
```

---

## 6. Backend: Injection into vLLM

### 6a. GraphAwareVllmBackend.inject_hints()

**File:** `crates/runtime/apxm-backends/src/llm/backends/vllm/backend.rs`

Serializes `ApxmGraphHints` into `extra_body.apxm`:

```rust
fn inject_hints(&self, mut request: LLMRequest) -> LLMRequest {
    if let Some(ref hints) = request.apxm_hints {
        let hints_json = serde_json::to_value(hints).unwrap_or_default();
        let mut extra = request.extra_body.take().unwrap_or(json!({}));
        extra["apxm"] = hints_json;
        request.extra_body = Some(extra);
    }
    request
}
```

### 6b. OpenAI backend: priority mapping

**File:** `crates/runtime/apxm-backends/src/llm/backends/openai/backend.rs`

`build_request_body()` maps `priority_class` to a vLLM integer and
merges `extra_body` into the top-level request:

```rust
// priority_class -> vLLM integer (lower = higher priority)
let priority = match priority_class.as_str() {
    "critical_path" => 0,
    "normal"        => 5,
    "speculative"   => 10,
    _               => 5,
};
body["priority"] = json!(priority);

// Merge extra_body (includes "apxm" key) into body
for (key, value) in extra_body {
    body[key] = value;
}
```

### 6c. Final HTTP request

The request sent to `POST /v1/chat/completions`:

```json
{
  "model": "meta-llama/Llama-3.1-8B-Instruct",
  "messages": [{"role": "user", "content": "You are reviewing a large codebase...\nFocus on SECURITY..."}],
  "temperature": 0.7,
  "priority": 5,
  "apxm": {
    "schema_version": 1,
    "graph_id": "exec-abc-123",
    "execution_id": "exec-abc-123",
    "node_id": 0,
    "node_name": "review_security",
    "priority_class": "normal",
    "downstream_nodes": [4],
    "reuse_group": "shared_prefix_0",
    "pin_policy": {"mode": "prefix"},
    "compiler_hints": {
      "shared_prefix_est_tokens": 512,
      "warmup_candidate": true
    }
  }
}
```

vLLM's scheduler reads `priority` to order the request queue and
`apxm.*` fields to make KV-cache pinning and prefetch decisions.

### 6d. Post-generation KV-cache pinning

After a successful response, if `pin_policy.mode == "prefix"`:

```rust
// Fire-and-forget: POST /v1/apxm/pins
self.pin_prefix(graph_id, node_id, reuse_group, ttl_ms).await;
```

```json
{
  "graph_id": "exec-abc-123",
  "node_id": 0,
  "reuse_group": "shared_prefix_0",
  "ttl_ms": 30000
}
```

This tells vLLM to keep the KV blocks for this request's prompt prefix
alive so nodes 1-3 (same `reuse_group`) can reuse them.

### 6e. Graph release

After the DAG finishes (success or error), the engine calls
`release_graph_all()` which sends `DELETE /v1/apxm/graphs/{graph_id}`
to free all pinned blocks.

---

## 7. Data Flow Summary

```
+---------------------------------------------------------------------+
|                        COMPILE TIME                                  |
|                                                                      |
|  Python (@compile)                                                   |
|      |                                                               |
|      v                                                               |
|  ApxmGraph.to_air() --> .air (valid MLIR text)                       |
|      |                                                               |
|      v  (alt: JSON graph --> AirModule --> to_air() --> .air text)   |
|  Module::parse() [C++ FFI]                                           |
|      |                                                               |
|      v                                                               |
|  MLIR Module                                                         |
|                                                                      |
|  +------------------ Pass Pipeline ------------------+               |
|  |                                                   |               |
|  |  PromptCanonicalization (O2+)                      |               |
|  |    sets: ais.shared_prefix_group                  |               |
|  |          ais.shared_prefix_est_tokens             |               |
|  |          ais.warmup_candidate                     |               |
|  |                                                   |               |
|  |  AssignPriority (O1+)                             |               |
|  |    sets: priority                                 |               |
|  |          ais.downstream_nodes                     |               |
|  |                                                   |               |
|  +---------------------------------------------------+               |
|                          |                                           |
|                    ArtifactEmitter                                    |
|                 strips "ais." prefix                                  |
|                          |                                           |
|                    .apxmobj binary                                    |
|         (attributes stored with bare keys)                           |
+----------------------------------------------------------------------+

+----------------------------------------------------------------------+
|                        RUN TIME                                      |
|                                                                      |
|  Artifact deserialized --> ExecutionDag with Node.attributes         |
|                                                                      |
|  +-- Engine (once per graph) ----------------------------+           |
|  |  register_graph_metadata()                            |           |
|  |    reads: downstream_nodes, shared_prefix_group,      |           |
|  |           shared_prefix_est_tokens, priority           |           |
|  |    sends: POST /v1/apxm/graphs/register               |           |
|  +-------------------------------------------------------+           |
|                                                                      |
|  +-- LLM Handler (per node) -----------------------------+           |
|  |  reads: warmup_candidate, shared_prefix_est_tokens,   |           |
|  |         shared_prefix_group, downstream_nodes, priority|           |
|  |  builds: ApxmGraphHints + CompilerHints + PinPolicy   |           |
|  |  attaches to LLMRequest via with_apxm_hints()         |           |
|  +-------------------------------------------------------+           |
|                          |                                           |
|  +-- GraphAwareVllmBackend ------------------------------+           |
|  |  inject_hints()   -> extra_body.apxm = {hints JSON}  |           |
|  |                          |                            |           |
|  |  +-- OpenAI Backend ---------------------------+      |           |
|  |  |  priority_class -> integer priority (0/5/10)|      |           |
|  |  |  merge extra_body into request body         |      |           |
|  |  |  POST /v1/chat/completions                  |      |           |
|  |  +---------------------------------------------+      |           |
|  |                          |                            |           |
|  |  pin_prefix()    -> POST /v1/apxm/pins (if prefix)   |           |
|  +-------------------------------------------------------+           |
|                                                                      |
|  +-- Engine (cleanup) -----------------------------------+           |
|  |  release_graph_all() -> DELETE /v1/apxm/graphs/{id}   |           |
|  +-------------------------------------------------------+           |
+----------------------------------------------------------------------+
```

---

## 8. Attribute Name Alignment

| C++ MLIR constant | MLIR attr name | After stripping | Rust constant | Runtime key |
|---|---|---|---|---|
| `attrs::SHARED_PREFIX_GROUP` | `ais.shared_prefix_group` | `shared_prefix_group` | `REUSE_GROUP` | `"shared_prefix_group"` |
| `attrs::SHARED_PREFIX_EST_TOKENS` | `ais.shared_prefix_est_tokens` | `shared_prefix_est_tokens` | `SHARED_PREFIX_EST_TOKENS` | `"shared_prefix_est_tokens"` |
| `attrs::WARMUP_CANDIDATE` | `ais.warmup_candidate` | `warmup_candidate` | `WARMUP_CANDIDATE` | `"warmup_candidate"` |
| `attrs::DOWNSTREAM_NODES` | `ais.downstream_nodes` | `downstream_nodes` | `DOWNSTREAM_NODES` | `"downstream_nodes"` |
| *(raw)* | `priority` | `priority` | *(metadata field)* | `node.metadata.priority` |

---

## 9. Key Files

| Stage | File | Purpose |
|-------|------|---------|
| Python frontend | `crates/compiler/apxm-frontend/python/apxm/ir.py` | `ApxmGraph.to_air()` -- Python graph to MLIR text |
| Python API | `crates/compiler/apxm-frontend/python/apxm/proxy.py` | `GraphRecorder` -- `g.ask()`, `g.merge()`, etc. |
| Python decorator | `crates/compiler/apxm-frontend/python/apxm/decorators.py` | `@compile()` -- captures graph from function |
| Constants (C++) | `crates/compiler/apxm-compiler/mlir/include/ais/Common/Constants.h` | `ais.*` attribute names |
| Constants (Rust) | `crates/core/apxm-core/src/constants.rs` | Runtime attribute keys |
| Pass: prefix | `crates/compiler/apxm-compiler/mlir/lib/Dialect/AIS/Transforms/PromptCanonicalization.cpp` | Shared prefix detection |
| Pass: priority | `crates/compiler/apxm-compiler/mlir/lib/Dialect/AIS/Transforms/AssignPriority.cpp` | Critical path + downstream |
| Pass: scheduling | `crates/compiler/apxm-compiler/mlir/lib/Dialect/AIS/Transforms/CapabilityScheduling.cpp` | Tier/cost/latency |
| Pipeline | `crates/compiler/apxm-compiler/src/passes/pipeline.rs` | Pass ordering by opt level |
| Artifact emit | `crates/compiler/apxm-compiler/mlir/lib/Dialect/AIS/Conversion/Artifact/ArtifactEmitter.cpp` | MLIR to binary, prefix stripping |
| Artifact parse | `crates/compiler/apxm-compiler/src/codegen/artifact.rs` | Binary to ExecutionDag |
| Engine lifecycle | `crates/runtime/apxm-runtime/src/executor/engine.rs` | Graph register/release |
| LLM handler | `crates/runtime/apxm-runtime/src/executor/handlers/llm.rs` | Hint construction |
| Warmup | `crates/runtime/apxm-runtime/src/executor/handlers/warmup.rs` | Warmup gating logic |
| Hint types | `crates/runtime/apxm-backends/src/llm/backends/vllm/graph_meta.rs` | ApxmGraphHints, GraphMetadata |
| vLLM backend | `crates/runtime/apxm-backends/src/llm/backends/vllm/backend.rs` | Hint injection, KV pinning |
| OpenAI backend | `crates/runtime/apxm-backends/src/llm/backends/openai/backend.rs` | Priority mapping, request body |
| Trait | `crates/runtime/apxm-backends/src/llm/backends/traits.rs` | register_graph/release_graph |
| Registry | `crates/runtime/apxm-backends/src/llm/registry/mod.rs` | Broadcast to all backends |
| Provider | `crates/runtime/apxm-backends/src/llm/provider.rs` | Enum delegation |

---

## 10. Running the Example

```bash
# Generate .air from Python
python3 -c "
from examples.python.benchmarks.shared_prefix_fanout import shared_prefix_fanout
print(shared_prefix_fanout._graph.to_air())
" > shared_prefix_fanout.air

# Compile without optimization (no hints)
dekk apxm execute shared_prefix_fanout.air -O0

# Compile with PromptCanonicalization (shared prefix hints + priority)
dekk apxm execute shared_prefix_fanout.air -O2

# Inspect artifact attributes after compilation
dekk apxm compile shared_prefix_fanout.air -O2 -o out.apxmobj
dekk apxm decompile out.apxmobj
```

At O0, all four ASK nodes are scheduled with equal priority and no
prefix reuse. At O2, the compiler detects the shared context prefix,
groups the nodes, and the runtime sends vLLM hints that enable KV-cache
sharing across all four parallel requests.
