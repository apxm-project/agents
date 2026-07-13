import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { GraphBuilder } from "../src/builder.js";
import { ApxmGraph, emitMultiFlowModule } from "../src/graph.js";

// Shared fixtures under the `agents`-owned Rust CLI crate: the same vectors
// the Rust `frontend_air` unit tests and Python's `test_air_parity.py`
// diff against, so all three language surfaces prove parity against one set
// of graphs.
const __dirname = dirname(fileURLToPath(import.meta.url));
const FIXTURES_DIR = join(__dirname, "../../../../tools/cli/tests/fixtures/frontend_graph_parity");

function readFixture(name: string): unknown {
  return JSON.parse(readFileSync(join(FIXTURES_DIR, name), "utf8")) as unknown;
}

function readGolden(name: string): string {
  return readFileSync(join(FIXTURES_DIR, name), "utf8");
}

function expectNativeDto(name: string, graph: ApxmGraph): void {
  expect(graph.toDict()).toEqual(readFixture(name));
}

function askFlow(): ApxmGraph {
  const graph = new GraphBuilder("ask_flow", { metadata: { is_entry: true } });
  graph.param("name", "str");
  const answer = graph.ask({ name: "ask", prompt: "Say hi to {name}" });
  graph.done(answer, "out");
  return graph.toGraph();
}

function parametrizedFlow(): ApxmGraph {
  const graph = new GraphBuilder("parametrized_flow", { metadata: { is_entry: true } });
  graph.param("topic", "str");
  graph.param("style", "str");
  const answer = graph.ask({
    name: "ask",
    prompt: "Research {topic} in the style of {style}",
    token_budget: 256,
  });
  graph.done(answer, "out");
  return graph.toGraph();
}

function profiledAgentFlow(): ApxmGraph {
  const graph = new GraphBuilder("agent_flow", { metadata: { is_entry: true } });
  const agent = graph.op("AGENT", {
    name: "coder",
    attributes: { profile: "codex", prompt: "Fix it", cwd: "/tmp/work" },
  });
  graph.done(agent, "out");
  return graph.toGraph();
}

function multiFlowConversational(): ApxmGraph[] {
  const main = new GraphBuilder("main", { metadata: { is_entry: true } });
  const runTurn = main.op("FLOW_CALL", {
    name: "run_turn",
    attributes: { agent_name: "conversation", flow_name: "turn" },
  });
  main.done(runTurn, "return_turn");

  const turn = new GraphBuilder("conversation.turn", { metadata: { is_entry: false } });
  turn.param("user_message", "str");
  const answer = turn.ask({ name: "ask", prompt: "Reply to: {user_message}" });
  turn.done(answer, "out");
  return [main.toGraph(), turn.toGraph()];
}

function reasoningFlow(): ApxmGraph {
  const graph = new GraphBuilder("reasoning_flow", { metadata: { is_entry: true } });
  graph.plan({ name: "plan", attributes: { goal: "Create a complete implementation plan" } });
  graph.reflect({ name: "reflect", attributes: { trace_query: "most_recent_execution" } });
  const verification = graph.verify({
    name: "verify",
    attributes: {
      claim: "The implementation plan is complete.",
      evidence: "The review lists every required frontend contract.",
    },
  });
  graph.done(verification, "out");
  return graph.toGraph();
}

function controlFlow(): ApxmGraph {
  const graph = new GraphBuilder("control_flow", { metadata: { is_entry: true } });
  const classified = graph.ask({ name: "classify", prompt: "Classify the request" });
  const branch = graph.branchOnValue({
    name: "branch",
    attributes: {
      value: "approved",
      true_label: "approved_path",
      false_label: "review_path",
    },
  });
  graph.addEdge(classified, branch);
  const routed = graph.switchNode({
    name: "route",
    attributes: { discriminant: "classification", case_labels: ["approved", "review"] },
  });
  graph.addEdge(classified, routed);
  graph.tryCatch({ name: "recover", attributes: { try_label: "route", catch_label: "fallback" } });
  graph.done(routed, "out");
  return graph.toGraph();
}

function synchronizationFlow(): ApxmGraph {
  const graph = new GraphBuilder("synchronization_flow", { metadata: { is_entry: true } });
  const answer = graph.ask({ name: "answer", prompt: "Prepare the durable result" });
  const checkpoint = graph.checkpoint({ name: "checkpoint", checkpointId: "after_answer", inputs: { answer } });
  const fence = graph.fence({ name: "fence", attributes: { ordering: "serial" } });
  const merged = graph.merge("merged", checkpoint, fence);
  graph.done(merged, "out");
  return graph.toGraph();
}

function coordinationFlow(): ApxmGraph {
  const graph = new GraphBuilder("coordination_flow", { metadata: { is_entry: true } });
  const worker = graph.spawnAgent({ name: "worker", agentName: "worker", mode: "collaborative" });
  const delegated = graph.delegate({
    name: "delegate",
    taskSpec: "Review the frontend contract",
    targetAgent: "worker",
  });
  graph.addEdge(worker, delegated);
  const transferred = graph.handoff({
    name: "handoff",
    attributes: {
      handoff_from: "orchestrator",
      handoff_to: "worker",
      payload: "Begin the assigned review.",
      transfer_state: true,
    },
  });
  graph.addEdge(delegated, transferred);
  graph.done(transferred, "out");
  return graph.toGraph();
}

describe("native authoring DTO parity", () => {
  it("matches every shared authoring vector before AIR emission", () => {
    expectNativeDto("ask_flow.json", askFlow());
    expectNativeDto("parametrized_flow.json", parametrizedFlow());
    expectNativeDto("profiled_agent_flow.json", profiledAgentFlow());
    expect(multiFlowConversational().map((graph) => graph.toDict())).toEqual(
      readFixture("multi_flow_conversational.json"),
    );
    expectNativeDto("reasoning_flow.json", reasoningFlow());
    expectNativeDto("control_flow.json", controlFlow());
    expectNativeDto("synchronization_flow.json", synchronizationFlow());
    expectNativeDto("coordination_flow.json", coordinationFlow());
  });
});

describe("AIR emission through the canonical Rust printer", () => {
  it.each([
    ["ask flow", "ask_flow.golden.air", () => askFlow().toAir()],
    ["parameterized flow", "parametrized_flow.golden.air", () => parametrizedFlow().toAir()],
    ["profiled-agent flow", "profiled_agent_flow.golden.air", () => profiledAgentFlow().toAir()],
    [
      "multi-flow conversational shape",
      "multi_flow_conversational.golden.air",
      () => emitMultiFlowModule(multiFlowConversational()),
    ],
    ["reasoning flow", "reasoning_flow.golden.air", () => reasoningFlow().toAir()],
    ["control flow", "control_flow.golden.air", () => controlFlow().toAir()],
    ["synchronization flow", "synchronization_flow.golden.air", () => synchronizationFlow().toAir()],
    ["coordination flow", "coordination_flow.golden.air", () => coordinationFlow().toAir()],
  ])("matches the shared golden AIR for %s", (_name, goldenName, emit) => {
    expect(emit()).toBe(readGolden(goldenName));
  });
});
