/**
 * TSF-4: Python frontend parity lock.
 *
 * `workspace/contracts/vectors/air/*.air` (shared with the Rust and Python
 * frontends — see `crates/compiler/pipeline/tests/air_vectors.rs` and
 * `crates/compiler/frontend/python/apxm/ir.py`) are the cross-frontend AIR
 * reference vectors. All six fixtures were generated (or, for
 * `approval_gate`/`converse_agent`/`runtime_agent_routing`, regenerated) by
 * Python's `GraphRecorder.to_graph().to_air()` reference emitter and are
 * reproducible byte-for-byte from an equivalent TypeScript `GraphBuilder`
 * graph — this test locks that parity so it can't silently regress.
 *
 * `approval_gate`, `converse_agent`, and `runtime_agent_routing` were
 * regenerated from the original WF-2 shapes they were named for (a
 * PAUSE/RESUME approval barrier, a conversational AUTONOMOUS loop, and a
 * runtime-routed SPAWN_AGENT) using the current op-spec-driven emitter on
 * both frontends — see each fixture's file header for exactly what changed
 * and why (the pre-CM-7/op-spec syntax they used is no longer producible by
 * either frontend).
 */
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { GraphBuilder } from "../src/builder.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// crates/compiler/frontend/typescript/test -> workspace/contracts/vectors/air
// (workspace/agents and workspace/contracts are sibling repos under workspace/).
const CONTRACTS_AIR_DIR = path.resolve(
  __dirname,
  "../../../../../../contracts/vectors/air",
);

function readFixture(name: string): string {
  return readFileSync(path.join(CONTRACTS_AIR_DIR, `${name}.air`), "utf-8");
}

/** Strip the leading `//`-comment documentation header, keeping from `module {` on. */
function stripHeaderComment(airText: string): string {
  const idx = airText.indexOf("module {");
  return idx === -1 ? airText : airText.slice(idx);
}

describe("TSF-4: TS/Python AIR emission parity", () => {
  it("multi_agent_delegate: spawn_agent + delegate + communicate byte-matches the Python reference", () => {
    const g = new GraphBuilder("multi_agent_delegate");
    g.spawnAgent({ name: "spawn_worker", agentName: "worker", agentRoute: "auto" });
    const task = g.delegate({
      name: "delegate_task",
      taskSpec: "Summarize the quarterly report.",
      targetAgent: "worker",
    });
    const reply = g.communicate({
      name: "collect_result",
      targetAgent: "worker",
      message: "Findings so far: {task}. Send back your final summary.",
      inputs: { task },
    });
    g.done(reply);

    const tsAir = g.toAir();
    const pyAir = stripHeaderComment(readFixture("multi_agent_delegate")).replace(/\n$/, "");
    expect(tsAir).toBe(pyAir);
  });

  it("capability_pipeline: register_capability + chained invoke_capability byte-matches the Python reference", () => {
    const g = new GraphBuilder("capability_pipeline");
    const registered = g.registerCapability({
      name: "register_lookup",
      capabilityName: "lookup_docs",
      description: "Look up internal documentation by query.",
      parametersSchema: '{"type": "object", "properties": {"query": {"type": "string"}}}',
    });
    const lookup = g.invokeCapability({
      name: "invoke_lookup",
      capability: "lookup_docs",
      params: '{"query": "release checklist"}',
    });
    g.addEdge(registered, lookup, "Control");
    const digest = g.invokeCapability({
      name: "invoke_digest",
      capability: "summarize_text",
      params: '{"text": "{lookup}"}',
      inputs: { lookup },
    });
    g.done(digest);

    const tsAir = g.toAir();
    const pyAir = stripHeaderComment(readFixture("capability_pipeline")).replace(/\n$/, "");
    expect(tsAir).toBe(pyAir);
  });

  it("checkpoint_pipeline: ask -> checkpoint -> ask byte-matches the Python reference", () => {
    const g = new GraphBuilder("checkpoint_pipeline");
    const draft = g.ask({ name: "draft_notes", prompt: "Draft release notes for version {version}." });
    const gate = g.checkpoint("await_signoff");
    g.addEdge(draft, gate, "Control");
    const final = g.ask({
      name: "polish_notes",
      prompt: "Polish this release-notes draft:\n\n{draft}",
      inputs: { draft },
    });
    g.addEdge(gate, final, "Control");
    g.done(final);

    const tsAir = g.toAir();
    const pyAir = stripHeaderComment(readFixture("checkpoint_pipeline")).replace(/\n$/, "");
    expect(tsAir).toBe(pyAir);
  });

  it("approval_gate: ask -> pause -> resume byte-matches the Python reference", () => {
    const g = new GraphBuilder("approval_gate");
    const draft = g.ask({ name: "draft_change", prompt: "Propose a change to review." });
    const gate = g.pause({
      name: "await_approval",
      message: "Awaiting human approval.",
      checkpointId: "approval_gate_cp",
      inputs: { draft_change: draft },
    });
    const resumed = g.resume({
      name: "resumed",
      checkpoint: "approval_gate_cp",
      inputs: { await_approval: gate },
    });
    g.done(resumed);

    const tsAir = g.toAir();
    const pyAir = stripHeaderComment(readFixture("approval_gate")).replace(/\n$/, "");
    expect(tsAir).toBe(pyAir);
  });

  it("converse_agent: ask -> conversational autonomous byte-matches the Python reference", () => {
    const g = new GraphBuilder("converse_agent");
    g.param("turns", "str");
    const history = g.ask({ name: "load_history", prompt: "Summarize recent context for {turns}." });
    const reply = g.autonomous({
      name: "reply",
      prompt:
        "You are APXM Assistant, a precise conversational agent. Use the transcript for context and keep replies concise.",
      maxIterations: 50,
      converse: "true",
      tool_groups: ["web", "skills"],
      inputs: { load_history: history },
    });
    g.done(reply);

    const tsAir = g.toAir();
    const pyAir = stripHeaderComment(readFixture("converse_agent")).replace(/\n$/, "");
    expect(tsAir).toBe(pyAir);
  });

  it("runtime_agent_routing: spawn_agent with routing attrs byte-matches the Python reference", () => {
    const g = new GraphBuilder("runtime_agent_routing");
    const worker = g.spawnAgent({
      name: "routed_worker",
      agentName: "routed_worker",
      agentRoute: "auto",
      requiredCapabilities: ["execute"],
      preferredProfiles: ["codex"],
    });
    g.done(worker);

    const tsAir = g.toAir();
    const pyAir = stripHeaderComment(readFixture("runtime_agent_routing")).replace(/\n$/, "");
    expect(tsAir).toBe(pyAir);
  });
});
