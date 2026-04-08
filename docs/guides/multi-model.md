# Multi-Model Routing

APXM supports per-node model and backend selection, enabling cost/quality/latency trade-offs within a single workflow.

---

## Why Multi-Model?

Different tasks have different requirements:

| Task | Requirement | Optimal Model |
|------|-------------|---------------|
| **Triage** | Speed, low cost | gpt-3.5-turbo, claude-haiku |
| **Deep analysis** | Quality, reasoning | gpt-4, claude-opus |
| **Structured extraction** | Privacy, local | llama-3-70b (local) |
| **Formatting** | Speed, template | gpt-3.5-turbo |

A single model forces a trade-off: cheap means poor quality on complex tasks; expensive means high cost on simple tasks. Multi-model routing assigns each operation to the best model for the job.

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
        model="gpt-3.5-turbo"
    )

    # Powerful model for analysis
    solution = g.think(
        "Generate detailed solution: {ticket_text}\nTriage: {triage}",
        model="gpt-4"
    )

    # Fast model for formatting
    response = g.ask(
        "Format customer email: {solution}",
        model="gpt-3.5-turbo"
    )

    g.done(response)
```

### Per-Node Backend Selection

```python
from apxm import compile, GraphRecorder


@compile()
def privacy_workflow(g: GraphRecorder, user_data: str):
    g.param("user_data", "str")

    # Sensitive data -> local model
    extracted = g.think(
        "Extract PII: {user_data}",
        backend="ollama",
        model="llama-3-70b"
    )

    # Public data -> cloud model (faster)
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

Use cheap models for setup and cleanup, expensive models for core logic.

```python
@compile()
def research(g: GraphRecorder, topic: str):
    g.param("topic", "str")

    outline = g.ask("Create research outline: {topic}", model="gpt-3.5-turbo")
    findings = g.think("Deep research: {outline}", model="gpt-4")
    report = g.ask("Format as executive summary: {findings}", model="gpt-3.5-turbo")

    g.done(report)
```

**Cost savings:** ~60% vs all gpt-4. **Latency:** comparable (fast models add <500ms).

### 2. Parallel Multi-Model Fan-Out

Multiple models analyze in parallel, then a synthesis step combines their perspectives.

```python
@compile()
def multi_perspective(g: GraphRecorder, question: str):
    g.param("question", "str")

    quick = g.ask("Quick answer: {question}", model="gpt-3.5-turbo")
    deep = g.think("Detailed analysis: {question}", model="gpt-4")
    local = g.think("Local inference: {question}", backend="ollama", model="llama-3-70b")

    synthesis = g.reason(
        "Compare answers: Quick={quick}, Deep={deep}, Local={local}",
        model="gpt-4"
    )
    quick | synthesis
    deep | synthesis
    local | synthesis

    g.done(synthesis)
```

Benefits: parallel execution keeps latency near the slowest model, multiple perspectives reduce bias, and mixing cheap with expensive models controls cost.

### 3. Cascade: Try Fast, Fallback to Slow

Use a cheap model first; fall back to an expensive model if quality is low.

```python
@compile()
def smart_cascade(g: GraphRecorder, query: str):
    g.param("query", "str")

    fast_answer = g.ask("Answer: {query}", model="gpt-3.5-turbo")
    quality_check = g.think("Is this answer sufficient? {fast_answer}", model="gpt-3.5-turbo")

    # If low quality, use powerful model
    # (In practice, use GUARD op when available)
    fallback_answer = g.reason("Detailed answer: {query}", model="gpt-4")

    # Select best answer (future: conditional execution)
    g.done(fallback_answer)
```

**Note:** Full cascade requires conditional execution (GUARD op), planned for a future release.

---

## Backend Configuration

Register multiple backends, then reference them by name in graph nodes:

```bash
# Fast cloud backend
apxm backend add openai-fast \
  --type cloud --protocol openai --api-key sk-...

# Quality cloud backend
apxm backend add openai-quality \
  --type cloud --protocol openai --api-key sk-...

# Local backend
apxm backend add ollama \
  --type local --protocol ollama --endpoint http://localhost:11434

# Add models to backends
apxm backend add-model openai-fast gpt-4o-mini
apxm backend add-model anthropic claude-opus-4
```

See [Backends Guide](backends.md) for backend types and [Configuration Reference](../reference/config.md) for the full `config.toml` format.

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

## Optimization Target Interaction

Optimization targets can affect multi-model routing at compile time:

### `--target cost`

The compiler may downgrade model selection to minimize cost when quality analysis deems it acceptable. Override by marking critical nodes with `model_required=True` (future API).

### `--target latency`

The compiler prefers faster models for non-critical-path branches.

### `--target tokens`

The compiler prefers models with efficient tokenization to minimize total token consumption.

See [Optimization Overview](../optimization/overview.md) and [Optimization Passes](../optimization/passes.md) for details on how passes interact with routing.

---

## Best Practices

1. **Start with a single backend**, then add routing after profiling actual costs.
2. **Use fast models for** triage, classification, output formatting, and simple extractions.
3. **Use quality models for** deep reasoning, critical decisions, and complex synthesis.
4. **Use local models for** PII/sensitive data, compliance requirements, and high-volume batch tasks.
5. **Benchmark before production** with `--emit-metrics`.
6. **Document routing rationale** in code comments.

---

## Troubleshooting

### Model Not Found

**Error:** `Model 'gpt-5' not available in backend 'openai'`

**Fix:** Check available models and add the missing one:
```bash
apxm backend list
apxm backend add-model openai gpt-4
```

### Backend Not Found

**Error:** `Backend 'my-backend' not registered`

**Fix:** Register the backend:
```bash
apxm backend add my-backend --type cloud --protocol openai --api-key sk-...
```

### Unexpected Model Used

**Symptom:** Specified `model="gpt-4"` but metrics show `gpt-3.5-turbo`.

**Cause:** An optimization pass downgraded the model (cost target).

**Fix:** Disable model rewriting with `-O0`, or mark the node as requiring the specified model (future API).

---

## See Also

- [Optimization Overview](../optimization/overview.md) -- How optimization affects routing
- [Optimization Passes](../optimization/passes.md) -- Pass-level details (CSE, FuseReasoning, MemoCache)
- [Caching](caching.md) -- Multi-model cache behavior
- [Backends Guide](backends.md) -- Backend types, protocols, and health monitoring
- [Configuration Reference](../reference/config.md) -- Full `config.toml` routing specification
