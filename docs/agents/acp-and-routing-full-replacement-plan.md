# ACP interoperability and exact-selection full-replacement plan

- Status: canonical implementation plan subordinate to the APXM master plan
- Scope: ACP Client interoperability and v1 exact model/profile selection
- Contract: [ACP interoperability and selection](acp-and-routing-contract.md)
- Future work: [APXM-owned model and External Agent routing](future-apxm-routing-plan.md)

## 1. Outcome

Replace the prototype ACP/process/router/fallback stack with one exact path:

```text
Agent Program source
  -> FrontendGraph
  -> five-op AIR
  -> selected deployment composition + shared admission verifier
  -> runtime capability.invoke or model.call
  -> exact admitted ACP or inference adapter
  -> nested/usage/runtime evidence
```

The target supports exact pinned `claude-agent-acp` and `codex-acp` profiles
plus the same exact-profile conformance path for additional ACP agents. An
adapter may use a vendor SDK/App Server internally; APXM core does not depend
on it. The target contains no legacy wire
dialect, arbitrary command profile, AAM injection, runtime AgentRouter,
ModelRouter fallback, Server direct-generate bypass, alias or dual path.

## 2. Parallel lanes

| Lane | Repository | Exclusive scope | Depends on | Exit evidence |
| --- | --- | --- | --- | --- |
| C-ACP | `contracts` | ACP profile/session/closed evidence, operational-usage boundary and exact model binding schemas | C0a | closed schemas, provenance and skew vectors |
| A-ACP | `agents` | Python/TypeScript wrappers, typed ports, compiler lowering, runtime handler, false native token-accounting removal and prototype deletion | C-ACP | Python/TS goldens, accountant rejection and five-op absence scan |
| D-ACP | `adapters` | official ACP v1 client, process/Confinement boundary, Claude/Codex profiles | C-ACP, H-ACP | protocol and supply-chain matrix |
| S-BIND | `server` | shared admission verifier, managed exact deployment/profile catalogue, declared binding materialization, session correlation, removal of direct model bypass | C-ACP | managed/standalone parity, exact binding and no-bypass tests |
| H-ACP | `auth` | connection references, reverse-request grants/approvals, roots/egress and ephemeral injection | C-ACP | attenuation/revocation vectors |
| T-ACP | `studio` | profile catalogue, connection, conformance, session, cancel/close and nested evidence UX | generated clients | cross-surface journeys |
| E-ACP | `eval` | ACP compatibility, authority, Confinement, lifecycle, failure and usage provenance | A-ACP, D-ACP, S-BIND, H-ACP | signed release evidence |

Each repository uses its own clean lane worktree. Contract generation and
integration merges follow the master-plan queues; no lane edits another
repository or a coordinator integration worktree.

## 3. Contract freeze

Publish and pin:

- `apxm.external-agent-profile.v1`;
- `apxm.external-agent-session.v1`;
- `apxm.external-agent-evidence.v1`;
- `apxm.external-agent-reverse-request.v1`;
- `apxm.operational-usage-fact.v1` boundary rules;
- `apxm.model-deployment.v1`;
- `apxm.model-binding.v1`;
- exact errors for unsupported feature, denied reverse request, session lost,
  cancellation timeout and outcome unknown; and
- conformance vectors for initialization, authentication, session lifecycle,
  prompt updates, reverse requests, cancellation and close.

The official ACP schema/SDK version is a Compatibility Set input. Handwritten
wire names do not define the contract.

## 4. `agents` replacement

1. Add equivalent Python and TypeScript typed wrappers over the four
   external-agent Capabilities.
2. Record exact Capability and profile requirements in FrontendGraph.
3. Compile every wrapper call to `capability.invoke`; reject direct ACP/process
   builders.
4. Preserve exact portable `ModelTargetRef` on every `model.call` and accept
   only the immutable `ResolvedModelBinding` materialized by the shared
   deployment-composition verifier at runtime.
5. Implement opaque session-handle scoping and explicit Context retention.
6. Normalize ordered ACP events as nested evidence under the outer
   NodeExecution without creating APXM Turns/nodes.
7. Preserve peer measurements with explicit scope, accumulation, precision,
   origin, and availability; delete the current `usage_update.used/size` to
   native input/output-token mapping, native `TokenAccountant` call, and
   fabricated model/Generation identity.
8. Propagate structured cancellation and fence reverse requests after loss.
9. Delete `SPAWN_AGENT`, `COMMUNICATE`, `HANDOFF`, `DELEGATE`, process-table
   semantics, AAM bridge and hidden prompt injection.
10. Delete runtime `AgentRouter`, `ModelRouter`, profile routers, fallback tags,
   ambient model registries and direct provider escapes.
11. Regenerate all builders, parsers, docs, examples and tests; absence-scan
    source and generated outputs.

## 5. ACP adapter replacement

1. Replace handwritten protocol constants/types with the pinned official ACP
   schema/SDK or generated exact bindings.
2. Implement standard method names and notification/request semantics.
3. Validate initialization version and negotiated feature requirements.
4. Implement exact authentication and session setup/close behavior.
5. Run the agent only from an admitted executable/package digest; prohibit
   runtime downloads and shell interpolation.
6. Enforce mandatory Confinement, root mapping, environment allowlist, executable
   policy, egress and resource ceilings.
7. Route every reverse request through Auth-backed APXM Capability admission.
8. Preserve ordered updates, raw payload digests, redaction and terminal facts.
9. Distinguish cancel, close, process termination and uncertain effect outcome.
10. Ship exact Claude and Codex profiles only after their matrices pass; add no
    generic production command template.
11. Preserve generic/user-defined profiles only as migration, development, and
    configuration inputs; import them into exact profiles or reject them at
    production admission.
12. Pin one admitted MCP gateway descriptor per profile: local stdio or
    managed Streamable HTTP with audience-bound Auth and no token passthrough.

## 6. Server, Auth and Studio

The selected Runtime Profile and Deployment Composition Manifest predeclare
every exact model mapping. Managed Server replaces ambient registries with
authoritative catalogues and calls the same
`verify_deployment_composition(...)` contract available to standalone
Composition Roots; it validates and materializes the declared mapping before
runtime dispatch without searching or ranking candidates. `/v1/generate` and
any MCP/GAO path that calls a backend or ModelRouter directly are deleted or
rebuilt as ordinary Agent Program execution. No raw model fast path survives.

Auth owns adapter credential references, grants, approvals, workspace roots,
egress and revocation. Session and process state cannot outlive revoked
authority for new effects; reconciliation preserves historical facts.

Studio exposes:

- admitted profile catalogue and exact digest/version;
- negotiated feature and conformance matrix;
- connection/auth state without plaintext secrets;
- roots, executable and egress ceilings;
- grant/approval explanation;
- connection test;
- active/closing/lost/outcome-unknown sessions;
- cancel, close, revoke and terminate operations; and
- nested ACP timeline with provider-reported usage clearly separated from the
  APXM ledger;
- explicit `peer reported`, `derived estimate`, `provider reconciled`, and
  `cost unavailable` provenance/availability states; and
- no presentation of a seat/subscription or external MCP client's upstream
  model usage as APXM-billed cost.

Production UI accepts no arbitrary command or environment-secret field.

## 7. Test matrix

Positive vectors cover new session, optional negotiated load/resume, prompt,
ordered update types, reverse request, cancel, close, crash recovery and exact
model dispatch for every supported platform/profile.

Negative vectors cover version skew, missing required feature, malformed
message, duplicate id, out-of-order update, unknown extension, authentication
failure, workspace escape, symlink escape, forbidden command/env/egress,
revoked grant, denied permission, budget exhaustion, adapter mutation, runtime
download, process crash, cancellation race, close timeout and uncertain send.

Cross-layer assertions prove:

- one source call equals one outer Capability/model NodeExecution;
- ACP private loops do not become APXM nodes;
- external usage cannot alter Server ledger facts;
- ACP `used/size` never enters native model token accounting and no model
  identity is fabricated for the outer Capability;
- no automatic second profile/model is dispatched;
- exact identities survive recovery and Studio drill-down; and
- Python/TypeScript diagnostics and graphs are equivalent.

## 8. Cutover and point of no return

Before cutover, export and classify active prototype ACP sessions; no live
process/session is migrated. Drain or terminate them with explicit evidence.
Publish only the new exact profiles and bindings in the target Compatibility
Set. Remove old commands, configs, package ranges, generated ops, routes,
registries and docs before opening traffic.

The point of no return is the release-controller action that revokes old
adapter/model admission and admits the first external effect through the new
set. After it, repair is a new complete Compatibility Set; the prototype route
cannot be re-enabled.

## 9. Done

This plan is complete only when exact Claude Code, Codex and other admitted ACP
profile conformance journeys pass; exact model dispatch passes; Studio can explain and
control sessions; all authority/Confinement/evidence gates pass; and whole-tree
absence scans prove no old operation, router, fallback, arbitrary profile,
ambient registry, AAM prompt injection, false native token accounting,
fabricated model identity or direct-executor bypass remains.
