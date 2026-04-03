# LLM Operations

Category: **Reasoning**. These operations send prompts to the configured LLM backend. They are stratified by latency tier so the scheduler can overlap cheap calls with expensive ones. Run `apxm ops list --category reasoning` for the current set.

## ASK

Simple Q&A -- no extended thinking. Use for classification, extraction, reformulation.

| Field | Required | Description |
|-------|----------|-------------|
| `template_str` | yes | Prompt template (supports `{{node_N}}` interpolation) |
| `temperature` | no | Sampling temperature (0.0--1.0) |
| `model` | no | LLM model override |

**Latency tier:** Medium (single LLM call, seconds).

```json
{"id": 1, "op": "ASK", "attributes": {"template_str": "Summarize: {{node_0}}"}}
```

## THINK

Extended thinking with a token budget. The LLM produces internal chain-of-thought reasoning before the final answer.

| Field | Required | Description |
|-------|----------|-------------|
| `template_str` | yes | Prompt template |
| `budget` | no | Token budget for extended thinking |
| `temperature` | no | Sampling temperature (0.0--1.0) |
| `model` | no | LLM model override |

**Latency tier:** High (tens of seconds).

```json
{"id": 1, "op": "THINK", "attributes": {"template_str": "Solve step by step: {{node_0}}", "budget": 4096}}
```

## REASON

Structured reasoning that can update the agent's beliefs and goals (AAM state). The runtime parses the response for belief/goal mutations. Supports structured JSON output.

| Field | Required | Description |
|-------|----------|-------------|
| `template_str` | yes | Prompt template |
| `structured` | no | Enable structured JSON output |
| `temperature` | no | Sampling temperature (0.0--1.0) |
| `model` | no | LLM model override |

**Latency tier:** Medium (single LLM call, seconds).

```json
{"id": 1, "op": "REASON", "attributes": {"template_str": "Given {{node_0}}, update your analysis", "structured": true}}
```

## PLAN

Decomposes a high-level goal into concrete steps using the LLM.

| Field | Required | Description |
|-------|----------|-------------|
| `goal` | yes | Goal to decompose |
| `constraints` | no | Constraints on the plan |

**Latency tier:** High (tens of seconds).

```json
{"id": 1, "op": "PLAN", "attributes": {"goal": "Research and summarize recent AI papers", "constraints": "max 5 steps"}}
```

## REFLECT

Retrieves past execution traces and asks the LLM to analyze them for patterns, failures, or improvements.

| Field | Required | Description |
|-------|----------|-------------|
| `trace_query` | yes | Query to retrieve trace for reflection |
| `reflection_prompt` | no | Custom prompt for reflection |

**Latency tier:** Medium (single LLM call, seconds).

```json
{"id": 4, "op": "REFLECT", "attributes": {"trace_query": "last_execution"}}
```

## VERIFY

Cross-references a claim against provided evidence using the LLM. Returns a verification result.

| Field | Required | Description |
|-------|----------|-------------|
| `claim` | yes | Claim to verify |
| `evidence` | yes | Evidence to check against |

**Latency tier:** Medium (single LLM call, seconds).

```json
{"id": 5, "op": "VERIFY", "attributes": {"claim": "{{node_3}}", "evidence": "{{node_4}}"}}
```

## Summary

| Op | Latency | Key use |
|----|---------|---------|
| ASK | Medium | Classification, extraction, simple Q&A |
| THINK | High | Complex multi-step reasoning, math, code generation |
| REASON | Medium | State-updating analysis with structured output |
| PLAN | High | Goal decomposition |
| REFLECT | Medium | Self-improvement from execution history |
| VERIFY | Medium | Fact-checking outputs |

The scheduler exploits this stratification: while a THINK or PLAN is in flight, independent ASK or REASON calls can run concurrently.
