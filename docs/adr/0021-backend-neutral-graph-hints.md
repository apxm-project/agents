# APXM graph hints: one contract, two `apxm` receivers

- Status: accepted
- Supersedes: `docs/ideas/backend-neutral-graph-hints.md`

## Decision

Agents owns one closed `ApxmGraphHints` contract (`schema = apxm.inference-graph-hints`)
of graph facts and advisory intents. vLLM and llama.cpp each live in an
APXM-org repository on a living `apxm` branch that **receives** that envelope.
Agents adapters project; they do not inline either server and they do not keep
a SHA/release/digest join catalog.

The vLLM conformance join (`apxm.vllm-conformance-join`, `backend_join.rs`,
`PINNED_VLLM_*`) is retired. Envelope keys live in
`apxm_core::constants::llm::apxm::graph_hints` and are the only names adapters
and `apxm` servers may use.

Hints never select models, change sampling, tools, schemas, or authority.
Unsupported fields are `OmittedUnsupported`. Graph prepare/release is
exact-binding local. llama.cpp never hashes `affinity_ref` into `id_slot`.

An adapter records two claims: the field-by-field plan and the projection it
sent (`GraphHintPlan`, `GraphHintProjection`, written to response metadata by
`record_graph_hint_evidence` in
`crates/runtime/backends/src/llm/backends/graph_hint_dispatch.rs`). Provider
acknowledgement and outcome measurement are two further, different claims, and
no provider response any adapter here parses states either — so neither is
recorded, and a binding cannot declare per field that it produces one.
`GraphHintFieldCapability` is `Direct`/`Derived`/`Unsupported` with no evidence
set, and `tools/tests/test_canonical_only_reachability.py`
(`test_no_graph_hint_evidence_layer_reports_only_nothing`) keeps the retired
names out until a producer exists.
