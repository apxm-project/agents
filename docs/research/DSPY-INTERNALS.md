# DSPy Internals Research — Optimization Mechanics, Metrics, Token Handling

**Research Date**: 2026-04-08
**DSPy Version**: Latest (installed from PyPI)

## Executive Summary

DSPy is a **prompt optimization framework** that treats prompts as learnable parameters. Key findings:

1. **BootstrapFewShot** generates demonstrations by running a "teacher" model and filtering by metric success
2. **MIPROv2** uses Bayesian optimization (Optuna + TPE sampler) to search the space of {instructions × few-shot sets}
3. **Metrics** are user-defined boolean/float functions; DSPy has NO built-in quality heuristics
4. **Token tracking** exists but **NO token budget enforcement** — optimization can run indefinitely
5. **Cost tracking** captures `response_cost` from LM providers but doesn't enforce limits
6. **No dollar budgets** — users must manually stop optimization or set `num_trials`

---

## 1. BootstrapFewShot — How It Works

**File**: `dspy/teleprompt/bootstrap.py` (270 lines)

### Core Algorithm

```python
class BootstrapFewShot(Teleprompter):
    def __init__(self, metric=None, metric_threshold=None, max_bootstrapped_demos=4,
                 max_labeled_demos=16, max_rounds=1):
        # metric: callable(example, prediction, trace) -> bool or float
        # metric_threshold: if metric returns float, check >= threshold
```

**Steps**:

1. **Teacher Setup** (lines 93-103):
   - Clone the student program as a "teacher"
   - Prime teacher with `LabeledFewShot(k=max_labeled_demos)` — just raw labeled examples

2. **Bootstrap Loop** (lines 145-166):
   - For each training example, up to `max_rounds` attempts:
     - Run teacher(**example.inputs()) with `temperature=1.0` (bypass cache)
     - Capture full execution trace (every predictor call)
     - Evaluate with `metric(example, prediction, trace)`
     - If `success`, extract demos from trace

3. **Demo Extraction** (lines 220-252):
   - For each step in trace: `(predictor, inputs, outputs)`
   - Create `dspy.Example(augmented=True, **inputs, **outputs)`
   - Store per predictor: `name2traces[predictor_name].append(demo)`

4. **Train Student** (lines 256-269):
   - For each predictor, combine:
     - Up to `max_bootstrapped_demos` from successful traces
     - Fill remaining slots with raw labeled examples
   - Assign to `predictor.demos`

### Key Decisions

- **No optimization** — just success/failure filtering
- **Temperature=1.0** for diversity (line 188)
- **Multiple traces per predictor**: If one example triggers predictor N times, randomly sample 50/50 from first N-1 or last trace (lines 247-251)
- **Validation set**: Unbootstrapped examples become valset (line 170)

---

## 2. MIPROv2 — Bayesian Optimization of {Instructions × Few-Shot Sets}

**File**: `dspy/teleprompt/mipro_optimizer_v2.py` (854 lines)

### Three-Step Process

```python
def compile(self, student, trainset, valset=None, num_trials=None, auto="light"):
    # Step 1: Bootstrap few-shot examples
    demo_candidates = self._bootstrap_fewshot_examples(...)

    # Step 2: Propose instruction candidates
    instruction_candidates = self._propose_instructions(...)

    # Step 3: Find optimal {instruction, demos} via Bayesian search
    best_program = self._optimize_prompt_parameters(...)
```

#### Step 1: Bootstrap (lines 389-440)

- Calls `create_n_fewshot_demo_sets()` from `utils.py`
- Creates **N candidate sets** of few-shot examples per predictor
- Uses `BootstrapFewShot` with shuffled trainset
- Returns `demo_candidates[predictor_idx][candidate_idx] = [demos]`

#### Step 2: Propose Instructions (lines 442-493)

- Uses `GroundedProposer` (from `dspy/propose/grounded_proposer.py`)
- **Inputs to proposer**:
  - Dataset summary (10 LM calls to summarize data)
  - Program code (extracted via `inspect.getsource()`)
  - Few-shot examples from Step 1
  - Random prompting tip (e.g., "Be creative!", "Keep it simple")
- **Outputs**: `instruction_candidates[predictor_idx] = [N instruction strings]`
- **LM calls**: ~10 + (N × num_predictors) + num_predictors (program-aware)

#### Step 3: Bayesian Optimization (lines 495-680)

**Search Space**:
- For each predictor: `{instruction_idx: 0..N, demos_idx: 0..M}`
- Total combinations: `N^predictors × M^predictors`

**Optimizer**: Optuna with TPE sampler (Tree-structured Parzen Estimator)

**Objective Function** (lines 545-644):
```python
def objective(trial):
    # 1. Select instruction + demos for each predictor
    for i, predictor in enumerate(program.predictors()):
        instruction_idx = trial.suggest_categorical(f"{i}_predictor_instruction", ...)
        demos_idx = trial.suggest_categorical(f"{i}_predictor_demos", ...)

    # 2. Evaluate on minibatch or full valset
    score = evaluate(candidate_program, devset=minibatch)

    # 3. Every N trials, do full eval on best minibatch candidate
    if minibatch and (trial_num % minibatch_full_eval_steps == 0):
        full_score = evaluate(best_program, devset=valset)
        return full_score

    return score
```

**Minibatch Strategy** (lines 549-642):
- Default: Evaluate on `minibatch_size=35` examples (fast)
- Every `minibatch_full_eval_steps=5` trials: Full eval on entire valset
- **Cost savings**: `35 × num_trials + len(valset) × (num_trials/5)` instead of `len(valset) × num_trials`

**Auto Modes** (lines 33-37):
```python
AUTO_RUN_SETTINGS = {
    "light":  {"n": 6,  "val_size": 100},   # 6 candidates, 100 examples
    "medium": {"n": 12, "val_size": 300},   # 12 candidates, 300 examples
    "heavy":  {"n": 18, "val_size": 1000},  # 18 candidates, 1000 examples
}
```

**Trial Count Formula** (line 273):
```python
num_trials = max(2 * num_vars * log2(num_candidates), 1.5 * num_candidates)
# For light mode (n=6, 1 predictor): max(2*2*log2(6), 1.5*6) = max(10.3, 9) ≈ 10 trials
```

---

## 3. How DSPy Measures Quality

**File**: `dspy/evaluate/evaluate.py` (400 lines)

### Metric Functions

DSPy provides **NO built-in heuristics** — users must define metrics.

**Built-in metrics** (`dspy/evaluate/metrics.py`):
- `EM(prediction, answers)` — Exact Match after normalization
- `F1(prediction, answers)` — Token-level F1
- `HotPotF1(prediction, answers)` — F1 with special yes/no handling

**User metrics** are arbitrary Python functions:
```python
def my_metric(example, prediction, trace=None):
    # example: dspy.Example with ground truth
    # prediction: dspy.Prediction with model outputs
    # trace: list of (predictor, inputs, outputs) tuples
    return True  # or False, or 0.0-1.0
```

### Evaluation Pipeline

**Class**: `Evaluate` (lines 64-400)

```python
evaluator = Evaluate(
    devset=validation_set,
    metric=my_metric,
    num_threads=12,          # Parallel evaluation
    max_errors=10,           # Stop after N errors
    display_progress=True,   # tqdm progress bar
)

result = evaluator(program)
# result.score: float (e.g., 67.30)
# result.results: [(example, prediction, score), ...]
```

**Execution** (lines 170-182):
- Uses `ParallelExecutor` (num_threads workers)
- For each example: `prediction = program(**example.inputs())`
- For each prediction: `score = metric(example, prediction)`
- Aggregate: `sum(scores) / len(devset) * 100`

**No thresholding** — DSPy returns raw average, user decides if it's "good enough"

---

## 4. Token Budget Handling

**Tracking**: YES
**Enforcement**: NO

### Token Tracking (base_lm.py lines 52-82)

```python
class BaseLM:
    def _process_lm_response(self, response, ...):
        entry = {
            "prompt": prompt,
            "messages": messages,
            "response": response,
            "outputs": outputs,
            "usage": dict(response.usage),  # prompt_tokens, completion_tokens
            "cost": getattr(response, "_hidden_params", {}).get("response_cost"),
            "timestamp": ...,
            "uuid": ...,
        }
        self.update_history(entry)
```

**Usage extraction** (`utils.py` lines 270-291):
```python
def get_token_usage(model) -> tuple[int, int]:
    input_tokens = []
    output_tokens = []
    for interaction in model.history:
        usage = interaction.get("usage", {})
        input_tokens.append(usage.get("prompt_tokens", 0))
        output_tokens.append(usage.get("completion_tokens", 0))
    return sum(input_tokens), sum(output_tokens)
```

### No Budget Limits

- **BootstrapFewShot**: Runs until `max_rounds × len(trainset)` examples processed
- **MIPROv2**: Runs for exactly `num_trials` (no early stopping on token budget)
- **max_errors**: Only limits consecutive errors, not cost

**User responsibility**: Monitor `model.history` and stop manually

---

## 5. Cost Tracking

**File**: `dspy/clients/base_lm.py` (line 72)

```python
"cost": getattr(response, "_hidden_params", {}).get("response_cost")
```

**Provider support**:
- OpenAI: Sets `response_cost` via litellm pricing
- Other providers: May or may not populate `_hidden_params`

**Aggregation**: None built-in — user must sum:
```python
total_cost = sum(entry.get("cost", 0) for entry in model.history)
```

**No budget enforcement** — DSPy never checks cost limits

---

## 6. Key Patterns APXM Should Adopt

### 6.1 Signature-Based Type System

**DSPy signatures** are typed input/output specs:
```python
class QA(dspy.Signature):
    """Answer questions with short factoid answers."""
    question = dspy.InputField()
    answer = dspy.OutputField(desc="often between 1 and 5 words")
```

**APXM equivalent**: ASK node attributes could use this:
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

### 6.2 Metric-Driven Optimization

**DSPy**: User supplies `metric(example, prediction, trace) -> float`
**APXM**: Could define quality functions per node:
```python
def quality_fn(node_output, ground_truth):
    return F1(node_output["answer"], ground_truth["answer"])
```

At **compile time**, APXM could:
1. Run MIPROv2 on each ASK node independently
2. Store optimized prompts in artifact
3. Use quality metrics as heuristic feedback for FuseReasoning pass

### 6.3 Few-Shot Example Selection

**DSPy BootstrapFewShot**: Mines successful traces from teacher runs
**APXM runtime**: Could save successful ASK node executions to `~/.apxm/examples/`:
```text
~/.apxm/examples/
  ├── ASK_code_review/
  │   ├── success_001.json  # {"inputs": {...}, "outputs": {...}, "score": 0.95}
  │   └── success_002.json
  └── ASK_summarize/
      └── success_001.json
```

Then at **next compile**, use these as few-shot demos for similar nodes.

### 6.4 Instruction Optimization

**DSPy GroundedProposer**: Uses dataset summary + program code + tips to generate N instruction candidates
**APXM**: Could generate instruction variants at compile time:
```rust
// In apxm-compiler/src/passes/optimize_prompts.rs
pub fn optimize_ask_prompt(
    node: &AskNode,
    trainset: &[Example],
    metric: &dyn Metric,
) -> String {
    let proposer = GroundedProposer::new();
    let candidates = proposer.propose(
        dataset_summary(&trainset),
        node.signature(),
        Some(TIPS["simple"]),
    );

    // Evaluate each candidate on trainset
    let best = candidates.into_iter()
        .map(|prompt| (prompt, evaluate(&prompt, trainset, metric)))
        .max_by_key(|(_, score)| score)
        .unwrap().0;

    best
}
```

### 6.5 Assertion-Based Quality Guards

**DSPy Suggest** (runtime assertions):
```python
dspy.Suggest(len(answer.split()) <= 5, "Answer must be at most 5 words")
```

**APXM equivalent**: GUARD operation (already in Phase 1 ISA):
```json
{
  "op": "GUARD",
  "attributes": {
    "condition": "len(output.answer.split()) <= 5",
    "retry_on_fail": true,
    "max_retries": 3
  }
}
```

---

## 7. Integration Design — How APXM Should Use DSPy

### At Compile Time (Offline Optimization)

**Goal**: Pre-optimize prompts before deployment

**Approach 1: Per-Node Optimization**
```python
# In apxm-compiler (Python bridge)
from dspy import MIPROv2, Evaluate
from dspy.evaluate.metrics import F1

class AskNodeOptimizer:
    def optimize(self, node: AskNode, trainset: list[Example]) -> str:
        # Wrap ASK node as DSPy module
        dspy_program = self._to_dspy_module(node)

        # Run MIPROv2
        optimizer = MIPROv2(
            metric=lambda ex, pred: F1(pred.answer, ex.answer),
            auto="light",  # 6 candidates, ~10 trials
        )

        optimized = optimizer.compile(dspy_program, trainset=trainset)

        # Extract optimized prompt
        return optimized.predictors()[0].signature.instructions
```

**Approach 2: Graph-Level Optimization**
```python
# Optimize entire graph as DSPy pipeline
class GraphOptimizer:
    def optimize(self, graph: Graph, trainset: list[Example]) -> Graph:
        # Convert APXM graph -> DSPy Module
        dspy_program = self._graph_to_dspy(graph)

        # Define end-to-end metric
        def metric(example, prediction, trace):
            # Check if final output matches expected
            return prediction.final_answer == example.answer

        # Run optimization
        optimizer = MIPROv2(metric=metric, auto="medium")
        optimized = optimizer.compile(dspy_program, trainset=trainset)

        # Convert back: DSPy Module -> APXM graph
        return self._dspy_to_graph(optimized)
```

**Storage**: Optimized prompts stored in `.apxmobj` artifact:
```rust
// apxm-artifact/src/lib.rs
pub struct OptimizedAsk {
    node_id: u32,
    original_prompt: String,
    optimized_prompt: String,
    few_shot_examples: Vec<Example>,
    optimization_score: f64,
}
```

### At Runtime (Adaptive Refinement)

**Goal**: Use execution history to improve prompts dynamically

**Approach: Session-Based Learning**
```python
# After each session
from apxm.quality import refine_from_session

session_dir = "~/.apxm/sessions/exec-12345/"
refine_from_session(session_dir)
```

**How it works**:
1. Parse `trace.ndjson` to extract (node_id, inputs, outputs, success)
2. Filter successful executions
3. Store as few-shot examples in `~/.apxm/examples/`
4. Next compile: Use these examples in BootstrapFewShot

**Quality Feedback Loop**:
```python
# apxm-frontend/python/apxm/quality.py
def compute_quality_score(node_output: dict, expected: dict) -> float:
    # Use DSPy metrics
    from dspy.evaluate.metrics import F1, EM

    if "answer" in node_output:
        return F1(node_output["answer"], expected["answer"])

    # Fallback: embedding similarity
    return embedding_similarity(node_output, expected)
```

### Cost-Aware Optimization

**DSPy lacks this** — APXM should add:

```python
class CostAwareOptimizer:
    def __init__(self, max_cost_usd: float = 10.0):
        self.max_cost = max_cost_usd
        self.spent = 0.0

    def compile(self, program, trainset, metric):
        optimizer = MIPROv2(metric=metric, auto="light")

        # Wrap evaluator to track cost
        original_eval = optimizer.evaluate
        def cost_tracked_eval(*args, **kwargs):
            result = original_eval(*args, **kwargs)
            self.spent += self._extract_cost(result)
            if self.spent >= self.max_cost:
                raise BudgetExceeded(f"Spent ${self.spent:.2f}")
            return result

        optimizer.evaluate = cost_tracked_eval
        return optimizer.compile(program, trainset=trainset)
```

**Store cost in artifact**:
```rust
pub struct OptimizationMetadata {
    total_cost_usd: f64,
    num_trials: u32,
    best_score: f64,
    timestamp: String,
}
```

---

## 8. Concrete Example: Optimizing ASK Node

**Scenario**: APXM graph with ASK node for code review

**Step 1: Training Data**
```json
[
  {
    "inputs": {"code": "def foo():\n  x=1\n  return x"},
    "expected_output": {"review": "Missing type hints, inconsistent spacing"}
  },
  ...
]
```

**Step 2: Run MIPROv2**
```python
import dspy
from dspy import MIPROv2
from dspy.evaluate.metrics import F1

# Define signature
class CodeReview(dspy.Signature):
    """Review Python code for quality issues."""
    code = dspy.InputField()
    review = dspy.OutputField(desc="list of issues found")

# Wrap as DSPy module
program = dspy.Predict(CodeReview)

# Define metric
def metric(example, prediction):
    # Check if key issues are mentioned
    return F1(prediction.review, example.expected_output["review"])

# Configure LM
dspy.configure(lm=dspy.LM("openai/gpt-4o-mini"))

# Optimize
optimizer = MIPROv2(metric=metric, auto="light")
optimized = optimizer.compile(program, trainset=training_data)

# Extract optimized prompt
print(optimized.predictors()[0].signature.instructions)
# Output: "Review Python code for quality issues. Focus on type safety,
#          formatting, and common bugs. Be concise but thorough."
```

**Step 3: Store in APXM Artifact**
```json
{
  "op": "ASK",
  "attributes": {
    "prompt": "Review Python code for quality issues. Focus on type safety...",
    "few_shot_examples": [
      {"code": "def foo(): ...", "review": "..."},
      ...
    ],
    "optimization_metadata": {
      "score": 0.87,
      "cost_usd": 0.42,
      "timestamp": "2026-04-08T10:30:00Z"
    }
  }
}
```

**Step 4: Runtime Execution**
- ASK handler reads optimized prompt from artifact
- Injects few-shot examples into LM context
- Runs inference
- Logs quality score to `metrics.json`

---

## 9. Limitations & Gaps in DSPy

### 9.1 No Token Budget Enforcement
- **Problem**: Optimization can run indefinitely, burning tokens
- **APXM fix**: Add `max_tokens` param to MIPROv2, track cumulative usage

### 9.2 No Cost Budgets
- **Problem**: No `max_cost_usd` param
- **APXM fix**: Wrap evaluator to track cost, raise on budget exceeded

### 9.3 Minibatch Heuristic is Brittle
- **Problem**: Minibatch score may not correlate with full eval score
- **APXM fix**: Use stratified sampling, ensure minibatch is representative

### 9.4 No Multi-Objective Optimization
- **Problem**: Can only optimize for one metric (e.g., accuracy)
- **APXM fix**: Use Pareto optimization for {accuracy, latency, cost}

### 9.5 No Incremental Learning
- **Problem**: Must re-run full optimization if trainset changes
- **APXM fix**: Cache bootstrap traces, reuse when adding new examples

### 9.6 No Quality Heuristics
- **Problem**: User must define metrics manually
- **APXM opportunity**: Provide built-in heuristics via `apxm/quality.py`

---

## 10. Recommended APXM Roadmap

### Phase 1: Research Integration (Current)
- [x] Understand DSPy internals
- [ ] Create `apxm-frontend/python/apxm/quality.py` module
- [ ] Implement basic metrics (EM, F1, embedding similarity)

### Phase 2: Compile-Time Optimization
- [ ] Add `--optimize` flag to `apxm compile`
- [ ] Wrap MIPROv2 in Rust (via PyO3 or subprocess)
- [ ] Store optimized prompts in artifact metadata
- [ ] Add cost tracking to compilation logs

### Phase 3: Runtime Feedback Loop
- [ ] Capture quality scores in session trace
- [ ] Extract successful examples to `~/.apxm/examples/`
- [ ] Auto-bootstrap from past sessions

### Phase 4: Advanced Features
- [ ] Multi-objective optimization (accuracy vs. cost vs. latency)
- [ ] Incremental learning (update prompts without full recompilation)
- [ ] A/B testing (deploy 2+ prompt variants, measure in production)

---

## 11. Files to Create

```
crates/apxm-frontend/python/apxm/
  ├── quality.py          # Metric definitions (EM, F1, embeddings)
  ├── optimize.py         # DSPy integration (MIPROv2, BootstrapFewShot)
  └── feedback.py         # Session-based learning

docs/guides/
  └── prompt-optimization.md  # User guide for --optimize flag

crates/apxm-compiler/src/passes/
  └── optimize_prompts.rs     # Compile-time optimization pass
```

---

## References

- DSPy GitHub: https://github.com/stanfordnlp/dspy
- DSPy Paper: "DSPy: Compiling Declarative Language Model Calls into Self-Improving Pipelines"
- MIPROv2 Paper: "Optimizing Instructions and Demonstrations for Multi-Stage Language Model Programs"
- Optuna (Bayesian optimizer): https://optuna.org/

---

**Next Steps**: Implement `apxm/quality.py` with DSPy metrics, then design `apxm compile --optimize` flow.
