# apxm-compiler

`apxm-compiler` owns the canonical AIR-to-AIS verification boundary. It does
not own product routing, deployment, prompt optimization, or a second runtime.

## Pipeline

```text
FrontendGraph
  -> Rust verification and lowering
  -> AIR
  -> deterministic MLIR in the registered AIS dialect
  -> verified immutable artifact
```

The five public semantic operations are selected by the Rust-owned catalogue:
`model.call`, `capability.invoke`, `program.new`, `program.invoke`, and
`await.event`. Structural regions and `ais.loop` are compiler-emitted only.

The canonical MLIR lowering is [src/canonical.rs](src/canonical.rs). It emits
registered `ais.*` operations with typed token payloads, disables unregistered
dialects, and rejects invalid operands before an artifact can be produced.
The native MLIR parser/verifier is an optional build-time toolchain gate; AIR
validation and artifact admission do not select a backend or provider.

The compiler may perform backend-neutral validation and analysis. It does not
select providers, endpoints, credentials, grants, retries, or fallback
targets. Those enter through exact Invocation Admission and Port bindings.

## Verification

```bash
dekk agents test-compiler
```
