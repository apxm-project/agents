# A-PXM Value Hypotheses

Testable hypotheses about the advantages of building agents on A-PXM. Each hypothesis has a concrete prediction and a measurement plan.

---

## H1: Operation Fusion Reduces Token Cost

**Hypothesis**: Compile-time operation fusion (FuseAskOps) reduces total LLM token usage by 15–30% in workflows with 3+ sequential ASK nodes.

**Rationale**: Each ASK→ASK fusion eliminates one LLM call. The fused prompt is shorter than the sum of two separate prompts (no repeated context, no intermediate marshaling).

**Measurement**:
- Compile the same workflow at O0 (no fusion) and O2 (with fusion)
- Run both versions with identical inputs
- Compare total input + output tokens
- Compare total API cost

**Success criterion**: ≥15% token reduction on the benchmark workflows.

**Status**: FuseAskOps is implemented. Measurement infrastructure (--emit-metrics) exists. Needs systematic benchmarking.

---

## H2: Automatic Parallelism Reduces Latency

**Hypothesis**: A-PXM's dataflow scheduler extracts parallelism that reduces wall-clock latency by 2–5× compared to sequential execution in workflows with independent branches.

**Rationale**: In a fan-out/fan-in pattern (e.g., 4 independent ASK nodes followed by MERGE), sequential execution takes 4× the single-call latency. The scheduler runs all 4 concurrently.

**Measurement**:
- Create a fan-out workflow with N independent ASK nodes
- Measure wall-clock time with dataflow scheduling vs. forced sequential
- Report speedup factor

**Success criterion**: ≥2× speedup on N≥3 independent branches.

**Status**: Scheduler exists with 7.5μs overhead. Multi-agent perf table shows 10.37× speedup. Needs single-workflow parallelism benchmarks.

---

## H3: Prompt Caching Reduces Input Token Cost by 50–90%

**Hypothesis**: When multiple ASK nodes share a common prompt prefix (system prompt + project context), provider-side prompt caching reduces input token cost by 50–90%.

**Rationale**: Anthropic's prompt caching prices cached input tokens at 10% of full price. OpenAI offers similar discounts. If 5 ASK nodes share a 2000-token system prompt, 4 of them pay cached price.

**Measurement**:
- Implement shared prefix detection (compiler pass)
- Measure input token cost with and without caching on each provider
- Report cost savings as percentage

**Success criterion**: ≥50% reduction in input token cost for workflows with shared system prompts.

**Status**: Not implemented. Requires compiler pass + backend support. Anthropic and OpenAI both support caching APIs.

---

## H4: Response Memoization Eliminates Redundant Calls

**Hypothesis**: In iterative workflows (agent loops with retries), response memoization eliminates 20–40% of LLM calls by caching results of identical prompts.

**Rationale**: Coding agents frequently re-read files, re-classify errors, and re-evaluate conditions. If the underlying data hasn't changed, the response is the same.

**Measurement**:
- Instrument an agent session to track prompt uniqueness
- Measure cache hit rate over a 20-turn session
- Report eliminated calls as percentage

**Success criterion**: ≥20% cache hit rate in a typical coding agent session.

**Status**: Not implemented. Requires runtime memoization layer with invalidation.

---

## H5: Model Routing Reduces Cost Without Quality Loss

**Hypothesis**: Routing simple operations (extraction, formatting, classification) to cheaper models reduces total cost by 40–60% with <5% quality degradation.

**Rationale**: Most agent workflows have a mix of hard tasks (reasoning, planning) and easy tasks (formatting, summarization). The easy tasks don't need the most expensive model.

**Measurement**:
- Annotate nodes with complexity tier (via CapabilityScheduling)
- Run workflow with all-expensive-model and with routed models
- Compare cost and output quality (human eval or automated VERIFY pass rate)

**Success criterion**: ≥40% cost reduction with ≤5% quality loss.

**Status**: CapabilityScheduling pass exists. Model routing logic needs implementation.

---

## H6: Compile-Time Verification Catches Errors Earlier

**Hypothesis**: Graph validation catches 80%+ of structural errors (type mismatches, missing edges, unreachable nodes) at compile time that would otherwise surface at runtime.

**Rationale**: Sequential execution discovers errors one node at a time. Graph validation checks the entire workflow before any LLM call is made.

**Measurement**:
- Collect a corpus of malformed workflows (from bug reports, user errors, mutation testing)
- Measure detection rate: compile-time validation vs. runtime error
- Report early detection percentage

**Success criterion**: ≥80% of structural errors caught at compile time.

**Status**: `apxm validate` exists. Needs systematic error corpus.

---

## H7: Shared Infrastructure Reduces Agent Development Effort

**Hypothesis**: An agent built on AgentMate + A-PXM requires ≤500 lines of agent-specific code, compared to 5,000–50,000 lines for a from-scratch implementation.

**Rationale**: The seven runtime primitives (inference loop, tool dispatch, context management, memory, sandboxing, multi-agent coordination, streaming) are shared infrastructure. Agent-specific code should only be the workflow graph and domain logic.

**Measurement**:
- Build Codex-on-APXM and count agent-specific lines
- Compare to Codex OSS line count
- Report ratio

**Success criterion**: ≤10% of the code needed for a from-scratch implementation.

**Status**: Not yet tested. Codex-on-APXM implementation plan exists.

---

## H8: AAM State Model Enables Better Debugging

**Hypothesis**: The formal AAM state transition trace reduces debugging time by 50% compared to log-based debugging for multi-step agent failures.

**Rationale**: When an agent fails on step 12 of a 20-step workflow, the AAM trace shows the exact state (Beliefs, Goals, Capabilities) at each transition. No need to reconstruct state from scattered log entries.

**Measurement**:
- Create a set of debugging scenarios (agent failures at various points)
- Measure time-to-root-cause with AAM trace vs. log files
- Report improvement

**Success criterion**: ≥50% reduction in time-to-root-cause.

**Status**: AAM exists but only 7/32 ops produce transitions (22%). Full AAM coverage needed first.

---

## Priority Order

Based on expected impact and implementation difficulty:

1. **H1** (fusion) — Already implemented, just needs measurement
2. **H2** (parallelism) — Already implemented, just needs benchmarks
3. **H6** (compile-time verification) — Already implemented, needs test corpus
4. **H3** (prompt caching) — High impact, medium difficulty
5. **H5** (model routing) — High impact, medium difficulty
6. **H4** (memoization) — High impact, medium difficulty
7. **H7** (shared infrastructure) — Validates the entire thesis, needs Codex-on-APXM
8. **H8** (debugging) — Requires AAM gap closure first
