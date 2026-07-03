import { describe, expect, it } from "vitest";
import { GraphBuilder } from "../src/builder.js";

describe("GraphBuilder", () => {
  it("records spawn + delegate + communicate as a wired graph", () => {
    const g = new GraphBuilder("spawn_delegate_communicate");
    const worker = g.spawnAgent({ agentName: "worker", mode: "explore" });
    const task = g.delegate({ taskSpec: "research the topic", targetAgent: "worker" });
    g.addEdge(worker, task);
    const reply = g.communicate({ targetAgent: "worker", message: "status?" });
    g.addEdge(task, reply);
    g.done(reply);

    const graph = g.toGraph();
    expect(graph.nodes.map((n) => n.op)).toEqual(["SPAWN_AGENT", "DELEGATE", "COMMUNICATE", "RETURN"]);

    const spawnNode = graph.nodes[0];
    expect(spawnNode.attributes.agent_name).toBe("worker");
    expect(spawnNode.attributes.mode).toBe("explore");

    const delegateNode = graph.nodes[1];
    expect(delegateNode.attributes.task_spec).toBe("research the topic");
    expect(delegateNode.attributes.target_agent).toBe("worker");

    const communicateNode = graph.nodes[2];
    expect(communicateNode.attributes.recipient).toBe("worker");
    expect(communicateNode.attributes.message).toBe("status?");

    // spawn -> delegate, delegate -> communicate (explicit), and an implicit
    // spawn-session -> communicate Data edge (mirrors GraphRecorder session
    // tracking so COMMUNICATE always depends on its target's SPAWN_AGENT).
    const edgePairs = graph.edges.map((e) => [e.from, e.to, e.dependency]);
    expect(edgePairs).toContainEqual([spawnNode.id, delegateNode.id, "Data"]);
    expect(edgePairs).toContainEqual([delegateNode.id, communicateNode.id, "Data"]);
    expect(edgePairs).toContainEqual([spawnNode.id, communicateNode.id, "Data"]);
  });

  it("wires ask() inputs into input_names and Data edges", () => {
    const g = new GraphBuilder("ask_flow");
    const research = g.ask({ name: "research", prompt: "Research: {topic}" });
    const critique = g.ask({ prompt: "Critique: {research}", inputs: { research } });
    g.done(critique);

    const graph = g.toGraph();
    const critiqueNode = graph.nodes.find((n) => n.name === "ask")!;
    expect(critiqueNode.attributes.input_names).toEqual(["research"]);
    expect(graph.edges).toContainEqual({
      from: research.nodeId,
      to: critiqueNode.id,
      dependency: "Data",
    });
  });

  it("records a capability invocation (INV_TOOL)", () => {
    const g = new GraphBuilder("capability_flow");
    const invoked = g.invokeCapability({
      capability: "search:web",
      params: { query: "apxm" },
    });
    g.done(invoked);

    const graph = g.toGraph();
    const node = graph.nodes.find((n) => n.op === "INV_TOOL")!;
    expect(node.attributes.capability).toBe("search:web");
    expect(node.attributes.params_json).toBe(JSON.stringify({ query: "apxm" }));
  });

  it("records a checkpoint as a FENCE node with checkpoint=true", () => {
    const g = new GraphBuilder("checkpoint_flow");
    const cp = g.checkpoint();
    g.done(cp);

    const graph = g.toGraph();
    const node = graph.nodes.find((n) => n.name === "fence")!;
    expect(node.op).toBe("FENCE");
    expect(node.attributes.checkpoint).toBe(true);
  });

  it("rejects duplicate node names", () => {
    const g = new GraphBuilder("dup_flow");
    g.nop("same");
    expect(() => g.nop("same")).toThrowError(/already exists/);
  });

  it("rejects duplicate parameter names", () => {
    const g = new GraphBuilder("param_flow");
    g.param("topic");
    expect(() => g.param("topic")).toThrowError(/already exists/);
  });
});
