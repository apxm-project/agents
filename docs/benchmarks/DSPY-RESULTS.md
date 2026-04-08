# DSPy Optimization Results for APXM Workflows

This document presents empirical results from running DSPy prompt optimization on real APXM workflows.

## Experiment Overview

**Date**: 2026-04-08
**DSPy Version**: 3.1.3
**Benchmark**: `examples/python/benchmarks/dspy_quality.py`
**Workflow**: Technical Q&A pipeline with 3 LLM operations (ASK → THINK → REASON)

## Methodology

1. **Baseline Graph**: Created a technical Q&A workflow with simple templates
2. **Training Data**: 8 examples covering software architecture, ML, and system design topics
3. **Optimizer**: DSPy `LabeledFewShot` with k=3 examples (no API key required)
4. **Measurement**: Template structure analysis, few-shot example injection, length changes

## Results Summary

### Optimization Coverage

- **Nodes analyzed**: 3 LLM operations
- **Nodes optimized**: 3 (100.0% coverage)
- **Optimizer**: `labeled_fewshot` with max_demos=3

### Template Transformations

| Node      | Operation | Baseline Length | Optimized Length | Increase  | Examples Added |
|-----------|-----------|-----------------|------------------|-----------|----------------|
| answer    | ASK       | 12 chars        | 2,059 chars      | +2,047    | 3              |
| analysis  | THINK     | 187 chars       | 2,052 chars      | +1,865    | 3              |
| synthesis | REASON    | 185 chars       | 2,052 chars      | +1,867    | 3              |

**Total template expansion**: +5,779 characters (+1,926 chars per node average)

### Few-Shot Example Injection

DSPy successfully injected domain-relevant few-shot examples into each template:

**Example 1**: Microservices architecture (distributed systems)
**Example 2**: Caching strategies (performance optimization)
**Example 3**: URL shortening service design (system design)

These examples were automatically selected from the training set to provide contextual guidance for each LLM operation.

## Detailed Analysis

### Before Optimization (Baseline)

#### ASK Node Template
```
{{question}}
```

**Characteristics**:
- Minimal template (12 chars)
- Single variable placeholder
- No guidance or examples
- Zero-shot prompting

#### THINK Node Template
```
Given this answer:
{{0}}

Provide a detailed technical analysis covering:
1. Core concepts involved
2. Common misconceptions
3. Real-world applications
4. Related technologies or patterns
```

**Characteristics**:
- Structured prompt with enumerated requirements
- Single input placeholder (positional)
- No examples
- Medium complexity (187 chars)

#### REASON Node Template
```
Based on this analysis:
{{0}}

Synthesize the key insights and explain how this concept connects to broader
software engineering principles. What are the implications for system design?
```

**Characteristics**:
- Open-ended synthesis prompt
- Single input placeholder
- No examples
- Medium complexity (185 chars)

---

### After DSPy Optimization

#### ASK Node Optimized Template (excerpt)
```
Examples:
Example 1:
  question: What is microservices architecture?
  answer: Microservices architecture is a design approach where applications
  are composed of small, independent services that communicate over well-defined
  APIs. Each microservice handles a specific business capability, can be developed
  and deployed independently...

Example 2:
  question: How does caching improve system performance?
  answer: Caching improves performance by storing frequently accessed data in
  fast-access memory layers (RAM, SSD) closer to the application...

Example 3:
  question: Design a URL shortening service similar to bit.ly...
  answer: A URL shortening service requires four key components: 1) A hash
  generation service...

Now complete the following:
question: {{question}}
answer:
```

**Characteristics**:
- Few-shot prompting with 3 examples
- Examples cover diverse technical topics
- Structured format (question/answer pairs)
- Significant expansion: 12 → 2,059 chars
- Template now guides LLM with domain-appropriate patterns

#### THINK and REASON Nodes

Similar transformations applied:
- 3 few-shot examples injected
- Same example set used (domain-general technical Q&A)
- Placeholder format updated to match DSPy signature
- Maintained original prompt intent while adding guidance

---

## Key Findings

### 1. Automatic Few-Shot Enhancement

DSPy successfully transformed **zero-shot** templates into **few-shot** templates without manual prompt engineering:
- All 3 nodes received 3 examples each
- Examples selected from training data
- Format standardized across nodes

### 2. Template Expansion Trade-offs

**Pros**:
- Rich contextual guidance for LLM
- Consistent formatting across examples
- Domain-appropriate patterns shown

**Cons**:
- Significant token overhead (+5.7k chars = ~1.4k tokens)
- All nodes share same examples (not task-specific)
- No learned instructions (LabeledFewShot limitation)

### 3. Optimizer Choice Impact

Using `LabeledFewShot`:
- ✓ No API key required (fast, deterministic)
- ✓ Direct example injection
- ✗ No learned instructions or optimized phrasing
- ✗ Simple example selection (not bootstrapped)

**For comparison**, `BootstrapFewShot` would:
- Require API calls (slower, costs money)
- Bootstrap examples by running LLM
- Learn which examples work best
- Potentially add optimized instructions

### 4. Production Readiness Gaps

**Missing for production use**:
- [ ] **Task-specific examples**: All nodes share same example set; should vary by operation type
- [ ] **Instruction learning**: LabeledFewShot doesn't optimize prompt phrasing
- [ ] **Example selection**: No bootstrapping to find best examples
- [ ] **Token budget**: No length constraints applied
- [ ] **Caching**: Repeated optimization rebuilds from scratch
- [ ] **Evaluation metric**: Simple metric doesn't measure quality improvement

---

## Performance Implications

### Token Usage Impact

Original baseline graph: ~400 chars across 3 nodes
Optimized graph: ~6,200 chars across 3 nodes

**Implications**:
- **15x token increase** in prompt size
- For Claude Sonnet 4: ~1,500 input tokens added per execution
- Cost increase: ~$0.002 per workflow run (at $3/MTok input)
- Latency increase: Minimal (modern LLMs process long contexts efficiently)

### Trade-off Analysis

**When DSPy optimization helps**:
- Complex reasoning tasks where examples improve accuracy
- Workflows where quality > cost
- Tasks where few-shot significantly outperforms zero-shot

**When baseline may be better**:
- Simple, well-defined tasks
- Token-constrained environments
- Real-time/low-latency requirements
- High-volume workflows (cost-sensitive)

---

## Integration with APXM Compiler

### Current State

DSPy optimization runs as a **separate pass** before compilation:
1. User creates `.apxm` graph with simple templates
2. `python3 -m apxm.dspy_bridge graph.apxm --training-data data.json`
3. Outputs `graph_optimized.apxm` with enhanced templates
4. User compiles optimized graph: `dekk apxm compile graph_optimized.apxm`

### Proposed Integration (Future Work)

**Option 1: Compiler Flag**
```bash
dekk apxm compile graph.apxm -O3 --dspy-optimize --training-data data.json
```
- DSPy runs as part of MLIR pass pipeline
- Integrated with existing optimization levels
- Cached optimized templates in artifact metadata

**Option 2: Auto-Optimization**
```bash
dekk apxm execute graph.apxm --auto-optimize
```
- Use session history as training data
- Incrementally improve prompts over executions
- Reinforcement learning approach

**Option 3: Template Registry**
```bash
dekk apxm template optimize qa-workflow --training-data data.json
```
- Optimize reusable template patterns
- Share optimized templates across workflows
- Template versioning and A/B testing

---

## Recommendations

### For Current Bridge (v0.1)

1. **Add task-specific example selection**: Use different examples for ASK vs THINK vs REASON
2. **Implement token budgets**: Limit few-shot examples to fit within token constraints
3. **Add quality metrics**: Measure actual output quality improvement
4. **Cache optimized templates**: Avoid re-optimization on every run
5. **Support MIPROv2**: Add instruction learning for better optimization

### For MLIR Integration (v0.2)

1. **Define DSPy optimization as MLIR pass**: `--dspy-optimize` flag triggers pass
2. **Store training data in artifact**: Embed examples in `.apxmobj` metadata
3. **Incremental optimization**: Use execution profiles to refine templates
4. **Multi-objective optimization**: Balance quality, tokens, and latency

### For Production Use (v1.0)

1. **A/B testing framework**: Compare baseline vs optimized in production
2. **Cost-quality trade-off UI**: Let users tune optimization aggressiveness
3. **Template marketplace**: Share and discover optimized templates
4. **AutoDSPy mode**: Automatic continuous optimization from session history

---

## Reproducibility

### Running the Benchmark

```bash
# Generate baseline and optimized graphs
PYTHONPATH=crates/apxm-frontend/python:$PYTHONPATH \
  python3 examples/python/benchmarks/dspy_quality.py --verbose

# View the comparison report
PYTHONPATH=crates/apxm-frontend/python:$PYTHONPATH \
  python3 examples/python/benchmarks/dspy_quality.py --compare

# Execute with real LLM (requires backend configured)
dekk apxm execute /tmp/dspy_quality_baseline.apxm
dekk apxm execute /tmp/dspy_quality_optimized.apxm
```

### Files Generated

- `/tmp/dspy_quality_baseline.apxm` - Original workflow
- `/tmp/dspy_quality_optimized.apxm` - DSPy-enhanced workflow
- `examples/python/benchmarks/dspy_training_data.json` - Training examples
- `examples/python/benchmarks/dspy_quality.py` - Benchmark script

---

## Conclusion

This experiment demonstrates that DSPy can successfully optimize APXM workflow templates through automatic few-shot example injection. The `LabeledFewShot` optimizer increased template sizes by ~15x (from 400 to 6,200 chars) while adding structured examples to guide LLM behavior.

**Key Takeaways**:
1. ✓ **DSPy integration works**: Bridge successfully optimizes ASK/THINK/REASON templates
2. ✓ **Few-shot enhancement**: All nodes receive contextual examples
3. ✗ **Token overhead**: 15x increase may be prohibitive for high-volume workflows
4. ✗ **No quality measurement yet**: Need real LLM evaluation to quantify improvement

**Next Steps**:
1. Implement task-specific example selection per operation type
2. Add quality evaluation with real LLM backend
3. Integrate DSPy optimization into MLIR compiler pipeline
4. Build template caching and versioning system

---

## Appendix: Full Template Comparison

### ASK Node - Full Optimized Template

```
Examples:
Example 1:
  question: What is microservices architecture?
  answer: Microservices architecture is a design approach where applications are composed of small, independent services that communicate over well-defined APIs. Each microservice handles a specific business capability, can be developed and deployed independently, and typically owns its own database. This architecture enables better scalability (scale services independently), resilience (failures are isolated), and team autonomy (different teams can work on different services). However, it introduces complexity in distributed system coordination, monitoring, and data consistency.

Example 2:
  question: How does caching improve system performance?
  answer: Caching improves performance by storing frequently accessed data in fast-access memory layers (RAM, SSD) closer to the application, reducing expensive operations like database queries or API calls. It works on the principle of temporal locality (recently accessed data is likely to be accessed again) and spatial locality (nearby data is likely to be accessed together). Effective caching strategies include: 1) Multi-level caching (L1/L2/CDN), 2) Cache invalidation policies (TTL, write-through, write-back), 3) Cache warming for predictable access patterns, and 4) Distributed caching for horizontal scaling.

Example 3:
  question: Design a URL shortening service similar to bit.ly. What are the key components?
  answer: A URL shortening service requires four key components: 1) A hash generation service that creates unique short codes from long URLs, using algorithms like Base62 encoding or MD5 hashing with collision detection. 2) A high-performance key-value database (like Redis or DynamoDB) to store URL mappings with fast lookup times. 3) A redirect service that handles HTTP redirects from short URLs to original URLs with minimal latency. 4) An analytics layer to track click metrics, geographic data, and referrer information for each shortened URL.

Now complete the following:
question: {{question}}
answer:
```

**Analysis**: The optimized template provides rich context showing the expected answer format, depth of technical detail, and structured explanation style. However, all examples are software architecture questions—not ideal if the actual question is about ML, data science, or other domains.

---

## References

- DSPy Documentation: https://dspy-docs.vercel.app
- APXM DSPy Bridge: `crates/apxm-frontend/python/apxm/dspy_bridge.py`
- Benchmark Script: `examples/python/benchmarks/dspy_quality.py`
- Training Data: `examples/python/benchmarks/dspy_training_data.json`
