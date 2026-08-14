# APXM design ideas

Documents in this directory are proposals, not shipped or canonical APXM
contracts. Each document states its own status and identifies the evidence
needed before promotion into a contract or architecture decision.

- [Backend-neutral graph hints](backend-neutral-graph-hints.md) — a common
  APXM graph-fact and execution-intent contract with explicit backend
  projection, including llama.cpp and vLLM lowerings.
- [Generating the source-first frontends from their contract](frontend-vocabulary-generation.md)
  — where each authoring vocabulary's single source of truth lives, the closed
  sets now projected into both frontends, and the record types, emitter, and
  diagnostic wiring still to be generated.
