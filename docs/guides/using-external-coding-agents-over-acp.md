# Use external coding agents through ACP

- Status: canonical target guide
- Normative contract: [ACP interoperability and selection](../agents/acp-and-routing-contract.md)

> **Target-state guide.** The current prototype still exposes
> `SPAWN_AGENT`/`COMMUNICATE`, injects an AAM preamble, accepts mutable profiles,
> and maps ACP usage into native model accounting. Those paths are migration
> evidence, not canonical API. The full-replacement plan deletes them; there is
> no compatibility wrapper or dual path.

## 1. What ACP means in APXM

APXM acts as an outbound ACP Client. Claude Code, Codex, or another
conformant agent runs outside APXM and is invoked through an exact admitted
External Agent Profile. It is not an Agent Program, model backend, Integration,
webhook, Runtime Profile or special AIR operation.

The exact `claude-agent-acp` adapter may use the Claude Agent SDK internally,
and `codex-acp` may use Codex App Server. Those are adapter implementation
details. APXM core has no direct dependency on either vendor SDK.

## 2. Administrator setup

Before source can use a profile, an administrator:

1. admits the exact adapter/profile in the Compatibility Set;
2. creates its Auth connection or authentication method reference;
3. selects allowed workspace and writable roots;
4. sets executable, environment, egress, process and resource ceilings;
5. grants the external-agent session Capabilities and allowed reverse
   filesystem/terminal/MCP operations;
6. runs the protocol/confinement connection test; and
7. publishes the exact profile reference to the Agent Program project.

Production setup never accepts `npx -y`, a floating package version, arbitrary
shell command, plaintext environment secret or unsandboxed profile.

## 3. Program source

```python
session = await external_agents.open(
    profile=ExactCodexProfile,
    workspace=WorkspaceRef("review-root"),
    context=CodeReviewContext(repository=repo, objective=objective),
)
try:
    result = await session.prompt(CodeReviewPrompt(diff=diff))
finally:
    await session.close()
```

```typescript
const session = await externalAgents.open({
  profile: ExactCodexProfile,
  workspace: new WorkspaceRef("review-root"),
  context: { repository: repo, objective },
});
try {
  const result = await session.prompt({ diff });
} finally {
  await session.close();
}
```

The TypeScript wrapper records the same four Capability operations.
There is no direct `AcpSession()` runtime class in either frontend. The opaque
session reference can be kept only by the same stateful Program Instance in
explicit Program Context. It is bound to the owner/principal/profile/grant
lease and carries no authority or process state.

## 4. Context and Skills

Pass only the typed context needed for the external prompt. APXM does not add
an AAM preamble, all parent context, Skill bodies, environment, credentials or
filesystem. If the agent needs Skills, source exposes Skill Discovery on demand
through an attenuated APXM MCP/Capability gateway and the current grant
authorizes each effect. MCP is not an ACP reverse request, and ACP `mcpServers`
connectivity alone grants nothing. A pinned profile references one admitted
gateway transport: local stdio or managed Streamable HTTP with its own Auth
audience and attenuation. Agent-native tools that remain inside the
peer stay inside the outer external-agent Capability under mandatory
root/executable/egress/sandbox ceilings.

## 5. Permissions and cancellation

Every ACP client-directed file, terminal, or permission request is evaluated as
a nested APXM Capability occurrence. MCP operations go through the separate
APXM gateway. The profile's roots and sandbox are ceilings; Auth may narrow
them further. ACP's interactive permission answer reports the APXM decision
and cannot widen it.

Cancellation requests the ACP prompt to stop, fences new reverse effects, and
waits for a terminal result. If process termination cannot prove the external
effect outcome, handle `ExternalAgentOutcomeUnknown`; do not open another
profile or repeat the prompt automatically.

## 6. Inspecting evidence

The outer APXM node is one Capability NodeExecution. Studio expands its nested
ACP timeline: initialization, negotiated features, messages, plans, diffs,
tools, terminal updates, permission decisions, cancellation and close. These
are attributed external events, not APXM Turns or child nodes.

ACP-reported model, context window, tokens, rate-limit state, or cost is
labelled peer-reported evidence with scope, accumulation, precision, origin,
and availability. It never enters the native model accountant. APXM Server's
`apxm.operational-usage-fact.v1` ledger remains the operational spend and
budget authority.

Metered company-owned provider/API-key execution may support a versioned
derived estimate or later provider reconciliation. Seat/subscription execution
may have no trustworthy per-run billed cost; Studio shows `cost unavailable`
instead of inventing one. If a local external MCP client invokes APXM, its own
upstream model usage and licence cost are `unavailable_not_observed`; APXM
accounts only for work performed inside APXM.

Server admits the outer Capability against a configured bounded maximum or an
explicit unknown-cost policy. A hard budget denies work when no trusted bound
can be established.

## 7. Compatibility claims

- Claude support means the exact admitted `claude-agent-acp` profile; its
  internal use of the Claude Agent SDK is not a core dependency or a claim of
  native Claude Code ACP.
- Codex support means the exact admitted ACP `codex-acp` profile over the Codex
  App Server, not an implicit native Codex CLI protocol promise.
- Another ACP agent is supported only for the exact feature/platform matrix
  that passed conformance; “ACP-compatible” text is insufficient.
