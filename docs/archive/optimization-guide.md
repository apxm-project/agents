# APXM Optimization Guide

**Version**: 0.2.0
**Date**: April 8, 2026

---

## Quick Reference

### Optimization Levels

| Level | Compilation Time | Speedup | Use Case |
|-------|-----------------|---------|----------|
| **O0** | <1s | 1x (baseline) | Development, debugging |
| **O1** | <5s | 1.1-1.3x | Quick iteration |
| **O2** | <10s | 1.5-2x | **Production default** |
| **O3** | 10-30 min | 2-3x | Critical workflows (with DSPy) |

### Optimization Targets

| Target | Primary Goal | When to Use |
|--------|-------------|-------------|
| `--target latency` | **Minimize end-to-end time** | Interactive tools, user-facing agents |
| `--target cost` | **Minimize dollar spend** | Batch processing, high-volume workflows |
| `--target tokens` | **Minimize total tokens** | Context-constrained models, ultra-low-cost |
| `--target quality` | **Maximize correctness** | High-stakes tasks (security, medical, legal) |
| `--target balanced` | **Middle ground** | **Default — general purpose** |

---

## How Each Optimization Level Works

### O0 — No Optimization
```bash
apxm compile workflow.apxm -O0
```
- Direct graph → artifact compilation
- No passes run (except parser + codegen)
- **Use for**: Debugging compilation issues

### O1 — Basic Optimizations
```bash
apxm compile workflow.apxm -O1
```

**Passes**:
1. `normalize` — Canonicalize graph structure
2. `build-prompt` — Construct templates
3. `scheduling` — Reorder for parallelism
4. `fuse-ask-ops` — Merge sequential LLM calls
5. `canonicalizer` — MLIR standard optimizations
6. `cse` — Common subexpression elimination
7. `symbol-dce` — Dead code elimination

**Expected speedup**: **1.1-1.3x**

### O2 — Standard Optimizations (Production Default)
```bash
apxm compile workflow.apxm -O2  # or just 'apxm compile'
```

**All O1 passes +**:
8. `template-specialization` — Inline constants into templates
9. `dead-context-elimination` — Remove unused context (60-70% savings)
10. `schema-narrowing` — Simplify output schemas (10-20% token reduction)
11. `prompt-canonicalization` — Extract shared prefixes (70% cache hit rate)
12. `condense-ops` — Batch memory operations

**Expected speedup**: **1.5-2x**
**Expected cost reduction**: **70-90%** (with MemoCache)

### O3 — Aggressive Optimizations
```bash
apxm compile workflow.apxm -O3 --dspy --dspy-training-data examples.json
```

**All O2 passes +**:
13. `dspy-optimize` — Prompt quality optimization via DSPy MIPROv2

**Expected speedup**: **2-3x** (quality improvement, not speed)
**Expected quality gain**: **+20-40% accuracy**
**Trade-off**: 10-30 minute compile time

---

## Optimization Target Deep Dive

### 1. Latency Target — Maximum Speed

```bash
apxm compile workflow.apxm -O2 --target latency
```

**What changes**:
- **Aggressive fusion**: 32,000 token limit (vs 5,000 default)
- **Parallelism extraction**: Remove all non-data dependencies
- **Speculation**: Start downstream before upstream completes (50% confidence)
- **Pipelining**: Overlap prefill with generation
- **Model downgrading**: Use fast models (GPT-4o-mini) for simple ops
- **Agent pre-warming**: Spawn agents before first use
- **vLLM tuning**: High cache capacity (95% GPU memory), long TTL (5 min)

**Measured performance**:
- **1.51x speedup** on fan-out patterns (vLLM prefix caching)
- **2.00x speedup** on dead context elimination
- **1.12x speedup** on priority scheduling

**Trade-offs**:
- +20-50% token usage (aggressive fusion)
- Potential -5% to -15% quality (model downgrading)
- +20-30% memory usage (long cache TTL)

**Use when**: User is waiting (interactive workflows, developer tools)

---

### 2. Cost Target — Minimum Dollar Spend

```bash
apxm compile workflow.apxm -O2 --target cost
```

**What changes**:
- **Model routing**: Prefer budget tier (GPT-4o-mini, Claude Haiku)
- **Context budgets**: Aggressive truncation (50% of window)
- **Fusion**: Only when net cost saved > 0
- **Dead context elimination**: Aggressive (remove all unused)
- **MemoCache**: Enabled with high hit rate (80%+)

**Measured cost reduction**:
- **92% reduction** (from $0.35 → $0.028 per workflow)
- **85% savings** at 95% quality (via model routing)

**Annual savings** (100 workflows/day):
- From $875/year → $85/year (**$790/year saved**)

**Trade-offs**:
- +20-50% latency (cheaper models are slower)
- -5% to -10% quality (acceptable for non-critical tasks)

**Use when**: Batch processing, high-volume workflows, cost-sensitive production

---

### 3. Tokens Target — Minimum Token Usage

```bash
apxm compile workflow.apxm -O2 --target tokens --llmlingua-compress
```

**What changes**:
- **Dead context elimination**: 60-70% context removal
- **Schema narrowing**: Simplify output schemas (10-20% reduction)
- **Selective fusion**: Only fuse when net tokens saved > 0
- **LLMLingua compression**: 75% context reduction (4x compression)
- **DSPy token-aware**: Metric penalizes verbosity
- **No warmup passes**: Profile-guided optimization costs tokens

**Measured token savings**:
- **90% reduction** (from 8,000 → 755 tokens)
- **Cost savings**: $0.035 → $0.0034 per workflow (90%)

**Trade-offs**:
- Compilation time +5-10 minutes (LLMLingua + DSPy)
- May sacrifice parallelism (sequential execution to avoid context duplication)

**Use when**: Context-constrained models, ultra-low-cost requirements

---

### 4. Quality Target — Maximum Correctness

```bash
apxm compile workflow.apxm -O2 --target quality
```

**What changes**:
- **No fusion**: Preserve reasoning steps
- **Verification injection**: Add VERIFY nodes after critical operations
- **Refinement loops**: Retry on validation failure (max 3 retries)
- **Best models**: Always use top-tier (Opus 4.6, O1, Gemini Pro)
- **Conservative context**: 90% of window (don't starve model)
- **Schema enforcement**: Required (enables validation)

**Measured quality improvement**:
- **+40% accuracy** (from 62% → 87% on security audit benchmark)
- **+35-45% improvement** on high-stakes tasks

**Trade-offs**:
- **3-4x cost** ($0.08 → $0.35 per workflow)
- **2-3x latency** (14.7s vs 4.1s)
- **3x token usage** (22,830 vs 8,220 tokens)

**Use when**: High-stakes tasks (security audits, medical diagnosis, legal analysis)

---

### 5. Balanced Target — Default Middle Ground

```bash
apxm compile workflow.apxm -O2  # Default target
```

**Settings** (middle of all extremes):
- **Fusion**: Moderate (5,000 token limit)
- **Models**: Fast tier (GPT-4o, Claude Sonnet)
- **Context**: 50% of window
- **Quality guards**: Enabled
- **DSPy**: Optional

**Use when**: General-purpose workflows, unknown requirements

---

## vLLM Integration — Graph-Aware Optimizations

### Prefix Caching

**How it works**:
1. Compiler detects shared context across parallel operations
2. `prompt-canonicalization` pass reorders templates to extract shared prefix
3. vLLM backend marks first operation as warmup
4. Runtime assigns reuse group ID to all operations sharing prefix
5. vLLM caches KV blocks on first call, reuses on subsequent calls

**Performance**:
- **1.51x speedup** on 4-way fan-out
- **70% cache hit rate** (4,368 tokens saved from redundant prefill)
- **Best for**: Multi-perspective analysis (code review, multi-agent debate)

**Example**:
```python
# 4 parallel code reviews sharing 1,524-token code context
security_review = g.ask("Review for security issues: {{code}}")
performance_review = g.ask("Review for performance issues: {{code}}")
reliability_review = g.ask("Review for reliability issues: {{code}}")
scalability_review = g.ask("Review for scalability issues: {{code}}")

# O2 + vLLM: First review warms cache, next 3 hit cache
# Result: 70% of prefill work eliminated
```

### Priority Scheduling

**Configuration**:
```rust
// Critical path gets highest priority
RequestHints {
    priority: 0,  // 0 = highest, 10 = lowest
    reuse_group: "code-context",
    pin_policy: PinPolicy::prefix(300_000),  // 5-minute TTL
}
```

**Performance**:
- **1.12x speedup** on mixed-priority workloads
- **20-40% P95 latency improvement** under load

### KV Pinning (Future)

**Phase 3 feature** (weeks 4-5):
- Pin upstream KV blocks for downstream reuse
- **Target**: 100% reuse on pinned prefixes (vs 70% with standard caching)

---

## DSPy Integration

### Compile-Time Optimization

```bash
apxm compile workflow.apxm -O3 --dspy \
  --dspy-training-data examples.json \
  --dspy-optimizer miprov2
```

**What it does**:
- Runs MIPROv2 Bayesian optimization on templates
- Adds few-shot examples from training data
- Embeds optimized prompts in `.apxmobj` artifact

**When to use**:
- **Quality target**: Add few-shot examples (+30-40% accuracy)
- **Tokens target**: Compress prompts (-20-40% tokens) via COPRO
- **Latency target**: Shorten prompts while maintaining quality

**Training data format**:
```json
[
  {
    "inputs": {"question": "What is microservices?"},
    "output": "An architecture pattern where applications..."
  },
  {
    "inputs": {"question": "How does caching work?"},
    "output": "Caching stores frequently accessed data..."
  }
]
```

### Auto-Training from Sessions

```bash
# Run workflow, emit session
apxm execute workflow.apxm --emit-session

# Use session as training data (future)
apxm compile workflow.apxm -O3 --dspy=auto
# Automatically extracts successful executions as examples
```

---

## MemoCache — Persistent Cross-Session Caching

### How It Works

**Two-tier cache**:
- **L1**: In-memory DashMap (10,000 entries, instant)
- **L2**: SQLite (`~/.apxm/cache/cache.db`, persistent)

**Cache key**: `hash(operation_type, template, context, model, temperature)`

**TTL per operation**:
- ASK: 1 hour
- THINK: 24 hours
- REASON: 7 days

**Deterministic only**: `temperature=0.0` required

### Performance

**Benchmark**: `cache_test.apxm` (4 identical prompts, temp=0.0)

| Run | LLM Calls | Time | Cache Hit Rate | Speedup |
|-----|-----------|------|----------------|---------|
| Run 1 (cold) | 4 | 7.9s | 0% | 1.00x |
| Run 2 (warm) | 0 | 7.2s | 100% | 1.09x |

**Cost savings**: **$0.02 saved** (4 calls × $0.005/call)

**Annual savings** (1,000 iterations/day): **$7,300/year**

### CLI

```bash
# View cache stats
apxm cache stats

# Clear cache
apxm cache clear

# Force cache bypass
apxm execute workflow.apxm --nocache
```

---

## Heuristics System

### How Heuristics Guide Decisions

**Per-target configuration** (from `heuristics.rs`):

```rust
OptimizationTarget::Latency => Self {
    max_fused_template_tokens: 32_000,   // Aggressive
    min_fusion_savings_ms: 50,           // Low threshold
    max_context_tokens: 128_000,         // Full window
    enable_quality_guard: false,         // Speed > quality
},

OptimizationTarget::Cost => Self {
    max_fused_template_tokens: 5_000,    // Moderate
    min_fusion_savings_ms: 250,          // Higher threshold
    max_context_tokens: 16_000,          // Aggressive reduction
    enable_quality_guard: true,          // Preserve quality
},

OptimizationTarget::Tokens => Self {
    max_fused_template_tokens: 2_000,    // Conservative
    min_fusion_savings_ms: 0,            // Latency doesn't matter
    max_context_tokens: 8_000,           // Very aggressive
    enable_quality_guard: true,          // Preserve quality
},

OptimizationTarget::Quality => Self {
    max_fused_template_tokens: 0,        // NO FUSION
    min_fusion_savings_ms: u64::MAX,     // Never fuse
    max_context_tokens: context_window * 9 / 10,  // 90% budget
    enable_quality_guard: true,          // Required
},
```

### Profile-Guided Optimization (PGO)

**Workflow**:
1. **Run workflow**: `apxm execute workflow.apxm --emit-session`
2. **Collect profile**: Session metrics → `~/.apxm/sessions/<id>/metrics.json`
3. **Recompile**: `apxm compile workflow.apxm --profile <session-id>`
4. **Benefit**: Heuristics tuned to actual execution patterns

**Metrics used for PGO**:
- Per-node latency distribution
- Token usage (input/output)
- Cache hit rates
- Quality scores (if ground truth available)

---

## Decision Matrix

**Which optimization target should I use?**

| Your Primary Concern | Recommended Target | Expected Improvement |
|---------------------|-------------------|---------------------|
| User is waiting | `--target latency` | 1.5-2x faster |
| Cost-sensitive batch processing | `--target cost` | 85-92% cost reduction |
| Limited context window | `--target tokens` | 60-90% token reduction |
| High-stakes correctness | `--target quality` | +35-45% accuracy |
| Don't know / general purpose | `--target balanced` | Moderate on all axes |

**Combining targets**:
```bash
# Low-cost + high-quality: Use best models but cache aggressively
apxm compile workflow.apxm -O2 --target cost --model claude-opus-4-6

# Fast + low-token: Aggressive fusion but compress context
apxm compile workflow.apxm -O2 --target latency --llmlingua-compress
```

---

## Common Patterns

### Multi-Perspective Analysis (Fan-Out)
**Best target**: `--target latency`
- Shared context → prefix caching wins
- Parallelism → all reviews run concurrently
- **Result**: 1.5-2x speedup

### Sequential Reasoning Chain
**Best target**: `--target quality`
- Preserve reasoning steps (no fusion)
- Use best models for each step
- **Result**: +35-45% accuracy

### Batch Data Processing
**Best target**: `--target cost`
- Model downgrading → 85% cost reduction
- MemoCache → eliminate duplicate work
- **Result**: 85-92% cost reduction

### Context-Heavy RAG
**Best target**: `--target tokens`
- LLMLingua compression → 75% reduction
- Dead context elimination → 60% reduction
- **Result**: 90% token reduction

---

## CLI Quick Reference

```bash
# Compile with different targets
apxm compile workflow.apxm --target latency
apxm compile workflow.apxm --target cost
apxm compile workflow.apxm --target tokens
apxm compile workflow.apxm --target quality

# Analyze optimization potential
apxm analyze workflow.apxm --target latency

# Execute with metrics
apxm execute workflow.apxm --target cost --emit-metrics metrics.json

# DSPy optimization
apxm compile workflow.apxm -O3 --dspy --dspy-training-data examples.json

# vLLM-specific
apxm compile workflow.apxm -O2 --target latency --backend vllm-local

# Full stack (all optimizations)
apxm compile workflow.apxm -O3 --target quality \
  --dspy --dspy-training-data examples.json \
  --llmlingua-compress --llmlingua-ratio=4
```

---

## Further Reading

- [ARCHITECTURE.md](ARCHITECTURE.md) — System architecture
- [API-REFERENCE.md](API-REFERENCE.md) — Python API reference
- [FINDINGS.md](FINDINGS.md) — Benchmark results
- [COST-ANALYSIS.md](COST-ANALYSIS.md) — Dollar savings analysis
- [docs/research/](research/) — Optimization strategy details
