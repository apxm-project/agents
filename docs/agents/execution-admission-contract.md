# Execution Admission contract (product-neutral)

- Status: frozen public contract for G3 / P-006
- Date: 2026-08-03
- Owner: APXM `agents`
- Authority: [ADR-0013](../adr/0013-core-semantics-are-closed-and-implementations-enter-through-exact-port-bindings.md)

## 1. Purpose

Freeze the product-neutral **Execution Admission** envelope (agents synonym:
**Invocation Admission**) before consumers bind against it. Downstream products
may seal admissions; APXM verifies them and never depends on product schema.

## 2. Schema

- Schema id: `apxm.execution-admission`
- Rust owner: `apxm_kernel::admission::ExecutionAdmission`
- Signature algorithm: `ed25519` only
- Digest form: `sha256:` + 64 lowercase hex

Required fields:

| Field | Meaning |
| --- | --- |
| `admission_ref` | Opaque admission identity |
| `admission_digest` | Content digest of the unsigned payload |
| `artifact_ref` / `artifact_digest` | Exact admitted artifact |
| `invocation_ref` | Exact invocation identity |
| `context_digest` | Admitted program-context digest |
| `resource_ceilings` | Explicit wall/memory/effect ceilings |
| `port_bindings[]` | Exact Port slot → contract → binding → proof |
| `confinement` | Pinned confinement type + sandbox/policy digests |
| `expires_at_ms` | Hard expiry (unix ms) |
| `nonce` | Single-use nonce |
| `issuer` / `audience` | Issuer identity and intended runtime audience |
| `caller_correlations` | Opaque string map only |
| `signature` | `{algorithm,key_ref,signature}` |

Optional:

| Field | Meaning |
| --- | --- |
| `model_target` | Exact model target/deployment/binding when inference is required |

## 3. Verification laws

1. Unknown schema version fails closed.
2. Signature, digest, key unknown/expired/revoked fail closed.
3. Expiry and nonce reuse fail closed.
4. Audience must equal the runtime's expected audience.
5. Every required Port is present exactly once; duplicates are ambiguous and fail.
6. `confinement` is always required. `execution_commit` is not a field on this
   envelope — it is a Port slot, so law 5 already covers it.
7. Each closed Port slot carries its exact contract schema id; a slot cannot be
   relabeled as another Port family.
8. Confinement type `unconfined` (and equivalent digests) fail closed.
9. Product-plane fields (`company_ref`, budgets, principals, grants, leases) are
   refused at parse time.
10. Ambient credential/filesystem/network/plugin markers in correlations fail.
11. When a model target is present, it must match the admitted model-inference
    binding; when the artifact requires inference, its presence is mandatory.
12. The nonce is consumed only after all admission invariants pass. Invalid
    records do not mutate the replay ledger.
13. Runtime construction must compare every supplied binding with the verified
    binding and obtain an attestation for the exact confinement type, sandbox,
    policy, host and execution identity before returning a usable bundle.

## 4. Non-goals

- Company Auth, Sessions, budgets, catalogues, or Host brand semantics.
- Inference driver implementation (P-010 / agents#37).
- provider-specific backend conformance.
- Hosted durable checkpoint product service decision (P-018 / agents#39).
- Compiler FrontendGraph/AIR freeze (P-002–P-005 / agents#35) beyond consuming
  artifact digests already present on the admission.

## 5. Owner surface

| Capability | Owner | Gate |
| --- | --- | --- |
| P-006 Execution Admission + exact binding | `agents` / `apxm-kernel::admission` | G3 |
| P-007 Runtime / checkpoint / cancel / confinement | `agents` / kernel + execution | G3 |
| P-008 Capability/effect + outcome_unknown | `agents` / `apxm-kernel::effect` | G3 |
| P-009 Atomic execution commit | `agents` / `apxm-kernel::commit` | G3 |
