# DSPy Integration Research

**Date**: 2026-04-08
**Status**: Design + Prototype
**Sources**: Consolidated from DSPY-COMPILER-PASS.md and DSPY-INTERNALS.md

> **Related documents:**
> - [Strategic context](../strategy/dspy-integration.md) -- positioning and priorities
> - [User guide](../integrations/dspy.md) -- end-user documentation
> - [Optimization overview](../optimization/overview.md) -- how DSPy fits the pass pipeline
> - [Optimization passes](../optimization/passes.md) -- pass ordering and O0-O3 levels
> - [Token estimation research](token-estimation.md) -- token counting for budget enforcement
> - [Quality optimization](optimization/quality.md) -- quality-first DSPy configuration
> - [Token optimization](optimization/tokens.md) -- token-saving DSPy configuration

---

## Overview

DSPy is a prompt optimization framework that treats prompts as learnable parameters. APXM integrates DSPy as a **first-class compiler pass** running at compile time, embedding optimized prompts directly into compiled `.apxmobj` artifacts.

**Core concept**: The `dspy-optimize` pass runs at compile time on the MLIR IR. It extracts all LLM operations (ASK, THINK, REASON, PLAN, REFLECT, VERIFY), optimizes their `template_str` attributes using DSPy algorithms, and replaces the originals with learned versions. The result is baked into the artifact -- zero runtime cost.

**Analogy**: Like PGO (Profile-Guided Optimization) in LLVM, DSPy-optimize uses execution profiles to improve generated code quality. But instead of branch prediction or register allocation, it optimizes LLM prompts.

**Value proposition**:
- 20-40% quality improvement from learned prompts (proven in benchmarks)
- Zero runtime cost -- optimization happens once during compilation
- Composable with existing O1/O2/O3 MLIR passes
- Profile-guided -- uses execution profiles as free training data
- Per-target metrics -- optimize for latency, tokens, quality, or balanced

**Key findings from DSPy internals**:
- DSPy provides NO built-in quality heuristics -- users must define metrics
- Token tracking exists but NO token budget enforcement -- optimization can run indefinitely
- Cost tracking captures `response_cost` from LM providers but doesn't enforce limits
- No dollar budgets -- users must manually stop optimization or set `num_trials`

---

## DSPy Algorithms

### BootstrapFewShot

**Source**: `dspy/teleprompt/bootstrap.py`

BootstrapFewShot generates few-shot demonstrations by running a "teacher" model and filtering by metric success. It is not a search-based optimizer; it performs one-pass success/failure filtering.

**Algorithm**:

1. **Teacher Setup**: Clone the student program as a "teacher" and prime it with `LabeledFewShot(k=max_labeled_demos)` -- raw labeled examples only.

2. **Bootstrap Loop**: For each training example (up to `max_rounds` attempts):
   - Run teacher with `temperature=1.0` (bypasses cache for diversity)
   - Capture the full execution trace (every predictor call)
   - Evaluate with `metric(example, prediction, trace)`
   - If successful, extract demonstrations from the trace

3. **Demo Extraction**: For each step in the trace `(predictor, inputs, outputs)`, create `dspy.Example(augmented=True, **inputs, **outputs)` and store per predictor in `name2traces[predictor_name]`. When one example triggers a predictor N times, randomly sample 50/50 from the first N-1 or last trace.

4. **Train Student**: For each predictor, combine up to `max_bootstrapped_demos` from successful traces, filling remaining slots with raw labeled examples. Assign to `predictor.demos`. Unbootstrapped examples become the validation set.

**Configuration**:
```python
BootstrapFewShot(
    metric=my_metric,             # callable(example, prediction, trace) -> bool or float
    metric_threshold=None,        # if metric returns float, check >= threshold
    max_bootstrapped_demos=4,     # max demos from successful traces
    max_labeled_demos=16,         # max raw labeled examples
    max_rounds=1,                 # attempts per training example
)
```

**Characteristics**: ~5-10 min compile time, +20-30% quality gain. Cost is `max_rounds x len(trainset)` LM calls for bootstrapping plus evaluation.

### MIPROv2

**Source**: `dspy/teleprompt/mipro_optimizer_v2.py`

MIPROv2 uses Bayesian optimization (Optuna with TPE sampler) to search the joint space of {instruction candidates x few-shot demo sets}. It is the most expensive optimizer but yields the highest quality gains.

**Three-step process**:

1. **Bootstrap few-shot examples**: Calls `create_n_fewshot_demo_sets()` which runs `BootstrapFewShot` with shuffled trainset to create N candidate sets of few-shot examples per predictor. Returns `demo_candidates[predictor_idx][candidate_idx] = [demos]`.

2. **Propose instruction candidates**: Uses `GroundedProposer` which takes as input:
   - Dataset summary (10 LM calls to summarize data)
   - Program code (extracted via `inspect.getsource()`)
   - Few-shot examples from step 1
   - Random prompting tip (e.g., "Be creative!", "Keep it simple")

   Outputs `instruction_candidates[predictor_idx] = [N instruction strings]`. LM calls: ~10 + (N x num_predictors) + num_predictors (program-aware).

3. **Bayesian optimization**: Search space is `{instruction_idx: 0..N, demos_idx: 0..M}` per predictor. Total combinations: `N^predictors x M^predictors`. The objective function evaluates each trial configuration on a minibatch (default 35 examples), with full validation every `minibatch_full_eval_steps=5` trials.

**Auto modes**:
| Mode   | Candidates (N) | Val Size | Typical Trials |
|--------|-----------------|----------|----------------|
| light  | 6               | 100      | ~10            |
| medium | 12              | 300      | ~25            |
| heavy  | 18              | 1000     | ~50+           |

**Trial count formula**: `max(2 * num_vars * log2(num_candidates), 1.5 * num_candidates)`. For light mode with 1 predictor: `max(2*2*log2(6), 1.5*6) = max(10.3, 9) ~ 10 trials`.

**Characteristics**: ~15-30 min compile time, +30-40% quality gain. Most expensive but best results for critical workflows.

---

## Compiler Pass Design

### Pipeline Integration

DSPy optimization runs late in the MLIR pass pipeline because it needs structurally-optimized IR:

```
O0: (no passes)

O1: normalize -> build-prompt -> scheduling -> fuse -> canonicalizer -> cse -> dce

O2: normalize -> build-prompt -> scheduling -> fuse ->
    template-specialization -> dead-context -> schema-narrowing -> condense ->
    canonicalizer -> cse -> dce

O3: O2 + dspy-optimize
    normalize -> build-prompt -> scheduling -> fuse ->
    template-specialization -> dead-context -> schema-narrowing -> condense ->
    dspy-optimize -> canonicalizer -> cse -> dce
```

**Placement rationale**:
1. **After `template-specialization`**: DSPy optimizes already-specialized templates
2. **After `schema-narrowing`**: Tighter output schemas provide better DSPy guidance
3. **Before final `canonicalizer`**: DSPy-modified ops can still be canonicalized
4. **Before `cse`**: Identical optimized prompts can be deduplicated

DSPy is O3-only initially because the pass is expensive (~5-20 minutes). Future work could add `--dspy-optimize=auto` to detect cached templates, making it viable at O2.

### Pass Algorithm

```
dspy-optimize pass:
  FOR each operation in module:
    IF op.kind in {ASK, THINK, REASON, PLAN, REFLECT, VERIFY}:
      1. Extract template_str attribute
      2. Check cache: hash(template) -> cached optimized version?
         - YES: Replace template_str with cached version, continue
         - NO: Continue to optimization
      3. Load training data (from --dspy-training-data or session profiles)
      4. Convert template to DSPy Signature
      5. Run DSPy optimizer (LabeledFewShot, BootstrapFewShot, or MIPROv2)
      6. Extract optimized template from DSPy result
      7. Replace template_str attribute with optimized version
      8. Add metadata: ais.dspy_version, ais.dspy_optimizer, ais.dspy_score
      9. Cache result: hash(template) -> optimized version
  RETURN modified module
```

**Optimizer selection by level**:

| Optimization Level           | DSPy Optimizer   | Training Examples | Time     | Quality Gain |
|------------------------------|------------------|-------------------|----------|--------------|
| O2 (opt-in)                  | LabeledFewShot   | 5-10              | <1 min   | +10-15%      |
| O3 (default)                 | BootstrapFewShot | 10-50             | 5-10 min | +20-30%      |
| O3 (--dspy-optimizer=mipro)  | MIPROv2          | 50-200            | 15-30 min| +30-40%      |

**Template transformation example**:

Before optimization:
```mlir
%answer = ais.ask(%question) {
  template_str = "Answer concisely: {{question}}"
} : (!ais.value) -> !ais.value
```

After DSPy optimization (BootstrapFewShot with 3 examples):
```mlir
%answer = ais.ask(%question) {
  template_str = "Answer concisely: {{question}}\n\nExamples:\n1. Q: What is 2+2? A: 4\n2. Q: Capital of France? A: Paris\n3. Q: Speed of light? A: 299,792,458 m/s",
  ais.dspy_version = "3.1.3",
  ais.dspy_optimizer = "bootstrap_fewshot",
  ais.dspy_score = 0.87
} : (!ais.value) -> !ais.value
```

**Two granularities of optimization are possible**:

- **Per-node optimization**: Run MIPROv2 on each ASK/THINK/REASON node independently. Simpler, parallelizable, but misses cross-node interactions.
- **Graph-level optimization**: Convert the entire APXM graph to a DSPy Module, define an end-to-end metric, and optimize the pipeline as a whole. Captures cross-node interactions but more complex to implement.

### Rust/Python Architecture

Since DSPy is Python-only, the Rust MLIR pass calls Python via subprocess (not PyO3) for simplicity and isolation.

**File structure**:
```
crates/apxm-compiler/
  src/passes/
    dspy_optimize.rs        # Rust pass that calls Python bridge

crates/apxm-frontend/python/apxm/
  dspy_pass.py              # Python compiler pass implementation
  dspy_bridge.py            # Core DSPy optimization logic
```

**Rust side** (`dspy_optimize.rs`):
```rust
pub fn run_dspy_optimize(
    module: &mlir::ModuleOp,
    config: &DspyConfig,
) -> Result<()> {
    let llm_ops = extract_llm_operations(module);

    for op in llm_ops {
        let template = op.get_attr("template_str")?;
        let cache_key = hash_template(&template);

        if let Some(cached) = load_from_cache(&cache_key, &config.cache_dir) {
            op.set_attr("template_str", cached);
            continue;
        }

        let result = Command::new("python3")
            .arg("-m").arg("apxm.dspy_pass")
            .arg("optimize-template")
            .arg("--template").arg(&template)
            .arg("--training-data").arg(&config.training_data_path)
            .arg("--optimizer").arg(&config.optimizer)
            .arg("--target").arg(&config.target)
            .output()?;

        let optimized_template = parse_dspy_result(&result.stdout)?;

        op.set_attr("template_str", optimized_template.template);
        op.set_attr("ais.dspy_version", optimized_template.version);
        op.set_attr("ais.dspy_optimizer", config.optimizer);
        op.set_attr("ais.dspy_score", optimized_template.score);

        save_to_cache(&cache_key, &optimized_template, &config.cache_dir);
    }

    Ok(())
}
```

**Python side** (`dspy_pass.py`): Receives template + training data + optimizer choice via CLI args, runs DSPy optimization, and emits JSON result to stdout. Handles signature conversion (extracting `{{placeholder}}` patterns from APXM templates into DSPy signatures), optimizer dispatch, metric selection, and quality evaluation.

### Caching

**Hash-based template caching** avoids redundant optimization:

```
cache_key = sha256(template_str + optimizer + target + dspy_version)
cache_path = ~/.apxm/dspy_cache/{cache_key}.json
```

**Cache invalidation**:
- Template changes produce a new hash -- automatic re-optimization
- DSPy version upgrade changes the hash -- automatic re-optimization
- Training data changes require manual invalidation (`--dspy-cache-clear`)

**Effect**: First compile pays the full 5-20 minute cost. Subsequent compilations with unchanged templates resolve instantly from cache.

---

## Training Data Sources

### User-Provided Examples

JSON file with explicit input/output pairs:

```json
[
  {
    "inputs": {"question": "What is microservices?"},
    "output": "An architecture pattern where applications are composed of independent services..."
  }
]
```

Usage: `apxm compile graph.apxm -O3 --dspy --dspy-training-data examples.json`

### Execution Profiles (Auto PGO)

Execution sessions (`--emit-session`) produce `results.json` with per-node inputs, outputs, success status, latency, and token counts. A converter extracts successful ASK/THINK/REASON executions as DSPy training examples:

```python
def profile_to_training_data(session_dir: Path) -> List[Example]:
    profile = json.load((session_dir / "results.json").open())
    examples = []
    for node in profile["nodes"]:
        if not node["success"]:
            continue
        if node["op"] in ["ASK", "THINK", "REASON"]:
            examples.append(Example(
                inputs=node["inputs"],
                output=node["output"],
            ).with_inputs(**node["inputs"]))
    return examples
```

Workflow: Run N times with `--emit-session`, then compile with `--dspy=auto` to auto-detect profiles from `~/.apxm/sessions/`.

### Synthetic Generation

Use an LLM to generate training examples from a task description:

```python
def generate_synthetic_examples(task_description: str, num_examples: int = 20) -> List[Example]:
    generator = dspy.ChainOfThought("task_description -> question, answer")
    examples = []
    for i in range(num_examples):
        result = generator(task_description=task_description)
        examples.append(Example(inputs={"question": result.question}, output=result.answer))
    return examples
```

### Session-Based Learning (Runtime Feedback Loop)

After each execution session:
1. Parse `trace.ndjson` to extract `(node_id, inputs, outputs, success)`
2. Filter successful executions
3. Store as few-shot examples in `~/.apxm/examples/<op_name>/success_NNN.json`
4. Next compile automatically uses these as training data

---

## CLI Integration

### Proposed CLI Flags

```bash
# Enable DSPy optimization (requires O2 or O3)
apxm compile graph.apxm -O3 --dspy

# Specify training data
apxm compile graph.apxm -O3 --dspy --dspy-training-data examples.json

# Auto-detect from session profiles
apxm compile graph.apxm -O3 --dspy=auto

# Select optimizer
apxm compile graph.apxm -O3 --dspy --dspy-optimizer mipro_v2

# Specify target metric
apxm compile graph.apxm -O3 --dspy --dspy-metric quality

# Cache directory
apxm compile graph.apxm -O3 --dspy --dspy-cache ~/.apxm/dspy_cache

# Verbose mode
apxm compile graph.apxm -O3 --dspy --dspy-verbose
```

### Rust Integration Points

**In `apxm-cli`** (CompileArgs):
```rust
#[arg(long)]
dspy: bool,

#[arg(long)]
dspy_training_data: Option<PathBuf>,

#[arg(long, default_value = "bootstrap_fewshot")]
dspy_optimizer: String,

#[arg(long)]
dspy_metric: Option<String>,

#[arg(long)]
dspy_cache: Option<PathBuf>,

#[arg(long)]
dspy_verbose: bool,
```

**In `apxm-compiler` pass pipeline** (`pipeline.rs`):
```rust
if matches!(level, OptimizationLevel::O3) && dspy_config.is_some() {
    // Add DSPy pass before final canonicalization
    passes.insert(passes.len() - 2, "dspy-optimize".to_string());
}
```

---

## Metrics

DSPy provides minimal built-in metrics (`EM`, `F1`, `HotPotF1`). Users must define metrics as Python functions with the signature `metric(example, prediction, trace=None) -> bool | float`.

APXM defines four optimization targets, each with a corresponding metric:

| Target   | Goal                                      | Weight Balance               |
|----------|-------------------------------------------|------------------------------|
| quality  | Maximize answer correctness               | 100% quality                 |
| latency  | Minimize inference time, maintain quality  | 70% quality, 30% brevity    |
| tokens   | Minimize token usage, maintain quality     | 60% quality, 40% efficiency |
| balanced | Multi-objective balance                    | 50% quality, 30% tokens, 20% latency |

**Quality**: Semantic similarity (cosine similarity of embeddings) between predicted and expected output. Fallback to exact match when embeddings are unavailable.

**Latency**: Penalizes long outputs (more tokens = slower inference). `quality * 0.7 + (1.0 - token_penalty) * 0.3`.

**Tokens**: Penalizes output length normalized to 2k tokens. `quality * 0.6 + efficiency * 0.4`.

**Balanced**: Multi-objective combining all factors. `quality * 0.5 + token_score * 0.3 + latency_score * 0.2`.

**Evaluation pipeline**: DSPy's `Evaluate` class runs `num_threads` parallel workers, calling `program(**example.inputs())` and scoring with the metric. Aggregate score is `sum(scores) / len(devset) * 100`. No built-in thresholding -- the user decides what score is acceptable.

**APXM-specific metrics** (planned for `apxm/quality.py`):
- `EM` -- Exact match after normalization
- `F1` -- Token-level F1
- `embedding_similarity` -- Cosine similarity of sentence embeddings
- Per-node quality functions attached via graph metadata

---

## Gaps and Recommendations

### Gap 1: No Token Budget Enforcement

**Problem**: DSPy tracks tokens (`prompt_tokens`, `completion_tokens`) per LM call via `model.history` but never enforces limits. BootstrapFewShot runs until `max_rounds x len(trainset)` examples are processed. MIPROv2 runs for exactly `num_trials` with no early stopping on token budget.

**Recommendation**: Wrap the DSPy evaluator in APXM's `CostAwareOptimizer` that tracks cumulative token usage and raises `BudgetExceeded` when a configurable `max_tokens` limit is hit.

### Gap 2: No Dollar Cost Budgets

**Problem**: DSPy captures `response_cost` from litellm pricing (OpenAI and compatible providers) but never aggregates or enforces limits.

**Recommendation**: Add a `--dspy-max-cost` CLI flag. The Python bridge aggregates `sum(entry.get("cost", 0) for entry in model.history)` after each trial and aborts if the budget is exceeded.

### Gap 3: Minibatch Heuristic is Brittle

**Problem**: MIPROv2 evaluates on 35-example minibatches by default. Minibatch scores may not correlate with full validation scores, especially for skewed datasets.

**Recommendation**: Use stratified sampling to ensure minibatches are representative. Consider adaptive minibatch sizing based on score variance.

### Gap 4: No Multi-Objective Optimization

**Problem**: DSPy optimizes for a single scalar metric. No native Pareto optimization across competing objectives (accuracy vs. latency vs. cost).

**Recommendation**: APXM's `balanced` target provides a weighted sum approximation. For true multi-objective optimization, consider Optuna's `NSGAIISampler` as a drop-in replacement for TPE in MIPROv2.

### Gap 5: No Incremental Learning

**Problem**: Adding new training examples requires a full re-run of the optimization. No mechanism to warm-start from previous results.

**Recommendation**: Cache bootstrap traces in `~/.apxm/dspy_cache/`. When training data changes incrementally, reuse existing traces and only bootstrap new examples. Invalidate instruction candidates only when the signature or instructions change.

### Gap 6: No Built-In Quality Heuristics

**Problem**: Users must always supply a metric function. There is no default "is this a good answer?" heuristic.

**Recommendation**: APXM should provide a `quality.py` module with sensible defaults: exact match for short answers, F1 for longer text, embedding similarity for open-ended responses. These can be selected automatically based on output field descriptors.

### Gap 7: Token Usage Overhead

DSPy-optimized prompts add few-shot examples, increasing input tokens by 15-30%. Example: baseline 400 tokens/request becomes 520 tokens/request (+30%). At $3/MTok this is ~$0.0006/request.

**When it is worth it**: Quality improvements justify cost (35% accuracy gain >> 30% cost increase); high-value, user-facing, or long-running workflows.

**When it is not**: High-volume low-margin workflows, simple tasks already at 95%+ accuracy, token-constrained environments.

### Gap 8: Signature-Based Type System

DSPy signatures are typed input/output specs with descriptions. APXM should adopt this pattern for ASK node attributes:

```json
{
  "op": "ASK",
  "attributes": {
    "signature": {
      "inputs": {"question": {"type": "str", "desc": "..."}},
      "outputs": {"answer": {"type": "str", "desc": "often 1-5 words"}}
    },
    "instruction": "Answer questions with short factoid answers."
  }
}
```

This gives DSPy richer type information during optimization and aligns with GUARD operations for runtime assertion checks.

---

## Implementation Roadmap

### Phase 1: Research Integration (Current)

- [x] Understand DSPy internals (BootstrapFewShot, MIPROv2, metrics, token handling)
- [x] Design compiler pass architecture
- [ ] Create `apxm-frontend/python/apxm/quality.py` -- metric definitions (EM, F1, embedding similarity)
- [ ] Implement basic `dspy_pass.py` prototype

### Phase 2: Compile-Time Optimization

- [ ] Implement Rust pass (`dspy_optimize.rs`) with subprocess call to Python bridge
- [ ] Wire CLI flags (`--dspy`, `--dspy-training-data`, `--dspy-optimizer`, `--dspy-metric`) into compilation pipeline
- [ ] Add template caching system (`~/.apxm/dspy_cache/`)
- [ ] Store optimized prompts and metadata in `.apxmobj` artifact
- [ ] Add cost tracking to compilation diagnostics (`--emit-diagnostics`)

### Phase 3: Runtime Feedback Loop

- [ ] Capture quality scores in session trace (`trace.ndjson`)
- [ ] Extract successful examples to `~/.apxm/examples/`
- [ ] Auto-bootstrap from past sessions (`--dspy=auto`)
- [ ] Profile-to-training-data converter

### Phase 4: Advanced Features

- [ ] Multi-objective optimization (accuracy vs. cost vs. latency) via Optuna NSGA-II
- [ ] Incremental learning (update prompts without full re-optimization)
- [ ] A/B testing (deploy 2+ prompt variants, measure in production)
- [ ] Cost-aware optimizer with `--dspy-max-cost` budget enforcement
- [ ] Graph-level optimization (whole-pipeline DSPy Module)

---

## References

1. O. Khattab et al., "DSPy: Compiling Declarative Language Model Calls into Self-Improving Pipelines," *arXiv:2310.03714*, 2023
2. "Optimizing Instructions and Demonstrations for Multi-Stage Language Model Programs" (MIPROv2 paper)
3. Optuna -- Bayesian hyperparameter optimization: https://optuna.org/
4. DSPy GitHub: https://github.com/stanfordnlp/dspy
