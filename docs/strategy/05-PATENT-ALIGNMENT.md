# Patent Alignment: Spaghetti-Stack Context Management

**Date**: March 31, 2026
**Patent**: "Spaghetti-Stack Context Management for LLM-Based AI Agents" (Internal)
**Inventors**: APXM Contributors

---

## Overview

The patent describes five integrated techniques for managing context in LLM-based agent pipelines. This document maps each technique to its implementation location in the APXM/vLLM/ACPX unified system.

---

## Technique 1: Spaghetti-Stack Context Organization

### Patent Description

Context organized as a cactus stack where only the active branch is loaded. Parent context remains accessible without duplication. Collapsed summaries with reference pointers to stored data chunks.

**Patent figure**: Active context = root summary + Branch B chunks (~1,150 tokens, 34% of flat 3,400 tokens).

### Implementation in APXM

**Location**: `apxm-runtime/src/context_stack/mod.rs`

```rust
pub struct ContextStack {
    root: ContextFrame,
    branches: HashMap<BranchId, Vec<ContextFrame>>,
    active_branch: BranchId,
}

pub struct ContextFrame {
    pub id: FrameId,
    pub parent: Option<FrameId>,
    pub summary: String,                    // Collapsed summary (always loaded)
    pub references: Vec<ContextReference>,  // Pointers to full data (lazy)
    pub data_chunks: Vec<DataChunk>,        // Loaded-on-demand
    pub status: FrameStatus,               // Active | Collapsed
}
```

**How it maps to APXM's existing architecture**:

| APXM Component | Patent Concept | Connection |
|----------------|---------------|------------|
| Memory System (STM/LTM/Episodic) | Data storage backend | STM = session cache, LTM = persistent storage for chunks |
| ExecutionContext | Active branch context | Each node execution gets assembled context from stack |
| DataflowScheduler | Branch management | Parallel branches = parallel execution paths in DAG |
| OperationDispatcher | Context consumer | Each handler receives assembled context, not raw flat state |

**Key insight**: APXM's existing 3-tier memory (STM/LTM/Episodic) maps naturally to the spaghetti stack's storage layers:
- **STM** = Active branch's loaded data chunks (microsecond access)
- **LTM** = Collapsed branches' stored data (millisecond access, demand-paged)
- **Episodic** = Reference pointers and traversal history (audit trail)

---

## Technique 2: Speculative Consumer Execution

### Patent Description

Start downstream agent stages before upstream producers complete. Use memoized outputs from prior runs as predicted inputs. Deterministic commit/rollback semantics.

**Patent distinction**: This is NOT thread-level parallelism. It's CPU-pipeline-style speculation where dependent stages start with predicted inputs and rollback on misprediction.

### Implementation in APXM

**Location**: `apxm-runtime/src/memo/speculation.rs`

```rust
pub struct SpeculativeExecutor {
    memo_cache: Arc<MemoCache>,
    active_speculations: DashMap<NodeId, SpeculativeTask>,
}

pub struct SpeculativeTask {
    pub node_id: NodeId,
    pub predicted_input: Value,          // From memo cache
    pub predicted_input_hash: u64,       // Hash of prediction
    pub result: Option<Value>,           // Speculative result (pending commit)
    pub status: SpeculationStatus,       // Running | Committed | Rolledback
}

impl SpeculativeExecutor {
    /// Check if we can speculate on a downstream node
    pub fn can_speculate(&self, downstream: &Node, graph: &ExecutionDag) -> Option<Value> {
        // 1. Look up downstream's upstream dependency
        // 2. Check memo cache for upstream's prior output
        // 3. If found with sufficient confidence, return predicted input
        None
    }

    /// Commit or rollback when actual upstream result arrives
    pub fn resolve(&self, upstream_id: NodeId, actual_output: &Value) {
        for spec in self.active_speculations.iter() {
            if spec.predicted_input_hash == hash(actual_output) {
                spec.status = SpeculationStatus::Committed;  // Hit!
            } else {
                spec.status = SpeculationStatus::Rolledback;  // Miss, re-execute
            }
        }
    }
}
```

**Integration with DataflowScheduler** (`apxm-runtime/src/scheduler/dataflow.rs`):

```rust
// In the worker loop, before waiting for all inputs:
if let Some(predicted_input) = speculator.can_speculate(&node, &dag) {
    // Start executing with predicted input
    let spec_task = SpeculativeTask::new(node.id, predicted_input);
    speculator.register(spec_task);
    let result = dispatch_node(&node, vec![predicted_input]).await;
    speculator.set_result(node.id, result);
}

// When actual upstream result arrives:
speculator.resolve(upstream_id, &actual_result);
if speculator.was_committed(node.id) {
    // Use speculative result directly -- zero additional latency
} else {
    // Re-execute with actual input
    let result = dispatch_node(&node, vec![actual_result]).await;
}
```

---

## Technique 3: Inter-Stage Token Pipelining

### Patent Description

Stream tokens incrementally into the next stage's prefill phase rather than waiting for full completion. The KV cache is constructed incrementally as chunks arrive. The spaghetti-stack structure tells the runtime which context portions are already materialized (prefillable) vs still being generated (streamable).

### Implementation: APXM Runtime + vLLM

**APXM side** (`apxm-runtime/src/executor/handlers/llm.rs`):

```rust
/// Pipeline tokens from current node into next node's prefill
pub async fn execute_with_pipeline(
    ctx: &ExecutionContext,
    current_node: &Node,
    next_node: Option<&Node>,
    inputs: Vec<Value>,
) -> Result<Value> {
    // 1. Identify pre-materializable context for next node
    let prefillable = if let Some(next) = next_node {
        ctx.context_stack.get_prefillable_context(next)
        // Returns: system prompt + skill embeddings + ancestor summaries
        // (all available before current node finishes)
    } else {
        None
    };

    // 2. Send prefillable context to vLLM for eager prefill
    if let Some(prefix) = prefillable {
        ctx.vllm_backend.start_eager_prefill(next_node.unwrap().id, &prefix).await?;
    }

    // 3. Generate current node with streaming
    let request = build_request(ctx, current_node, inputs)?;
    let mut stream = ctx.vllm_backend.generate_stream(request).await?;

    let mut full_output = String::new();
    while let Some(chunk) = stream.next().await {
        full_output.push_str(&chunk);

        // 4. Stream current output into next node's dynamic context
        if next_node.is_some() {
            ctx.vllm_backend.append_stream_chunk(next_node.unwrap().id, &chunk).await?;
        }
    }

    Ok(Value::String(full_output))
}
```

**vLLM side** (`vllm/extensions/incremental_prefill.py`):

```python
class IncrementalPrefillManager:
    """Manages incremental KV-cache construction for pipelined requests."""

    def start_prefill(self, node_id: int, static_context: str):
        """Begin prefilling static context (system prompt, skills, summaries)."""
        tokens = self.tokenizer.encode(static_context)
        self.partial_kv_caches[node_id] = self.compute_kv(tokens)

    def append_chunk(self, node_id: int, chunk: str):
        """Append streaming tokens to partial KV-cache."""
        tokens = self.tokenizer.encode(chunk)
        self.partial_kv_caches[node_id].extend(tokens)
        # Incrementally compute KV for new tokens

    def finalize(self, node_id: int) -> KVCache:
        """Return complete KV-cache, ready for generation."""
        return self.partial_kv_caches.pop(node_id)
```

**What the patent calls "structurally independent components"** maps to APXM graph knowledge:
- **Ancestor summaries**: Known at graph compile time (pre-materializable)
- **System prompt**: Known at graph compile time (pre-materializable)
- **Upstream output**: Streamed incrementally (dynamic)
- **Tool results**: Available after tool execution (dynamic)

---

## Technique 4: Input-Keyed Chunk Memoization

### Patent Description

Hash combination of chunk inputs, operation type, and model configuration. Two-tier cache: microsecond session-local + millisecond cross-session.

### Implementation in APXM

**Location**: `apxm-runtime/src/memo/mod.rs`

```rust
pub struct MemoCache {
    // Tier 1: Session-local (microsecond access)
    session_cache: DashMap<CacheKey, CachedResult>,

    // Tier 2: Persistent (millisecond access)
    persistent_store: SqlitePool,  // or LTM backend

    // Stats
    hits: AtomicU64,
    misses: AtomicU64,
}

pub struct CacheKey {
    pub input_hash: u64,           // xxhash of all input values
    pub operation: AISOperationType,
    pub model_id: String,
    pub temperature: f64,          // Only cache at temperature=0
    pub system_prompt_hash: u64,   // Hash of system prompt
}

pub struct CachedResult {
    pub output: Value,
    pub created_at: Timestamp,
    pub hit_count: u64,
    pub confidence: f64,           // For speculation: how often this key produces this output
}
```

**Integration with handlers** (`apxm-runtime/src/executor/dispatcher.rs`):

```rust
// Before dispatching to handler:
if node.attributes.get("memoizable").map_or(false, |v| v.as_bool().unwrap_or(false)) {
    let key = CacheKey::from_node_and_inputs(&node, &inputs);
    if let Some(cached) = memo_cache.get(&key).await {
        // Return immediately -- microsecond response
        return Ok(cached.output);
    }
}

// After handler returns:
if node.attributes.get("memoizable").map_or(false, |v| v.as_bool().unwrap_or(false)) {
    let key = CacheKey::from_node_and_inputs(&node, &inputs);
    memo_cache.put(key, CachedResult::new(result.clone())).await;
}
```

**Connection to APXM's existing memory system**:
- Session cache = STM (Short-Term Memory, in-memory HashMap)
- Persistent cache = LTM (Long-Term Memory, SQLite backend)
- Both already exist in the APXM runtime

---

## Technique 5: Stage-Specific Context Loading

### Patent Description

Each pipeline stage sees only context relevant to its task. Stage manifests declare context dependencies, resolved dynamically against the spaghetti stack.

### Implementation in APXM

**Graph-level declaration** (in APXM graph JSON):

```json
{
  "id": 5,
  "name": "review_api",
  "op": "ASK",
  "attributes": {
    "template": "Review the API endpoint: {0}",
    "context_manifest": {
      "include": ["api_endpoints", "project_conventions", "test_results"],
      "exclude": ["auth_module", "build_logs", "old_reasoning_traces"],
      "max_tokens": 4096
    }
  }
}
```

**Runtime resolution** (`apxm-runtime/src/context_stack/manifest.rs`):

```rust
pub struct ContextManifest {
    pub include: Vec<String>,      // Required context keys
    pub exclude: Vec<String>,      // Explicitly excluded
    pub max_tokens: Option<usize>, // Token budget for this node
}

impl ContextStack {
    pub fn assemble_for_manifest(&self, manifest: &ContextManifest) -> AssembledContext {
        let mut context = AssembledContext::new();

        // 1. Always include: root summary + active branch summaries
        context.add_summaries(self.walk_to_root());

        // 2. Include declared dependencies
        for key in &manifest.include {
            if let Some(chunk) = self.resolve_reference(key) {
                context.add_chunk(chunk);
            }
        }

        // 3. Exclude explicitly blocked context
        context.remove_keys(&manifest.exclude);

        // 4. Enforce token budget
        if let Some(max) = manifest.max_tokens {
            context.truncate_to_budget(max);
        }

        context
    }
}
```

---

## Synergy Map

The patent emphasizes that these techniques **reinforce each other**:

```
                    ┌──────────────────┐
                    │ Spaghetti Stack  │
                    │ (Technique 1)    │
                    └────────┬─────────┘
                             │
                     Provides structure for
                             │
              ┌──────────────┼──────────────┐
              │              │              │
              v              v              v
     ┌────────────┐  ┌────────────┐  ┌────────────┐
     │ Stage-Spec │  │ Speculation│  │ Pipelining │
     │ Loading    │  │ (Tech 2)   │  │ (Tech 3)   │
     │ (Tech 5)   │  │            │  │            │
     └──────┬─────┘  └──────┬─────┘  └──────┬─────┘
            │               │               │
            │    Memoization feeds           │
            │    speculation accuracy        │
            │               │               │
            └───────┐       │       ┌───────┘
                    v       v       v
              ┌────────────────────────┐
              │     Memoization        │
              │     (Technique 4)      │
              └────────────────────────┘
```

| Synergy | How |
|---------|-----|
| Memo -> Speculation | Cached outputs serve as predicted inputs for speculative execution |
| Stack -> Stage Loading | Reference pointers determine per-stage data needs |
| Pipelining + Speculation | Speculative consumers process pipelined tokens before producer completes |
| Stack -> Pipelining | Stack structure separates pre-materializable vs streaming context |
| Memo -> Cost Reduction | Cached results eliminate entire inference calls |
| Stage Loading -> Quality | Filtering irrelevant context improves model attention |

---

## APXM Existing Infrastructure Reuse

| Patent Concept | APXM Existing | Reuse Potential |
|---------------|---------------|----------------|
| Data storage | STM/LTM/Episodic memory | Direct reuse for chunk storage + cache tiers |
| Branch management | DataflowScheduler parallel paths | DAG branches = context branches |
| Reference pointers | Node attributes | Store manifest references as attributes |
| Traversal | DAG dependency resolution | Bottom-up = reverse topological walk |
| Token counting | LLMRequest token estimates | Extend for context budget analysis |
| Rollback | TryCatch error handler | Extend for speculative rollback |
| Streaming | SSE in apxm-server | Extend for token pipelining |

---

## Product Alignment

Per the patent's deployment section:

| Product | Integration Point |
|---------|------------------|
| accelerator hardware | vLLM running on GPUs with graph-aware KV-cache management |
| GPU Runtime | Context-aware memory management libraries for KV-cache pinning |
| Edge Hardware | Edge deployment of APXM runtime with local model routing |

**Strategic value**: Implementing the patent in APXM + vLLM creates a reference implementation on target hardware, demonstrating the techniques described in the filing.
