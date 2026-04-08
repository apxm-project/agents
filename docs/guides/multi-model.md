# Multi-Model Routing

APXM supports per-node model and backend selection, enabling cost/quality/latency trade-offs within a single workflow. This guide covers routing strategies, configuration, and best practices.

---

## Why Multi-Model?

Different tasks have different requirements:

| Task | Requirement | Optimal Model |
|------|-------------|---------------|
| **Triage** | Speed, low cost | gpt-3.5-turbo, claude-haiku |
| **Deep analysis** | Quality, reasoning | gpt-4, claude-opus |
| **Structured extraction** | Privacy, local | llama-3-70b (local) |
| **Formatting** | Speed, template | gpt-3.5-turbo |

**Single-model problem:**
- Use cheap model → poor quality on complex tasks
- Use expensive model → high cost on simple tasks

**Multi-model solution:**
Route each operation to the best model for the job.

---

## Basic Usage

### Per-Node Model Selection

```python
from apxm import compile, GraphRecorder


@compile()
def support_ticket(g: GraphRecorder, ticket_text: str):
    g.param("ticket_text", "str")

    # Fast, cheap model for triage
    triage = g.ask(
        "Classify ticket priority: {ticket_text}",
        model="gpt-3.5-turbo"  # Fast model
    )

    # Powerful model for analysis
    solution = g.think(
        "Generate detailed solution: {ticket_text}\nTriage: {triage}",
        model="gpt-4"  # Quality model
    )

    # Fast model for formatting
    response = g.ask(
        "Format customer email: {solution}",
        model="gpt-3.5-turbo"  # Fast model
    )

    g.done(response)
```

### Per-Node Backend Selection

```python
from apxm import compile, GraphRecorder


@compile()
def privacy_workflow(g: GraphRecorder, user_data: str):
    g.param("user_data", "str")

    # Sensitive data → local model
    extracted = g.think(
        "Extract PII: {user_data}",
        backend="ollama",  # Local backend
        model="llama-3-70b"
    )

    # Public data → cloud model (faster)
    summary = g.ask(
        "Summarize (no PII): {extracted}",
        backend="openai",
        model="gpt-4"
    )

    g.done(summary)
```

---

## Routing Strategies

### 1. Fast-Slow-Fast Pattern

Common pattern: cheap models for setup/cleanup, expensive for core logic.

```python
@compile()
def research(g: GraphRecorder, topic: str):
    g.param("topic", "str")

    # Fast: Initial planning (gpt-3.5-turbo)
    outline = g.ask(
        "Create research outline: {topic}",
        model="gpt-3.5-turbo"
    )

    # Slow: Deep research (gpt-4)
    findings = g.think(
        "Deep research: {outline}",
        model="gpt-4"
    )

    # Fast: Format output (gpt-3.5-turbo)
    report = g.ask(
        "Format as executive summary: {findings}",
        model="gpt-3.5-turbo"
    )

    g.done(report)
```

**Cost savings:** ~60% vs all gpt-4
**Latency:** Similar (fast models are <500ms)

### 2. Parallel Multi-Model Fan-Out

Different models analyze in parallel, then synthesize.

```python
@compile()
def multi_perspective(g: GraphRecorder, question: str):
    g.param("question", "str")

    # Fast model: quick answer
    quick = g.ask(
        "Quick answer: {question}",
        model="gpt-3.5-turbo"
    )

    # Quality model: thorough analysis
    deep = g.think(
        "Detailed analysis: {question}",
        model="gpt-4"
    )

    # Local model: privacy-preserving
    local = g.think(
        "Local inference: {question}",
        backend="ollama",
        model="llama-3-70b"
    )

    # Synthesize all perspectives
    synthesis = g.reason(
        "Compare answers: Quick={quick}, Deep={deep}, Local={local}",
        model="gpt-4"  # Use quality model for synthesis
    )
    quick | synthesis
    deep | synthesis
    local | synthesis

    g.done(synthesis)
```

**Benefits:**
- Latency: Parallel execution (fastest model determines speed)
- Quality: Multiple perspectives reduce bias
- Cost: Mix of cheap/expensive models

### 3. Cascade: Try Fast, Fallback to Slow

Use cheap model first, fall back to expensive if quality is low.

```python
@compile()
def smart_cascade(g: GraphRecorder, query: str):
    g.param("query", "str")

    # Try fast model
    fast_answer = g.ask(
        "Answer: {query}",
        model="gpt-3.5-turbo"
    )

    # Verify quality
    quality_check = g.think(
        "Is this answer sufficient? {fast_answer}",
        model="gpt-3.5-turbo"
    )

    # If low quality, use powerful model
    # (In practice, use GUARD op when available)
    fallback_answer = g.reason(
        "Detailed answer: {query}",
        model="gpt-4"
    )

    # Select best answer (future: conditional execution)
    g.done(fallback_answer)
```

**Note:** Full cascade requires conditional execution (GUARD op), planned for v1.1.

---

## Backend Configuration

### Registering Multiple Backends

```bash
# Fast cloud backend
dekk apxm backend add openai-fast \
  --type cloud \
  --protocol openai \
  --api-key sk-... \
  --default-model gpt-3.5-turbo

# Quality cloud backend
dekk apxm backend add openai-quality \
  --type cloud \
  --protocol openai \
  --api-key sk-... \
  --default-model gpt-4

# Local backend
dekk apxm backend add ollama \
  --type local \
  --protocol ollama \
  --endpoint http://localhost:11434
```

### Listing Backends

```bash
dekk apxm backend list
```

**Output:**
```
Registered Backends:
────────────────────────────────

openai-fast (cloud, openai)
  Default model: gpt-3.5-turbo
  Status: active

openai-quality (cloud, openai)
  Default model: gpt-4
  Status: active

ollama (local, ollama)
  Default model: llama-3-70b
  Status: active
```

### Adding Models to Backends

```bash
# Add gpt-4o-mini to fast backend
dekk apxm backend add-model openai-fast gpt-4o-mini

# Add claude-opus to quality backend
dekk apxm backend add-model anthropic claude-opus-4
```

---

## Cost/Quality/Latency Trade-offs

### Benchmark: Support Ticket Workflow

From `multi_model.py` benchmark:

| Routing Strategy | Cost | Latency | Quality Score |
|------------------|------|---------|---------------|
| All gpt-3.5-turbo | $0.05 | 2.1s | 6.2/10 |
| All gpt-4 | $0.42 | 3.8s | 9.1/10 |
| **Multi-model (optimal)** | **$0.18** | **2.4s** | **8.7/10** |

**Multi-model routing:**
- Triage: gpt-3.5-turbo ($0.002)
- Extraction: llama-3 local ($0)
- Solution: gpt-4 ($0.15)
- Response: gpt-3.5-turbo ($0.003)

**Savings:** 57% cost vs all gpt-4, 96% quality vs all gpt-3.5

### Model Recommendations

| Model | Cost/1M tokens | Latency | Best For |
|-------|----------------|---------|----------|
| **gpt-3.5-turbo** | $0.50 | ~400ms | Triage, formatting, simple tasks |
| **gpt-4o-mini** | $0.15 | ~600ms | Balanced speed/quality |
| **gpt-4** | $30 | ~1200ms | Deep reasoning, quality-critical |
| **claude-haiku** | $0.25 | ~350ms | Fast, cheap, good quality |
| **claude-sonnet-4** | $3 | ~800ms | Balanced reasoning |
| **claude-opus-4** | $15 | ~1500ms | Best quality, expensive |
| **llama-3-70b** | $0 (local) | ~2000ms | Privacy, local, no API cost |

---

## Per-Node Attributes

Full API for model/backend control:

```python
result = g.ask(
    prompt="Your prompt here",
    model="gpt-4",           # Specific model name
    backend="openai-quality", # Specific backend
    max_tokens=2048,          # Token limit
    temperature=0.7,          # Sampling temperature
    nocache=True              # Disable caching
)
```

**Defaults:**
- `model`: Backend's default model
- `backend`: First registered backend
- `max_tokens`: 2048
- `temperature`: 0.95
- `nocache`: False

---

## Advanced: Optimization Target Interaction

Optimization targets affect multi-model routing:

### `--target cost`

Compiler may rewrite model selection to minimize cost:

```python
# You specified:
result = g.ask("Analyze", model="gpt-4")

# Compiler may downgrade (if quality analysis deems acceptable):
# result = g.ask("Analyze", model="gpt-3.5-turbo")
```

**Override:** Mark critical nodes with `model_required=True` (future API)

### `--target latency`

Compiler prefers faster models for non-critical paths:

```python
# Parallel branches: compiler may upgrade to faster model
branch1 = g.ask("Task 1", model="llama-3-70b")  # Slow
# Compiler may rewrite to: model="gpt-3.5-turbo"  # Fast
```

### `--target tokens`

Compiler prefers models with efficient tokenization:

```python
# Compiler tracks tokens-per-call and may switch models
# to minimize total token consumption
```

---

## Best Practices

1. **Default to single backend** initially, add routing after profiling
2. **Use fast models for:**
   - Triage/classification
   - Output formatting
   - Simple extractions
3. **Use quality models for:**
   - Deep reasoning
   - Critical decisions
   - Complex synthesis
4. **Use local models for:**
   - PII/sensitive data
   - Compliance requirements
   - High-volume/batch tasks (cost)
5. **Benchmark before production** with `--emit-metrics`
6. **Document routing rationale** in code comments

---

## Example: Customer Support Pipeline

Full example showing optimal routing:

```python
from apxm import compile, GraphRecorder


@compile()
def support_pipeline(g: GraphRecorder, ticket: str):
    """Multi-model customer support workflow."""
    g.param("ticket", "str")

    # Step 1: Triage (fast, cheap)
    triage = g.ask(
        "Classify priority and category: {ticket}",
        model="gpt-3.5-turbo"
    )

    # Step 2: Extract PII (local, privacy)
    pii = g.think(
        "Extract customer info (email, transaction ID): {ticket}",
        backend="ollama",
        model="llama-3-70b"
    )

    # Step 3: Generate solution (quality, critical)
    solution = g.reason(
        """
        Ticket: {ticket}
        Triage: {triage}
        Customer: {pii}

        Generate comprehensive solution:
        1. Root cause analysis
        2. Remediation steps
        3. Customer communication template
        """,
        model="gpt-4"  # Quality model for critical task
    )

    # Step 4: Format response (fast, cheap)
    email = g.ask(
        "Format as customer email: {solution}",
        model="gpt-3.5-turbo",
        max_tokens=400  # Short output
    )

    g.print("=== Support Ticket Resolution ===\n{email}")
    g.done(email)


if __name__ == "__main__":
    print(support_pipeline._graph.to_air())
```

**Cost breakdown:**
- Triage (gpt-3.5): $0.002
- PII extraction (local): $0
- Solution (gpt-4): $0.15
- Email (gpt-3.5): $0.003
- **Total: $0.155** (vs $0.42 all gpt-4)

**Latency:** ~2.8s (fast models are <500ms, gpt-4 dominates)

---

## Troubleshooting

### Model Not Found

**Error:** `Model 'gpt-5' not available in backend 'openai'`

**Fix:** Check available models:
```bash
dekk apxm backend list
dekk apxm backend add-model openai gpt-4
```

### Backend Not Found

**Error:** `Backend 'my-backend' not registered`

**Fix:** Register backend:
```bash
dekk apxm backend add my-backend --type cloud --protocol openai --api-key sk-...
```

### Unexpected Model Used

**Symptom:** Specified `model="gpt-4"` but metrics show `gpt-3.5-turbo`

**Cause:** Optimization pass downgraded model (cost target)

**Fix:** Disable model rewriting (future API) or use `-O0`

---

## Next Steps

- [Optimization Guide](optimization.md) — How optimization affects routing
- [Caching](caching.md) — Multi-model cache behavior
- [Backend Setup](backends.md) — Configuring multiple backends
- [Cost Tracking](../implementation/cost-tracking.md) — Monitoring multi-model costs
