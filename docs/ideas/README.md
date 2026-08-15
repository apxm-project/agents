# APXM design ideas

Documents in this directory are proposals, not shipped or canonical APXM
contracts. Each document states its own status and identifies the evidence
needed before promotion into a contract or architecture decision.

- [Collapsing the org package onto one authored manifest](org-package-collapse.md)
  — implemented: one authored `org.toml`, published `apxm.org` contract and
  13 vectors, join deleted rather than relocated.
- [Backend-neutral graph hints](backend-neutral-graph-hints.md) — a common
  APXM graph-fact and execution-intent contract with explicit backend
  projection, including llama.cpp and vLLM lowerings.
- [Generating the source-first frontends from their contract](frontend-vocabulary-generation.md)
  — where each authoring vocabulary's single source of truth lives, the closed
  sets now projected into both frontends, and the record types, emitter, and
  diagnostic wiring still to be generated.
- [CLI shell, Compilation Service, Runtime Service, OpenAI protocol API, and program-owned harness migration](cli-interaction-runtime-service-migration.md)
  — a full-replacement plan for a Crush-quality terminal client, canonical
  runtime lifecycle and source-neutral external-Event ingress spanning human,
  webhook, broker, schedule, explicit Hook signals, device, and sensor sources,
  separate Compilation and Runtime Services joined only by a committed
  artifact, Python/TypeScript authoring with Rust-owned compiler lowering, and
  secured Event/OpenAI-protocol Program-serving edges and a zero-legacy
  cutover, with all
  conversational and orchestration behavior remaining compiled Agent Program
  source.
