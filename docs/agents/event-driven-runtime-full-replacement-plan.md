# Event-driven runtime full-replacement plan

Status: canonical implementation plan under accepted workspace ADR-0027 and
Agents ADR-0018. Contract publication and isolated implementation work are
authorized, but no product implementation merges before `J1` and production
remains unreachable until `R4`. This plan targets one canonical cutover with no
runtime fallback, alias, translator, legacy reader, dual write, or mixed
contract generation.

Date: 2026-07-30

## Target ownership invariant

The migration must produce exactly this enduring separation:

- **Agents:** event/reference semantics, transition reducers, activation
  meaning, local readiness, and local scheduling/work stealing.
- **Server:** durable accepted occurrences, delivery/activation/effect work,
  leases, retry/DLQ, schedules, recovery, and managed observability.
- **Adapters, Host SDK, and Auth:** protocol execution, device/provider
  correlation, Host protocol meaning, and authority remain their own concerns.

Contracts indexes and generates exact owner publications without becoming a
fourth semantic owner. Studio and CLI consume generated operational projections
without owning event, delivery, execution, or recovery truth.

Within that split, Agents owns portable `EventOccurrence<T>`/
`EventProvenance` meaning and PXM transition reducers. Server owns the generic
`SourceContract`/`SourceReducerDescriptor`, registration/admission policy,
durable source records, and response obligations. Adapters and Host SDK own
protocol-specific source descriptors plus deterministic classification,
normalization, or reduction implementations. The ingress protocol owner may
physically emit an acknowledgement—through a Server route, admitted Adapter,
or Host gateway—only from Server's exact committed disposition. Host attachment
state is called gateway availability or dispatch eligibility; **readiness** is
reserved for Agents graph dependencies. APXM schedules remain Server-owned;
standalone Agents composition has no implicit scheduler.

## Documentation and completion rule

Target-state ADRs describe only the enduring owners and contracts above. They
do not narrate the removed product, its state, or the cutover. This plan and the
workspace amendment matrix exclusively own source inventory, transformation,
authority barriers, deletion, point-of-no-return, rollback, and negative-
reachability proof. Git history preserves superseded rationale.

This planning pass changes documentation only. No owner schema, generator,
consumer, product code, deployment, credential, state, or repository membership
changes before the ownership decisions are accepted.

The master's total-scope closure law is binding here. This plan has no
`out-of-scope`, deferred, optional-cleanup, or later-release category for a
migration-relevant item. `P2` classifies every declared repository and every
affected surface using the master's closed disposition tuple. A newly
discovered contract, consumer, durable row, credential, route, build/release
input, operational surface, document, or physical resource automatically joins
the owning lane and blocks the corresponding join until closed.

## Goal

Replace the current sequential canonical driver and OS-owned event/delivery
plane with:

- Agents-owned typed Program Event semantics, immutable execution plans,
  dependency readiness, local activation running, and high-performance local
  work stealing;
- Server-owned durable occurrence acceptance, delivery/target application,
  activation and effect-work persistence/leasing, scheduling,
  retry/DLQ/redrive/reconciliation, Host gateway, and operational queries; and
- one atomic full-replacement migration that removes every old OS and legacy
  scheduler path.

The target preserves the closed five-operation effect/composition AIS family.
It does not add an AIR queue, resume, scheduler, or work-stealing operation.

## Affected owners and surfaces

### Agents

- Governance and contract:
  - `docs/adr/0018-event-readiness-and-local-scheduling-are-agents-semantics.md`
  - `docs/agents/event-driven-runtime-and-scheduler-contract.md`
  - `docs/agents/agent-program-composition-and-air-contract.md`
  - `docs/agents/portable-core-interface-contract.md`
  - `contracts/descriptors/apxm.agents-owner-descriptor.v1.json`
  - new owned semantic schemas, vectors, and Port Contracts under `contracts/`
- Compile/admission:
  - `crates/machine/program/src/air.rs`
  - `crates/machine/program/src/artifact.rs`
  - `crates/machine/program/src/runtime_evidence.rs`
  - canonical AIR validation/lowering consumers
- Runtime replacement:
  - replace `crates/runtime/execution/src/structural.rs` flat scheduling
  - replace `crates/runtime/execution/src/driver.rs::drive_from`
  - replace the polling-shaped event boundary in
    `crates/runtime/execution/src/ports.rs`
  - replace schedule-position continuations in
    `crates/runtime/execution/src/resume.rs`
  - deepen `crates/runtime/kernel/src/commit.rs`
  - add focused execution-plan, readiness, activation-runner, local-executor,
    parker, and scheduler-observation modules under `crates/runtime/`
  - delete capability-owned scheduling in
    `crates/runtime/capability/src/builtins/{schedule,store}.rs`
  - remove stale scheduler coupling/comments from `capability-iface` and
    capability builtins
- Tests/benchmarks:
  - add focused concurrency/property/failpoint tests next to owning crates
  - add scheduler microbenchmarks under an Agents runtime crate `benches/`
  - store run output only below `.apxm/benchmarks/results/`

### Server and workspace consumers

- Deepen Server's strongest transaction in
  `workspace/server/crates/core/src/execution_commit.rs`.
- Replace split continuation/resume state in
  `workspace/server/crates/core/src/canonical_session/{continuation,supervisor}.rs`
  and `workspace/server/crates/core/src/event_resume_api.rs`.
- Replace the Integration pseudo-admission path in
  `workspace/server/crates/core/src/{integration_catalogue,integration_api}.rs`.
- Delete volatile `TaskQueueManager` in
  `workspace/server/crates/core/src/tasks.rs` after durable activation leasing
  is live.
- Replace Server's OS gateway and startup wiring in
  `workspace/server/crates/server/src/{os_gateway,startup}.rs` with a
  Server-owned Host gateway using Host SDK types.
- Migrate the valuable atomic occurrence/delivery/Host facts from
  `workspace/os/crates/os-state/`; remove the product from the active repo set,
  build inputs, and target topology in `J4`, then physically retire its
  already-fenced deployed remnants after cutover.
- Regenerate Contracts, Auth, Adapters, Host SDK, Studio, CLI, and coordinator
  consumers from their owners.

### Public API impact

Breaking and full replacement. Old OS routes, event-resume routes, generated
OS consumers, string-shaped event constructors, legacy scheduler APIs, and
OS-named configuration are deleted. New schema ids are used for changed
meaning; old ids are not aliases.

## Boundaries

The following are target-semantic prohibitions, not omitted migration work. If
a current surface implements one of them, that surface is in scope for
migration or deletion and its absence is proved at `J4`. This change does not:

- add a sixth effect/composition AIR operation;
- make Python or TypeScript a runtime;
- move Server durability, leases, ingress, or tenant policy into Agents;
- move Auth authority or secret custody into Agents or Server runtime code;
- make a transport acknowledgement equal Program fulfillment or success;
- claim exactly-once external effects;
- revive the removed `crates/runtime/engine` scheduler;
- use dynamic DAG splicing or a “requeue when inputs are missing” path;
- make Tokio task placement the semantic PXM scheduler;
- execute hard-real-time physical safety loops; or
- preserve a product fallback after cutover.

## Parallel execution model

This section governs implementation scheduling. The detailed phases below
specify behavior and evidence; they are not global barriers. A work item may
start as soon as its **start dependencies** are frozen in an owner branch, but
it may land only after its **landing dependencies** are integrated and green.
This distinction permits real parallel work without merging guessed contracts,
hand-editing generated consumers, or creating a mixed production path.

The identifiers in this section are temporary plan coordinates. They never
become schema ids, endpoints, configuration names, metrics, feature flags, or
coordinator campaign state. Git commits, owner-local tests, generated outputs,
and CI are the implementation record.

### Parallelism laws

1. Governance acceptance is the only global prerequisite for product work.
   Before it, only documentation, read-only inventory, benchmark exploration,
   and test-harness design may occur.
2. One semantic owner writes each owner contract. Other lanes may compile
   against an immutable frozen digest in an isolated worktree, but cannot land
   until that digest and its generated bindings are canonical.
3. Contract generation is one serialized cohort. Owner descriptors may be
   authored in parallel; Contracts writes its index/bundle once and each
   owner-local generator writes its own checked-in outputs once from the same
   exact landed digests.
4. Producers and consumers start from frozen owner interfaces in parallel and
   land only after generated types exist. No lane hand-writes a temporary DTO,
   translator, alias, or compatibility reader.
5. New target code remains unreachable from the production Composition Root.
   Parallel branches may contain complete target modules and tests, but no
   branch may receive ingress, write/effect authority, or fallback routing.
6. One file has one active writer. Disjoint modules in one repository may use
   separate worktrees; shared manifests, module roots, migrations, generated
   files, and startup/composition roots have a named integrator.
7. Owner-local correctness, security, recovery, and performance evidence runs
   continuously in parallel. Cross-plane gates join exact green revisions;
   they do not replace owner gates.
8. The authority barrier, final snapshot/transform, target activation, and
   point of no return are deliberately serial. Parallelism resumes only for
   post-activation physical cleanup and independent absence checks.

### Governance and contract DAG

| ID | Owner | Deliverable | May start after | May land after |
| --- | --- | --- | --- | --- |
| `P0` | Workspace governance | Accepted ADR-0027, Agents ADR-0018, Server ADR-0001, and required Auth/Host SDK owner decisions as one authority packet on 2026-07-30 | complete | complete |
| `P1` | Coordinator and each owner, one repo at a time | Rewrite live ADR indexes, contexts, master clauses, skills, and present-tense owner documentation to the accepted target; regenerate agent instructions where owned | `P0` | `P0` |
| `P2` | Every declared repository and affected owner | Pin revision/dirty state and assign every declared or undeclared checkout/worktree, schema, generator, producer, store/replay row family, generated/handwritten consumer, deployment member, credential, backup, operational surface, document and physical removal input a stable `P2/<repository>/<surface-kind>/<stable-slug>` coordinate and the master's closed disposition tuple; populate source, live-resource, state-disposition and release/physical manifest views | `P0`; repository/source-family/environment inventories may shard in parallel | `P1`; complete transitive classification, owner-signed fragments, no unclassified/out-of-scope/deferred item, and no coordinator campaign ledger |
| `S-AR` | Server data/evidence integrator with Auth security/redaction approval | Exact retention, immutable export, access/audit, redaction, terminal-reader and destruction policy for retiring audit evidence, Server quarantine rows, backups and transformer diagnostics; digest-pin the sole non-product migration corpus | `P2` state inventory may shard while policy is drafted | complete affected-row inventory; before `J1`; no item uses `archive-only` without accepted policy |
| `A-C1` | Agents | Portable Event/EventRef/occurrence/provenance, PXM reducers, target/application/activation/effect/commit/evidence contracts and executable vectors | `P0`, immutable ADR packet | `P1` |
| `S-C1` | Server | Source contract/reducer, disposition, occurrence, delivery/application, activation/lease, managed effect, schedule, Host gateway, retry/DLQ, query contracts and vectors | `P0`, immutable ADR packet; may draft against `A-C1` | `P1`, `A-C1` |
| `U-C1` | Auth | Exact verification consumer, decision-lease epochs/audiences, revocation, late-result, secret/redaction contracts and vectors | `P0` | `P1` |
| `H-C1` | Host SDK | Exact Server-facing Link method table, durable checkpoint interfaces, source contributions, credit/result/lifecycle vectors | `P0` | `P1` |
| `D-C1` | Adapters | Source-descriptor/contribution and provider effect idempotency/reconciliation implementation contracts and vectors | `P0` | `P1`, `A-C1`, `S-C1` |
| `C-G1` | Contracts transaction coordinator plus each owner-local generator writer | Verify exact owner descriptors, allocate changed schema ids, index one bundle, then run each owner generator once and publish all checked-in direct consumers byte-identically as one cohort | all owner contract and generator-entry-point branches plus sibling-portfolio schema inputs may prepare | `P2`, `A-C1`, `S-C1`, `U-C1`, `H-C1`, `D-C1`, Managed MCP `W6` contract/schema work; every declared repository's generation/impact disposition and every owner-local generator/drift gate green |
| `J1` | Cross-owner join | Freeze the one contract generation used by every implementation lane and record the exact Scope Closure Manifest digest | owner-local descriptor/vector gates and complete `P2` classification | `V-SCOPE`, `S-AR`, `C-G1`, generated-consumer drift green in every affected repo, and exact `prove-unaffected` evidence for declared repos without changed output |

`A-C1`, `U-C1`, and `H-C1` are independent and may land in parallel.
`S-C1` may be implemented concurrently but lands after the exact Agents values
it embeds. `D-C1` may prototype provider-owned behavior concurrently but lands
after both adjacent semantic contracts. `C-G1` is not sharded: parallel owner
publication ends at one deterministic generation transaction.

### Agents implementation lanes

| ID | Deliverable | May start after | May land after |
| --- | --- | --- | --- |
| `A-FE` | Unforgeable typed `Event<T>`/`EventRef<T>` through Python/TypeScript, BoundAgentTree, FrontendGraph, AIR wait, artifact, continuation, and evidence | `A-C1` frozen | `J1` |
| `A-PL` | Immutable `ExecutionPlan`, dynamic occurrence frames, and deterministic reference executor with Hook/Context/loop/effect parity | `A-C1` frozen | `J1`, `A-FE` |
| `A-RD` | Readiness kernel, operand publication, predecessor accounting, structured joins/cancellation, ActivityCredit, quiescence, and model tests | `A-PL` interface frozen | `A-PL` |
| `A-EC` | Execution Commit and prepared-effect reducers, commit/effect uncertainty separation, fences, and managed/standalone conformance seam | `A-C1` frozen | `J1` |
| `A-AR` | Storage-neutral `ActivationRunner`, exact claim validation, closed outcomes, checkpoint/end-of-activation semantics, and deterministic parity | `A-PL`, `A-EC` interfaces frozen | `A-PL`, `A-RD`, `A-EC` |
| `A-QP` | Pin and audit queue candidates; build loom/stress/benchmark harness and topology corpus without selecting a production fallback | `P0` | `A-C1` and a soundness decision |
| `A-LX` | Local executor, owner/thief deques, injectors/control inboxes, permits, fairness/aging, NUMA placement, park/wake, worker-loss fencing, and resource-pool isolation | `A-RD` interface frozen, `A-QP` | `A-RD`, accepted queue decision |
| `A-PF` | Digest-bound performance admission policy and complete scaling/NUMA/overload/reclamation evidence | benchmark harness from `A-QP`; may collect baselines early | `A-LX` |
| `A-CLI` | CLI uses exact generated compile, event, activation, approval, evidence, and lifecycle clients with no local semantic copy | `J1`; Server query/API shape frozen | `S-OQ`, generated clients |
| `A-RM` | Delete flat/sequential production driver, polling Event port, capability scheduler, string constructors, and superseded tests | deletion patch may prepare after `A-PL` | `A-AR`, `A-LX`, `S-RI`; target branch remains unreachable |

`A-FE`, `A-PL`, `A-EC`, and `A-QP` are separate initial worktrees. `A-RD`
uses the immutable plan interface while `A-LX` uses only the readiness output;
neither waits for Server storage. `A-PF` runs while Server integration proceeds.

### Server durable-module lanes

| ID | Deliverable | May start after | May land after |
| --- | --- | --- | --- |
| `S-SA` | Source registration/classification, presentation refusal, disposition journal, occurrence acceptance, idempotency/frontier, response obligation, content claims, and initial delivery transaction | `S-C1` frozen | `J1`, generated Auth/Adapter types; live interoperability joins at `V-E2` |
| `S-DA` | Delivery attempts, strict lanes, stable target application, EventRef reducer adapter, root admission and pre-fulfillment/fail-busy behavior | `S-C1`, `A-C1` frozen | `J1`, `S-SA`, `S-AK`, Agents vectors |
| `S-AK` | Activation store, atomic Instance claim, leases/epochs/fences, heartbeat, settlement, reclaim, eligibility, fairness, and recovery | `S-C1`, `A-C1` frozen | `J1` |
| `S-EW` | Managed prepared-effect work, `DispatchStarted`, attempts/results, uncertainty/reconciliation scheduling, and exact next-activation publication | `S-C1`, `A-EC`, `D-C1` interfaces frozen | `J1`, `A-EC`, `D-C1`, `S-AK`; concrete Adapters join at `V-AP`/`V-E2` |
| `S-SC` | Server-owned schedule/time engine, fire identities, UTC/timezone/DST/leap/catch-up/cancellation semantics | `S-C1` and `S-SA` occurrence interface frozen | `J1`, `S-SA` |
| `S-HG` | Durable Host gateway attachment/outbox/result/credit/takeover/gap/revocation state using generated Host SDK types | `H-C1`, `S-C1` frozen | `J1`, `H-GN` |
| `S-OQ` | Implement purpose-filtered operational queries, cursors, projections, and redaction behind the frozen OpenAPI contract; prove the cohort-generated Studio/CLI surfaces | query/OpenAPI schema from `S-C1` frozen | producing modules `S-SA`, `S-DA`, `S-AK`, `S-EW`, `S-SC`, `S-HG` |
| `S-RI` | Adapt Server lease to Agents claim, invoke `ActivationRunner`, enforce commit fences, settle closed results, and converge every source on one activation path | `A-AR`, `S-SA`, `S-DA`, `S-AK`, `S-EW`, `S-SC`, `S-HG` interfaces frozen | `A-AR`, `S-SA`, `S-DA`, `S-AK`, `S-EW`, `S-SC`, `S-HG` |
| `S-RM` | Delete direct execution, volatile task queue, separate event-resume execution, process-local Host result channels, old gateway projection, and superseded tests | deletion patch may prepare once replacements are named | `S-RI`, `S-SC`, `S-HG`, `S-OQ`; target branch remains unreachable |

`S-SA`, `S-AK`, and `S-HG` may land independently after `J1`. `S-SC` may start
against the frozen occurrence-admission interface but lands after `S-SA`.
`S-DA` depends on occurrence identity and Agents reducers, while `S-EW` depends
on the effect seam. `S-OQ` can build generated route/query scaffolding early,
but its producer-completeness tests land only with all six durable modules.

### Authority, protocol, consumer, topology, and transform lanes

| ID | Owner | Deliverable | May start after | May land after |
| --- | --- | --- | --- | --- |
| `U-AU` | Auth | Implement exact Server operation-scoped consumers, decision leases/epochs, pre-send revalidation, revocation races, late-result recording, and redaction | `U-C1` frozen | `J1` |
| `U-BR` | Auth | Build issuance/revocation of replacement credentials and the signed old-authority barrier; do not execute it | `U-AU`, `P2` | `J1` and owner tests; rehearsal contributes to `J5`, execution is `R2` only |
| `H-GN` | Host SDK | Consume the cohort-generated Rust/Python/TypeScript Server bindings; implement the conformance harness, durable-store injection, and resume/takeover/gap/credit/late-result histories | `H-C1` frozen; generated bindings may prepare only in `C-G1` | `J1` |
| `D-SI` | Adapters, one lane per source family | Implement exact source descriptor, interpretation, classification/normalization/reduction contribution, checkpoint/gap policy, and response-from-committed-disposition behavior | `D-C1`, `S-C1` frozen | `J1`, `S-SA` interface |
| `D-EF` | Adapters, one lane per effect provider/device | Implement restart-safe send/status/idempotency/cancellation/reconciliation with typed uncertainty and no blind resend | `D-C1`, `A-EC` frozen | `J1`, `A-EC` |
| `D-ST` | Adapters | Implement a separately admitted standalone durability adapter for the Agents-owned EventRef/Execution Commit Port Contracts, with no Server storage type, managed lease, schedule engine, or production fallback | `A-C1`, `A-EC` interfaces frozen | `J1`, `A-EC`; shared histories gate `V-E2` |
| `T-CL` | Studio | Consume the one cohort-generated Server operational client, then implement occurrence/delivery/application/activation/effect/schedule/Host/evidence UX in disjoint feature modules | `S-OQ` contract/OpenAPI shape frozen | `S-OQ`, generated-client drift green |
| `P-CU` | Plugin | Inventory and rewrite/delete every packaged document, Skill, reference, test and generated owner-bundle consumer that teaches the retired trigger-sidecar/event-loop owner; regenerate and sign the catalogue against target APIs | `P2`; target Server vocabulary and `S-OQ` contract frozen | `J1` cohort where consumed, `S-OQ`, deterministic package/signature regeneration; absence joins `J4` |
| `K-TO` | Coordinator | Prepare target-only native/Compose topology, health, configuration, backup/restore, dashboards, repo map, wrappers, skills, and smoke with no removed member | Server/Host interfaces frozen; may prepare unreachable composition | `J3`; included in `J4` and activated only in `R4` |
| `M-TX` | Server-owned offline migration tooling, never product-linked or shipped | Deterministic old-to-new identity/state transformer consuming a pinned source export, with restart journal, partition manifests, counts/frontiers/Merkle roots, archive/invalid classification, and rollback verification | `P2`, target identities frozen at `J1` | source/target store schemas frozen; rehearsal `J5` |
| `X-DL` | Per-owner deletion join, not a separate writer | Each owner prepares deletion of its replaced contracts/schema ids, routes, readers/writers, queues, generated and handwritten consumers, credential references, configuration/env, repo/fanout/commands, metrics/dashboards, tests/fixtures/examples, docs/vocabulary, packages/images/SBOM inputs and runbooks in its own lane; coordinator-owned registry/topology/skill removal belongs only to `K-TO` | complete `P2` inventory, replacements named and negative/absence tests written | `J3`; every deletion plus `archive-only`/`prove-unaffected` proof joins `J4`, never deferred as compatibility cleanup |
| `RC-AG`…`RC-RS` | Each release integrator named in the master | Emit the exact per-owner artifact/package/image/generated-bundle/SBOM/provenance/signature/install evidence or retiring-source target-absence/rollback-custody proof | owner functional/release surfaces frozen | `J3`, complete owner gates and `P2` release view; consumed by `RC-AS` |
| `RC-AS` | Release controller | Assemble the unreachable candidate from clean owner revisions, `J1` cohort, Scope Closure Manifest, supported-environment matrix and every `RC-*` result | all `RC-*` inputs may prepare in parallel | complete owner evidence; verified by `V-RC` before `J4` |

Adapter lanes may run independently per exact provider/device/source directory.
They serialize only on shared workspace manifests, descriptor indexes, and lock
files. Studio feature work starts from one generated client snapshot; no feature
branch regenerates it. `M-TX` may be built while runtime work proceeds because
it depends on frozen identities and store schemas, not on the local executor.

### Verification lanes and join gates

| ID | Owner(s) | Evidence that runs in parallel | Join condition |
| --- | --- | --- | --- |
| `V-SCOPE` | Every declared repository/environment; release controller joins owner attestations | Stable-coordinate completeness, transitive dependency closure, revision/dirty pins, live binding/resource discovery, state/release views, archive policy, duplicate/missing coordinate checks and manifest reproducibility | before `J1`, then repeated on the exact candidates at `J4` and `J5`; identical Scope Closure Manifest digest everywhere |
| `V-CT` | Every declared repository, semantic owner and Contracts | Scope-disposition, positive/negative/mutation/vector, unknown-field, digest, generator-drift, cross-language parity and exact `prove-unaffected` suites | every `P2` item is classified and all exact owner/consumer digests agree with `J1` |
| `V-AG` | Agents | Frontend parity, deterministic executor, model/loom, randomized parallel parity, cancellation/quiescence, worker loss, and evidence tests | `A-FE` through `A-LX` green |
| `V-SV` | Server | Transaction failpoints, split-brain/fence, source capacity/gap, strict-lane, schedule, Host, backup/restore, saturation, and crash recovery | `S-SA` through `S-OQ` green |
| `V-AP` | Auth, Host SDK, Adapters | Audience/epoch/revocation/security/redaction, Host lifecycle/credit, provider duplicate/checkpoint/uncertainty, and physical-edge freshness/isolation | `U-AU`, `H-GN`, all admitted `D-*` lanes green |
| `V-CU` | Studio, CLI and Plugin | Generated-client/bundle drift, typed-state coverage, replay-to-live/cursor, redaction, route/vocabulary absence, accessibility, deterministic signed catalogue packaging and installed-Skill smoke | `T-CL`, `A-CLI`, `P-CU` green |
| `V-PF` | Agents and Server | Local scheduler plus durable admission/lease/effect throughput, p50/p95/p99, fairness, NUMA, overload, memory, no-busy-polling, and topology-class admission | `A-PF` and Server capacity policies green |
| `V-E2` | Cross-plane | Every event case from ingress through occurrence, application, activation, effect/commit, evidence, reconnect/replay, and operational clients; managed/standalone parity | exact functional revisions for `A-FE`, `A-PL`, `A-RD`, `A-EC`, `A-AR`, `A-LX`, all `S-*` replacement modules through `S-RI`/`S-OQ`, `U-AU`, `H-GN`, every admitted `D-SI`/`D-EF`, `D-ST`, `T-CL`, and `A-CLI`; excludes removal, topology, barrier, transform, rehearsal, and release lanes; no production authority |
| `V-MG` | Transformer and operators | Empty/populated transforms, crash/restart, counts/frontiers/Merkle roots, live digests, rollback rehearsal, and stale-writer negative reachability | exact `J4` candidate, `M-TX`, `U-BR`, target store schemas, and deletion inventory green |
| `V-RC` | Release controller plus independent owner release reviewers | Candidate-manifest closure, clean revisions, private distribution, SBOM/provenance/signature inputs, install/upgrade evidence and exact supported-environment matrix | `RC-AS` reproduces every digest and contains one target generation with no missing owner artifact |
| `V-LA` | Each owner signs its absence; coordinator joins only coordinator-owned hygiene/topology | Negative reachability for replaced source/contracts/generators/readers/writers/routes/config/credential refs/repo/build/release members/packages/images/generated consumers/docs/skills/tests; exact non-product migration-corpus exception | J4 Logical Absence Manifest covers every `P2` coordinate and contains no broad allowlist or dormant member |
| `J2` | Agents and Server | Owner kernels are independently complete | `V-AG`, `V-SV`, `V-AP`, `V-CT` green on exact revisions |
| `J3` | Cross-plane | One source-to-commit path is integrated and all consumers understand its exact states | `S-RI`, `S-OQ`, `T-CL`, `A-CLI`, `V-CU`, `V-E2` green |
| `J4` | Release candidate | Target code, every owner deletion/proof, topology, configuration, credentials-by-reference, generated consumers, artifacts, packages/images, descriptors, SBOM/provenance inputs, operations/docs and absence tests form one unreachable target Compatibility Set candidate | `J2`, `J3`, repeated `V-SCOPE`, `V-PF`, `A-RM`, `S-RM`, `P-CU`, `K-TO`, `X-DL`, `RC-AS`, `V-RC`, `V-LA`; no unclassified item, old reader/writer, fallback or dormant member in target |
| `J5` | Release rehearsal | The exact `J4` candidate installs, upgrades/transforms, backs up/restores, fails over, rolls back before PONR, and passes security/capacity/recovery in every supported environment | exact `J4`, parallel environment rehearsals, repeated live-resource discovery and `V-SCOPE`, and `V-MG` green; contributes to global `J-V1` |

Testing follows the implementation rather than waiting for a final test phase.
For example, loom work starts with `A-RD`, failpoint scaffolding starts with
`S-SA`, Host vectors start with `H-C1`, and transformation fixtures start with
`P2`. `V-E2` is the first cross-plane semantic join; `dekk apxm verify` remains
the final coordinator/delegation check, not a substitute for these owner gates.

### Earliest-start waves

A wave is the earliest point an item may start, not a barrier. Work in a later
wave starts as soon as its own dependencies are frozen; it does not wait for
unrelated items in an earlier wave.

| Wave | Work that can run concurrently | Required join before the next critical edge |
| --- | --- | --- |
| `W0` authority | `P0` only; continue read-only `P2` discovery, `A-QP` exploration, validation-harness design | accepted owner packet |
| `W1` contract publication | `P1`, per-repo/per-surface/per-environment `P2`, `S-AR`, `A-C1`, `S-C1` drafting, `U-C1`, `H-C1`, `D-C1` and sibling-portfolio schema drafting, queue/benchmark baselines, transform inventory | `V-SCOPE` over the populated manifest and owner contracts land in dependency order |
| `W2` generation and foundations | serialized `C-G1` owner-writer cohort; in isolated worktrees begin `A-FE`, `A-PL`, `A-EC`, `A-QP`, `S-SA`, `S-AK`, `S-SC`, `S-HG`, `U-AU`, `H-GN`, `D-SI`, `D-EF`, `D-ST`, test fixtures | `J1` before producer/consumer merges |
| `W3` independent engines | `A-RD`, `A-AR`, `A-LX`, `S-DA`, `S-EW`, Host/Adapter families, query/client scaffolding, `M-TX`, owner verification lanes | exact local interfaces and owner gates |
| `W4` integration and consumers | `S-RI`, `S-OQ`, `T-CL` feature lanes, `A-CLI`, `P-CU`, `A-PF`, `K-TO`, sibling-portfolio seams, deletion branches, cross-plane E2E and recovery/security/performance matrices | `J2`, then `J3` |
| `W5` target release candidate | integrate `A-RM`, `S-RM`, `P-CU`, `X-DL`, target topology, generated clients and `RC-*`; run `V-SCOPE`, `V-RC`, `V-LA` and complete conformance | `J4` |
| `W6` rehearsals | supported-environment install/upgrade/transform, empty/populated data, backup/restore, capacity, fault, security, rollback, and stale-writer rehearsals in parallel | `J5` |
| `W7` global join and authority transfer | Agentification/Managed-MCP candidate gates join event `J5` at `J-V1`, then serial `R1` through `R4` below | point of no return |
| `W8` physical cleanup | named owner `PC-*` tasks independently remove already-fenced store/stream/credential/volume/dashboard/archive/deployment objects | `J6` final signed Physical Absence Manifest |

Duration estimates do not yet justify calling one branch the critical path.
The precedence-critical joins are:

```text
P0 -> P1 -> P2 -> max(
  A-C1 -> S-C1 -> D-C1,
  U-C1,
  H-C1,
  every owner-generator preparation,
  all sibling-portfolio contract/schema inputs
) -> (S-AR + V-SCOPE + C-G1) -> J1 ->
  max(
    A-FE -> A-PL -> A-RD -> A-AR,
    (A-QP + A-RD) -> A-LX -> A-PF,
    (S-SA + S-AK) -> S-DA,
    (A-EC + D-C1 + S-AK) -> S-EW,
    H-GN -> S-HG,
    S-SA -> S-SC
  )
-> (J2 + J3 + V-PF + A-RM + S-RM + P-CU + K-TO + X-DL
    + RC-* + V-RC + V-LA + repeated V-SCOPE) -> J4
-> (environment rehearsals + (M-TX + U-BR -> V-MG)) -> J5
-> global J-V1 -> R1 -> R2 -> R3 -> R4 -> parallel owner PC-* -> J6
```

Host gateway, Auth, Adapter, schedule, Studio/CLI, topology, transformer, and
deletion lanes are parallel unless they fail to meet their named join. None is
optional merely because it is off the longest-duration branch.

### Repository conflict and serialization matrix

| Repository | Safe concurrent ownership | Single-writer or ordered surfaces |
| --- | --- | --- |
| Agents | frontend/compiler event flow; execution plan; readiness; commit/effect reducer; queue research; local executor; benchmarks; CLI, each in disjoint modules/worktrees | owner descriptor/schema ids/vectors, `Cargo.toml`/lockfile, module roots, runtime evidence, `driver.rs`, `structural.rs`, `resume.rs`, `ports.rs`, kernel commit, shared snapshots; one Agents integrator lands `A-FE -> A-PL/A-EC -> A-RD/A-AR -> A-LX -> A-RM` |
| Server | source admission, delivery/application, activation store, effect work, schedule engine, Host gateway, queries can use separate new modules | owner descriptor, SQL migration numbering/schema, Execution Commit adapter, shared error/types, OpenAPI root, routes, `startup.rs`, Cargo files; one Server integrator lands schema/store modules before `S-RI`, then `S-RM` |
| Auth | exact-consumer/decision implementation, security vectors, and barrier rehearsal may use separate modules | one Auth schema/issuer integrator owns the descriptor, issuer/audience registry, migrations, credential issuance/revocation root and generated contracts |
| Host SDK | conformance histories, language-specific convenience layers, and store implementations may run separately after the method table freezes | one Host SDK protocol/generator integrator owns protocol source, templates, method table, schema ids and generated Rust/Python/TypeScript outputs |
| Adapters | one worktree per exact provider/device/source adapter and one for the standalone durability adapter | one Adapters workspace/descriptor integrator owns the manifest/lockfile, shared descriptor index and common fixtures; common code cannot acquire provider dispatch semantics or Server managed-plane semantics |
| Contracts | owner submissions are prepared outside this repo | the descriptor index, schema-id registry, Contracts generator/bundle, and Contracts-owned generated outputs are its one writer segment inside `C-G1`; owner-local outputs retain their own writers |
| Studio | occurrence, delivery, activation, effect, schedule, Host, and evidence feature projections may run separately after client freeze | one Studio client/navigation integrator owns the Server generated-client/OpenAPI snapshot, routing/state vocabulary, package lockfiles, shared navigation and design-system files |
| Plugin | docs, individual Skills/references and tests may be rewritten in disjoint content worktrees after target vocabulary freezes | one Plugin catalogue/signature integrator owns the owner-bundle input, catalogue index, package manifest, signatures/digests and installed-content fixtures |
| vLLM | inference/effect conformance and impact analysis may run independently; consumer changes begin only when `P2` proves an affected generated surface | one vLLM release integrator owns any affected generated consumer, manifest/lockfile, shared build root, package/image/SBOM/provenance input and release artifact; otherwise the same role signs the revision-pinned `prove-unaffected` result |
| Coordinator | accepted docs/skills and read-only topology analysis are disjoint from product repos | `repos.toml`, `.dekk.toml`, Compose files, composition renderer/output, stack scripts, generated agent docs, repo/skill removal; one topology integrator |
| Offline transformer | read-only inventories and transformation rules per old table/stream may be authored independently | one executable, identity map, journal, partition manifest, output writer, counts/Merkle root and final signed report; never linked into a product |

Concurrent lanes reserve file ownership in their owner change before editing.
If two lanes need the same listed hotspot, the earlier dependency owner edits it
and the later lane rebases; they do not both modify it and resolve semantics in
a merge conflict. Generated files are never conflict-resolution surfaces.

### Generation cohort protocol

Generation is a global transaction, not feature-branch work. The current
generator surfaces still include consumers of the replaced generation and
cannot be safely sharded. The implementation therefore establishes and uses
this protocol:

1. Each semantic owner lands its frozen descriptor, schema sources, vectors,
   and generator inputs at one immutable revision. The cohort manifest records
   every owner revision and digest before generation starts.
2. Missing owner-local generator entry points are added and verified before
   coordinator fanout delegates to them. Contracts may index and generate an
   owner's publication, but it never acquires that owner's semantics.
3. A dedicated clean generation cohort checks out all affected repositories at
   exactly the recorded revisions. No dirty feature worktree and no worktree
   with an unpublished descriptor may participate.
4. The Contracts writer begins `C-G1` once: validate owner inputs, allocate
   changed ids, produce the indexed bundle, and verify a second clean Contracts
   generation is byte-identical.
5. Still inside `C-G1`, Host SDK, Agents, Server, Auth, Adapters, Studio, CLI,
   every Plugin owner-bundle-derived generator, and any vLLM consumer whose
   `P2` impact proof requires regeneration consume that exact bundle/digest
   cohort in dependency
   order, with a byte-identical clean rerun. All outputs from one owner—
   especially the Host SDK method table, templates, and
   Rust/Python/TypeScript bindings—are one owner-written transaction.
6. Generated outputs return to their owner repositories as one cohort. Feature
   branches rebase onto the cohort; they never regenerate, cherry-pick only
   part of it, or resolve generated output conflicts by hand.
7. `dekk apxm sync` is a separate coordinator-wide generated-file lock. It runs
   once per intentionally frozen source-input cohort, never from a feature
   worktree and never concurrently with `C-G1` or owner generation. `P1` owns
   the accepted-governance instruction cohort; `K-TO` owns the final target-only
   repo/skill-removal cohort admitted to `J4`. Any intervening source change
   invalidates its generated instruction set and requires a newly pinned
   cohort, not an incremental hand edit.
8. Any owner input revision or digest change invalidates all downstream
   generated artifacts, cross-plane results, release candidates, and migration
   rehearsals. The owners freeze again, generation runs once, and consumers
   rebase; there is no mixed-generation grace period.

`J1` names the immutable result of this protocol. Product implementation may
start earlier against frozen interfaces in isolated worktrees, but no
product implementation merge occurs before `J1`.

### Worktree and integration protocol

Parallel execution uses exact bases and explicit convergence ownership:

1. Before implementation, preserve every current dirty change on an explicit
   owner branch. Do not build this migration by resetting, cleaning, or
   absorbing unrelated work.
2. After the governance packet lands, record one clean accepted base revision
   per repository. Create one owner-local isolated worktree per lane from that
   exact base; the lane record names its files and named integrator.
3. Build the generation cohort in adjacent exact-revision worktrees so
   cross-repository commands cannot silently resolve a developer's unrelated
   checkout. Join repositories only by immutable revision plus contract digest,
   never by a mutable branch name or “latest” workspace state.
4. Give each runnable worktree unique build/cache directories, runtime state
   roots, database/schema names, broker namespaces, ports, credentials, and
   benchmark result roots. A lane must not observe or mutate another lane's
   process or durable state.
5. Merge leaf modules and their owner-local tests first. Only the named
   repository integrator edits shared manifests, lockfiles, SQL migration
   numbering, module roots, route/OpenAPI roots, startup/composition roots,
   generator roots, and deletion hotspots listed above.
6. The integrator rebases the converged leaf set onto the exact generation
   cohort, performs the hotspot edits once, and runs repository-local gates.
   Semantic disagreements return to the owning lane; they are not settled in a
   mechanical merge conflict.
7. A changed owner digest or base revision invalidates dependent branches. Run
   generation once where required, then rebase and rerun affected evidence;
   never introduce a temporary DTO, compatibility shim, shadow writer, or
   alternate scheduler to keep stale branches moving.
8. Cross-plane joins consume only clean owner revisions, the common cohort
   digest, owner-local gate reports, and explicit production-unreachability
   proof. Rehearsal manifests pin the same tuple so results cannot be credited
   to a different candidate.

This protocol permits many coding and verification lanes at once without
pretending that a generator, migration sequence, lockfile, or Composition Root
has more than one writer.

### Lane hand-off contract

Every implementation lane hands the next lane:

- exact input contract/descriptors and digests;
- its accepted base revision, owner repository revision, generation-cohort
  digest, owned file list, and named integrator;
- typed outputs/failures and authority/data/network/process ceilings;
- generator command and generated outputs where applicable;
- positive, negative, mutation, replay/recovery, redaction, and performance
  evidence appropriate to the lane;
- a statement that the target path remains unreachable from production; and
- the superseded files/identifiers its deletion lane will remove.

A downstream lane rejects a hand-off with a changed digest, patched generated
file, missing production producer, unowned state transition, or incomplete
failure semantics. It does not add a temporary adapter or fallback.

### Serialized release window

All implementation and most verification is parallel before this window. The
release controller then performs exactly one ordered authority transition:

| ID | Required serial action | Parallel work forbidden during the action |
| --- | --- | --- |
| `R1` | Freeze exact clean revisions, sign the target Compatibility Set candidate, verify backups/snapshots and the rehearsed rollback package | owner merges, schema regeneration, new deployment artifacts |
| `R2` | Apply the signed barrier: stop and revoke/fence every old ingress, response writer, source/activation/effect worker, Host attachment, credential and store writer | either generation accepting, acknowledging, dispatching, or writing externally |
| `R3` | Take the final snapshot, run the one journaled transformer, reconcile counts/frontiers/DLQ/Host sequences/live digests/Merkle roots, and prove target integrity plus stale-writer negative reachability | old readers/writers, target writers, independent partial transforms, guessed repair |
| `R4` | Activate the complete target Composition Root once; the first irreversible target write, acknowledgement, Host frame, or external effect is the PONR | partial owner activation, mixed generation, per-service rollback |

Before `R4`, failure restores the whole pinned prior Compatibility Set from the
verified snapshot with fresh higher-epoch credentials. After `R4`, recovery is
forward-only. Post-PONR physical deletions may execute in parallel by owner only
after the target remains healthy, because none can be part of a rollback path.

The post-PONR coordinates are `PC-AGENTS`, `PC-SERVER`, `PC-AUTH`, `PC-HOST`,
`PC-ADAPTERS`, `PC-CONTRACTS`, `PC-STUDIO`, `PC-PLUGIN`, `PC-VLLM`, `PC-COORD`
and `PC-RETIRING-SOURCE`, with target classes exactly as defined in the master.
Every task starts after `R4`, the signed target-health interval and the relevant
retention release; resolves exact destructive identities from the repeated live
resource inventory; and emits before/after absence evidence or a signed zero-
item proof. Obsolete local/CI checkouts and worktrees are exact physical
coordinates, not an implicit developer cleanup. The release-controller-owned
`J6` reruns discovery and signs the
final Physical Absence Manifest only after every coordinate closes. These tasks
cannot delete product source/configuration/repository members because `V-LA`
and `J4` have already proved those absent.

## Detailed work-package specifications

The numbered phases below define the required behavior and tests referenced by
the DAG. They do not override the start/landing dependencies, file ownership,
join gates, or serialized release window above.

### Phase 0 — accept ownership before code

1. Accept a workspace ADR that amends every active clause assigning durable
   ingress, scheduling, delivery, Host relay, retry/DLQ, or replay to OS.
2. Accept Agents ADR-0018 and a corresponding Server owner ADR plus required
   Auth and Host SDK owner amendments.
3. After acceptance, rewrite affected current ADRs and ADR indexes as target-
   state records with no removed-product vocabulary, then update canonical
   present-tense context/master-plan claims. Do not change those accepted
   surfaces before governance accepts the ownership decision.
4. Pin all affected repo revisions and dirty state. Record owner schema,
   generator, producer, store/replay path, generated consumer, deployment, and
   removal ledger as populated stable `P2` coordinates, including live deployed
   resources rather than source inspection alone.
5. Publish a clause-by-clause amendment matrix covering every affected accepted
   ADR, master-plan/context entry, owner descriptor, generated consumer,
   deployment member, credential, migration, backup, dashboard, and runbook.

Verification:

- ADR link/status validation;
- ownership table has exactly one owner per semantic fact;
- no accepted document assigns the same fact to both OS and Server;
- dirty user work is identified and preserved.

### Phase 1 — freeze owner contracts and identities

1. Agents preserves public `Event<T>` as the typed reference value and
   publishes the internal `EventTypeRef`/schema digest, generation-scoped
   EventRef reservation/wait/fulfillment/consume/abandon transitions, execution
   plan, `RootOccurrence | AwaitFulfillment` activation causes, storage-neutral
   activation claim, runnable activation envelope, effect state machine, runner
   result, portable occurrence/provenance meaning, PXM transition reducers, and
   authoritative runtime evidence. Business event definitions form an open
   content-addressed catalogue over these closed mechanics; they are never
   runtime enum variants.
2. Agents deepens the Execution Commit Port with expected activation/Program
   revisions, closed event transitions, prepared-effect facts, and activation
   consequences while exposing no Server storage type.
3. Server publishes `SourceContract`, `SourceReducerDescriptor`,
   occurrence-candidate admission, `AcceptedOccurrenceRecord`, source
   response/frontier/committed-disposition, delivery/attempt, target
   application, durable activation/lease, managed effect work/attempt,
   ordering lane, schedule/timer, DLQ/redrive, Host outbox/result, and
   operational query schemas. Its record embeds exact Agents values/digests;
   it does not co-own their semantic state machines.
4. Auth, Adapters, and Host SDK amend only their owned verification, provider,
   and protocol contracts. Adapter/Host publications contain the exact
   protocol-specific source descriptors and classification/normalization/
   reduction implementations; transport responders act only from Server's
   committed disposition.
5. Each owner first exposes a deterministic owner-local generator entry point.
   Contracts then indexes owner descriptors and coordinates the one `C-G1`
   cohort across those writers. Generated files are never hand-patched.
6. Agents publishes executable transition/reducer vectors used by both the
   managed Server transaction adapter and a genuine standalone durability
   adapter; a schema-only reimplementation is insufficient.
7. Define exact Auth decision-lease epochs, schedule/time semantics, live-
   digest retirement, Host lifecycle, root `T -> I` binding proof, and EventRef
   reservation co-commit before implementation.
8. Freeze the frequency-independent admission rule: each exact source declares
   whether every item is an occurrence or whether reduction/windowing/sampling
   happens before admission. Accepted occurrences are never silently dropped
   or coalesced; exhausted capacity produces a typed source disposition.

Verification:

- `dekk agents owner-descriptor-sync` then `dekk agents owner-descriptor`;
- executable state-machine/vector tests for EventRef, occurrence/source,
  delivery/strict lanes, activation/checkpoint/reclaim, effect dispatch/
  reconciliation, schedule/time, Host result, Auth revocation, and digest
  retirement, plus negative tests for unknown variants/cross-type confusion;
- generator drift checks in every owner repo;
- scans prove no Server table/route/broker name enters Agents semantic schemas;
- scans prove no authority/secret value enters Event payload or local queues.

### Phase 2 — build the deterministic readiness reference

1. Replace string-constructed frontend/runtime event refs with unforgeable typed
   `Event<T>`/`EventRef<T>` value flow through BoundAgentTree, FrontendGraph,
   AIR `await.event`, artifact requirements, continuations, and evidence. Root
   input remains the ordinary typed artifact entry input; no sixth AIR op or
   transport-specific frontend construct is added.
2. Replace flat `ScheduleStep` meaning with immutable `ExecutionPlan`
   construction and validation.
3. Implement dynamic occurrence frames, exact operand slots, predecessor
   accounting with slot `Empty -> Writing -> Published`, explicit
   zero-predecessor admission, cancellation races, activity-credit quiescence,
   closed node lifecycle, structured cancellation, and joins.
4. Implement a deterministic single-thread executor as the semantic reference.
   It is a test/conformance executor, not a production fallback.
5. Port Hook ordering, Context transitions, loop occurrence identity,
   NodeExecution evidence, model/Capability/program effects, yield/return, and
   Execution Commit to the new plan.
6. In a separately admitted Adapter crate, implement a genuine standalone
   durability adapter for the same Agents EventRef/Execution Commit Port
   Contracts and histories. It supplies portable local persistence, not Server
   managed leases/schedules, and is never a fallback selected after managed
   execution failure.
7. Remove the single-shot behavior that records a parked `await.event` and then
   continues execution.

Verification:

- existing canonical execution tests migrated with no old driver call;
- deterministic result/evidence parity for all legal sequential graphs;
- property tests for branch, loop, nested structured tasks, cancellation, and
  dynamic occurrence ids;
- managed Server and standalone Adapter executions produce identical Agents-
  owned semantic transitions and evidence for the same contract histories;
- loom/model tests for slot-claim/duplicate-writer, last-predecessor,
  zero-predecessor, cancellation, queue-publication, activity transfer,
  quiescence, and result-publication races;
- negative tests reject underflow, saturating decrement, missing operands,
  duplicate runnable publication, and stale fences.
- absence gates reject `Event<T>(string)`, Python `Event[T]("...")`, default
  payload type `"Event"`, optional/missing event-wait operands, generic
  declaration `target_ref` as an EventRef, and legacy static Event declarations;
  no compatibility translator reads them.

### Phase 3 — implement the high-performance local executor

1. Evaluate queue candidates at exact source/checksum. Review Rust memory-model
   soundness, weak-memory behavior, reclamation, cache-line layout, overload,
   panic/loss behavior, and model-test evidence. Do not accept the currently
   pinned `crossbeam-deque` 0.8.6 without resolving its acknowledged volatile
   concurrent-access/data-race concern.
2. Add one owner-local stealable deque per compute worker per service class,
   per-class/domain injectors, a separate owner-only pinned/control queue, and
   a bounded MPSC control inbox per worker with publication-before-wakeup.
   Use opposite-end capped batch stealing only if the selected implementation
   proves it soundly.
3. Implement bounded local victim selection, last-success locality, rotated
   persistent permutation cursors with bounded coverage, bounded remote-domain
   stealing, and steal hysteresis.
4. Implement bounded spin/yield/park and publication-before-wakeup with no
   lost wakeups.
5. Implement closed priority classes, weighted/deficit service,
   aging/starvation bounds without whole-deque scans, non-empty eligible-class
   masks, per-activation dispatch tokens plus bounded ready backlogs, atomically
   reserved activation/frame and per-domain runnable-slot permits, atomic
   remote-steal permit transfer, lossless dynamic-capacity waits, and blocking-
   I/O isolation.
6. End the activation at every external-effect preparation commit. Compute
   workers never dispatch or await external effects; durable effect workers and
   reconciliation publish the exact next activation.
7. Pin workers within the effective cpuset, define NUMA first-touch/allocation
   and topology-change behavior, and isolate compute, blocking-I/O,
   accelerator, Host-streaming, and controller real-time resource pools.
8. Define worker-panic/loss policy: v1 fences the runtime instance and every
   active claim, retains frames/permits until stale entries drain, and lets only
   the durability owner revoke/reclaim from the last durable checkpoints. It
   never continues after losing a private queue or partially written slot.

Verification:

- loom/model tests for park/wake, shutdown, cancellation, ActivityCredit
  terminal ownership, remote permit transfer, and queue-publication races;
- deterministic-reference parity under randomized worker schedules;
- stress with duplicate completion, burst fan-out, skew, cancellation, and
  worker loss;
- microbenchmarks for dependency release, local pop, steal, batch size,
  park/wake, fork/join, effect completion, and activation start;
- scaling at 1/2/4/8/available cores plus NUMA/SMT/oversubscription matrices;
- p50/p95/p99 ready-to-start, throughput, scheduler ns/task, steal success and
  distance, park/unpark rate, wake amplification/idle CPU, queue-age tails,
  per-activation fairness, break-even granularity, LLC/remote-DRAM counters,
  peak/RSS reclamation, overload recovery, effect-fence latency, and memory
  ceilings recorded with machine/topology/cpuset/governor/allocator/corpus and
  confidence metadata.

Before benchmark execution, check in a digest-bound threshold policy naming the
fixed corpus/profile digests, supported topology classes, deterministic and
non-stealing baselines, normalized throughput/scaling minima, p99/fairness
regression maxima, victim-probe and batch caps, class-dispatch and activation
caps, idle/wake budgets, and memory/RSS reclamation ceilings. Benchmark output
itself remains under `.apxm/`. A topology/profile without a passing policy
result is not admitted for production parallel execution and does not select a
fallback scheduler.

### Phase 4 — implement Server durable occurrence and activation truth

1. Build a Server transaction adapter that executes/proves conformance with the
   Agents transition contract while keeping Server storage types out of Agents.
2. Atomically accept/deduplicate an occurrence and create the immutable
   occurrence, exact response obligation, source-position disposition/frontier
   consequence, and exactly one initial delivery. Acceptance does not directly
   create an activation. Separate non-durable `AttemptPresented` refusal from
   `AttemptAdmittedToDispositionJournal`; the latter begins only after atomic
   reservation of disposition, payload/provenance/evidence bytes, idempotency/
   frontier state, response/outbox, and initial-delivery capacity. An admitted
   attempt commits accepted, duplicate, handshake, no-event, permanent-reject,
   authorized-skip, committed-gap, committed-reset, or transient-failure. Only
   source-contract-declared terminal dispositions acknowledge or advance a
   frontier; backpressure and transient failure stay unresolved.
3. Implement fenced delivery and stable `TargetApplicationRef`. Root
   application atomically creates root admission/Invocation plus initial
   activation. Await application atomically records EventRef fulfillment and
   creates the unique activation only when the exact wait is bound. Closed
   application outcomes include applied/already-applied, target-closed,
   Program-Instance-busy, identity-conflict, authority-unavailable/denied, and
   stale-binding-generation. Busy is fail-busy, never a hidden mailbox or
   unbounded retry. Applied and Delivered remain distinct even if one managed
   transaction collapses them.
4. Implement EventRef reserve/bind/fulfill/consume/abandon/expire/cancel with
   Company, generation, owner, correlation, revision, exact WaitBinding, and
   fulfillment application identity.
5. Implement activation claim as atomic `Activation Ready + Instance Ready ->
   Activation Leased(epoch) + Instance Executing(owner)`, plus heartbeat,
   complete/reclaim, expected Program/EventRef revisions, eligibility time,
   bounded attempts, and tenant/ordering lanes.
6. Implement managed effect preparation consequence, durable
   `Dispatchable -> ClaimedBeforeSend -> DispatchStarted`, Adapter outcome,
   reconciliation scheduling, and exact next-activation publication. Never
   redrive an ambiguous send through the activation queue.
7. Derive notification/outbox messages after durable state. Missing
   notifications are reconciled; notifications never become truth.
8. Implement timer/schedule occurrences in the same durable authority with
   canonical UTC/leap-second, clock authority, fixed-rate/fixed-delay anchor,
   timezone-data digest, DST, catch-up, lateness, and generation semantics.
9. Implement strict-lane DLQ policy, replay/redrive distinctions, source
   frontiers, count-and-byte ceilings by global/Company/source/stage scope,
   isolated control/gap/live/backfill/delivery/activation/effect capacity, and
   protocol-specific Host credit, broker ack withholding, HTTP 413/429/503,
   pausable-source, and declared-lossy gap behavior.

Verification:

- transaction failpoints at every write boundary;
- activation/Instance atomic claim, reclaim, split-brain, stale-fence, lost
  commit response, and checkpoint-ends-old-activation tests;
- pre-fulfillment/bind races and duplicate/conflicting fulfillment vectors;
- source acknowledgement only after its exact durable terminal disposition;
- identical source classification across frequency sweeps, independent count/
  byte exhaustion, and reservation failure at every acceptance component;
- no occurrence/delivery on refusal or permanent rejection, and no silent drop
  or coalescing under reducer, ingress, delivery, or activation overload;
- gap commit before frontier advance, pre-admission refusal during store
  unavailability, tenant/source isolation, live-vs-backfill fairness, HTTP
  413/429/503, broker ack withholding, and Host credit exhaustion;
- accepted occurrences survive local executor saturation and process loss;
- crash recovery from every occurrence, delivery, activation, effect, schedule,
  and Host transition, including `DispatchStarted`;
- unknown external-effect and unknown commit reconciliation without blind retry;
- multi-tenant fairness, queue depth, lease latency, and saturation benchmarks.

### Phase 5 — connect ActivationRunner without a fallback route

1. Server adapts one exact managed lease to the storage-neutral Agents
   `ActivationClaim`; Agents never imports a Server lease/storage type.
2. Agents validates the full artifact/admission/profile/claim/commit fence,
   executes locally, and reports one closed result with commit uncertainty
   distinct from effect uncertainty.
3. Server settles activation state only from matching stable activation id,
   claim epoch, expected Program revision, Instance execution owner, consumed
   EventRef revision where applicable, and commit proof.
4. Root API, webhook/Integration, broker, Widget/user-message, awaited event,
   schedule, Host/device, and future exact source descriptors all converge on
   occurrence application and this route. Child/effect completion uses its
   separate typed consequence but publishes activations through the same
   durable seam.
5. Delete direct request execution, volatile task queueing, and separate event-
   resume execution in the same target branch before enabling production.

Verification:

- end-to-end root and awaited activation tests for webhook, API/user message,
  broker, schedule, Host/device, pre-fulfill, and active-Instance-busy cases;
- tests prove a user message can start a Program or resume an authored wait,
  and can finish only when the resumed authored control flow commits return;
- pre-fulfillment tests prove an exact published EventRef may fulfill before the
  wait is bound without interrupting the running Instance or creating a
  concurrent activation; a generic message remains fail-busy;
- frequency sweeps prove classification is unchanged while capacity produces
  exact accept/backpressure/gap/reject behavior with no accepted-occurrence
  loss or implicit coalescing;
- process kill before/after lease, first node, effect preparation/dispatch,
  Execution Commit, runner reply, and Server settlement;
- absence scan proves no canonical direct-execute or old resume handler remains;
- load test proves durable queue backpressure and local worker saturation
  interact without unbounded memory or busy polling.
- managed Server and genuinely standalone durability adapters pass the same
  Agents-owned histories, failpoints, and semantic output/evidence comparisons.
- API tests keep transport receipt, durable acceptance/duplicate resolution,
  target application, and Program terminal result as four distinct stages.

### Phase 6 — migrate Integrations, Host, Auth, Adapters, Studio, and CLI

1. Server directly consumes Auth verification and Adapter occurrence
   contributions into its generic occurrence-candidate command; Auth still does
   not own occurrence deduplication.
2. Server terminates Host Link using Host SDK generated types and durable
   request/result correlation; remove `OsGatewayClient` and no-op gateway
   substitution.
3. Adapters replace OS-named bindings/comments and deepen provider effect
   reconciliation where current implementations are memory-only.
4. Studio and CLI switch to generated Server event/delivery/schedule/evidence
   APIs and render occurrence, delivery, admission, execution, and effect
   outcomes separately. Plugin rewrites or deletes every packaged Skill,
   reference, document and test that teaches the retired trigger-sidecar/event-
   loop owner, regenerates/signs the target catalogue, and proves installed
   content contains only target APIs.
5. Physical/streaming Host paths publish explicit sample/gap/watermark
   candidates, declared window/watermark/late/reorder/hysteresis policy, stable
   derived occurrence keys, mandatory clock/boot/frame/calibration provenance,
   and separate live-command/metadata/bulk-evidence credits.
   Sources that declare every item to be a Program occurrence bypass reduction
   without bypassing bounded admission or durable acceptance.
6. Auth makes the old OS audience/principal/credentials enforceably invalid at
   the signed pre-activation cutover barrier. `J4` has already removed every
   source/configuration reference; only physical deletion of the revoked
   credential records follows target activation.

Verification:

- owner-local generated-client and conformance suites;
- Host resume/backpressure/takeover/late-result/revocation tests;
- provider duplicate/ack/checkpoint tests;
- physical clock uncertainty/freshness, stream gap/overflow, live-vs-backfill,
  stale command, actuation outcome/stop, and controller-isolation tests;
- Studio/CLI route and vocabulary absence scans;
- no no-op, first-available, or environment-selected event gateway.

### Phase 7 — rehearse the one-time state transform

1. Pin the source Compatibility Set and inventory every deployed retiring-
   source/Server durable store, dynamic source binding, physical resource and
   backup, not merely local checkouts.
2. Classify each old row as transform, archive-only, or invalid.
3. Exercise a signed, enforceable barrier that stops and revokes every old
   ingress, source lease, response writer, scheduler, delivery/redrive worker,
   Host attachment, effect dispatcher, credential, and store writer; drain or
   explicitly classify in-flight unknown work.
4. Run one offline deterministic transformer whose signed execution manifest
   pins its source/binary digest, source/target schemas, identity map, restart
   journal namespace, partition manifests, barrier policy, rollback package and
   operator procedure, with schema/digest validation, counts and Merkle roots.
5. Reconcile source and target counts/frontiers/DLQ/Host sequences. Ambiguous
   EventRef, target, authority, or effect state aborts; it is never guessed.
6. Prove target negative reachability before transferring authority.

The transformer is the only old reader. It is not linked, packaged, installed,
or callable by the canonical product.

Verification:

- empty-state and populated-state rehearsals;
- crash/restart at every transformer journal boundary;
- deterministic identical output roots on repeat;
- signed emptiness manifest when applicable;
- rollback rehearsal before the point of no return.

### Phase 8 — atomic cutover and physical retirement

Precondition: the exact `J4` candidate has already deleted every replaced
source contract, route, reader, queue, generated consumer, active repository
membership, configuration key, topology/Compose member, package/image input,
skill, current runbook, and product-vocabulary reference. The target candidate
contains no old path; none of these deletions is deferred until activation.

1. Apply the signed barrier: stop and enforceably revoke/fence every old
   ingress, source lease, response writer, scheduler, delivery/redrive worker,
   Host attachment, effect dispatcher, credential, and store writer. Prove a
   stale OS process cannot act.
2. Snapshot, transform, and verify state while neither generation has external
   authority; verify target integrity and negative reachability.
3. Activate the complete target Compatibility Set once.
4. Treat the earliest target-only irreversible write, provider acknowledgement,
   Host frame, or external effect as the point of no return.
5. After that point recover forward only; never restart a mixed generation.
6. After target health is established, independently remove only the already-
   fenced physical remnants: stopped service objects, old credential records,
   tables, streams, volumes, archives, deployment artifacts, and related cloud
   infrastructure. This cleanup cannot remove source, configuration, routes,
   generated consumers, repository membership, or topology because `J4`
   already removed them. Execute only the named owner `PC-*` tasks and close
   `J6`; an unclassified surviving resource blocks final completion.

Verification:

- complete owner-local gates, then coordinator `dekk apxm verify`;
- production-topology config/smoke with no OS member;
- retired-token scan across source, generated files, deployment, active ADRs
  and indexes, docs, and runbooks;
- signed proof no OS process/credential/database/stream could write before
  target activation and none exists after deletion;
- final `J6` Physical Absence Manifest covering every environment and every
  owner cleanup coordinate;
- event-to-activation-to-commit smoke, event wait/resume smoke, retry/DLQ/redrive
  smoke, Host reconnect smoke, and physical-edge provenance smoke.

## Landing and integration order

This is merge order at shared convergence points, not execution order. The DAG
and earliest-start waves above govern parallel work:

1. land the accepted authority packet and present-tense documentation;
2. land owner contracts in their semantic dependency order, then perform the
   one generation cohort and freeze `J1`;
3. land disjoint owner-local leaf modules and tests in parallel after `J1`;
4. let the named Agents, Server, Auth, Host SDK, Adapters, Studio, Plugin,
   Contracts, vLLM and coordinator integrators make each repository's
   shared-root edits once;
5. join exact owner revisions at `J2` and `J3`, including cross-plane recovery,
   security, correctness, and performance evidence;
6. land all replacement-path removals, generated consumers, target-only
   topology, repository-map changes, artifacts, and absence tests as the
   unreachable `J4` candidate;
7. attach transformer and supported-environment rehearsal evidence to that
   exact candidate at `J5`; the transformer remains excluded from product and
   release artifacts; and
8. freeze and transfer authority only through serial release actions `R1`–`R4`.

No product implementation merge occurs before `J1`, and no commit introduces a
production fallback. New modules and the complete `J4` candidate remain
unreachable from production composition. `R4`, not a source commit or gradual
service merge, makes the already-complete target reachable exactly once.

## Rollback and recovery

Before the point of no return, rollback restores the entire pinned old
Compatibility Set and verified snapshot, issues fresh old-release-compatible
credentials, and advances source, Host, and authority epochs strictly beyond
the revoked values. Revoked credentials are never revived. This is an operator
release recovery procedure, not a packaged product fallback. Partial owner
rollback is forbidden.

After the point of no return, rollback to OS is forbidden. Recovery fixes or
rolls forward the complete target set, preserving new occurrence, activation,
effect, and Host facts. A mixed old/new deployment is never a recovery mode.

Development branches and document changes are ordinarily reversible. Accepted
contract ids, transformed durable identities, revoked credentials, and
target-only external actions require the coordinated cutover rules above.

## Definition of done

The work is complete only when:

- the new ownership ADRs are accepted and all canonical docs agree;
- current ADR files and indexes describe only the target owners; superseded
  product vocabulary exists only in Git history and migration evidence;
- Agents ships the readiness kernel, deterministic reference, local
  work-stealing executor, ActivationRunner, model tests, and performance gates;
- Server owns all durable occurrences, deliveries/target applications,
  activations/leases, managed effect work/reconciliation, schedules, retries,
  DLQ/redrive, and Host gateway state;
- every producer/consumer uses generated new-owner contracts and every signed
  Plugin package teaches only the target owner/API model;
- every declared repository and every contract, producer, durable row family,
  consumer, credential/configuration, deployment/operational surface,
  document, and physical remnant has one closed master disposition and
  consumed evidence; none is out of scope, deferred, optional cleanup, or an
  unowned follow-up;
- the state transform and cutover are verified in every environment;
- `V-RC` and `V-LA` prove one unreachable candidate, global `J-V1` proves all
  three master portfolios use it, and `J6` proves final physical closure;
- OS has no active repository, process, credential, route, table, stream,
  generated consumer, deployment member, or product skill;
- the sequential canonical driver, capability scheduler, volatile Server task
  queue, separate event-resume route, and no-op/fallback gateways are gone; and
- negative-reachability, correctness, crash, replay, performance, NUMA,
  backpressure, and physical-edge boundary tests all pass.
