import { describe, expect, it } from "vitest";
import { GraphBuilder } from "../src/builder.js";
import { GENERATED_GRAPH_BUILDER_OP_METHODS } from "../src/generated/builder-ops.js";
import { ALL_OPERATIONS } from "../src/generated/ops.js";

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
    // spawn-session -> communicate Data edge so COMMUNICATE always depends on
    // its target's SPAWN_AGENT.
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

  it("records a capability invocation (INV_CAP)", () => {
    const g = new GraphBuilder("capability_flow");
    const invoked = g.invokeCapability({
      capability: "search:web",
      params: { query: "apxm" },
    });
    g.done(invoked);

    const graph = g.toGraph();
    const node = graph.nodes.find((n) => n.op === "INV_CAP")!;
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

  it("records generic catalog ops with attributes and input edges", () => {
    const g = new GraphBuilder("generic_ops");
    const source = g.op("CONST_STR", { name: "topic", attributes: { value: "topic" } });
    const branch = g.op("BRANCH_ON_VALUE", {
      name: "branch",
      attributes: { value: "yes", true_label: "ok", false_label: "no" },
      inputs: { topic: source },
    });

    const graph = g.toGraph();
    const node = graph.nodes.find((n) => n.id === branch.nodeId)!;
    expect(node.op).toBe("BRANCH_ON_VALUE");
    expect(node.attributes).toEqual({
      value: "yes",
      true_label: "ok",
      false_label: "no",
      input_names: ["topic"],
    });
    expect(graph.edges).toContainEqual({
      from: source.nodeId,
      to: branch.nodeId,
      dependency: "Data",
    });
  });

  it("exposes a named builder surface for every catalog op", () => {
    const prototype = GraphBuilder.prototype as unknown as Record<string, unknown>;
    for (const spec of ALL_OPERATIONS) {
      const method = GENERATED_GRAPH_BUILDER_OP_METHODS[spec.op];
      expect(method, spec.op).toBeDefined();
      expect(typeof prototype[method], `${spec.op}.${method}`).toBe("function");
    }
  });

  it("serializes the compiler FrontendGraph DTO shape", () => {
    const g = new GraphBuilder("dto_flow", { is_entry: true });
    g.param("topic", "str");
    const answer = g.ask({ name: "answer", prompt: "Research: {topic}" });
    const invoked = g.invokeCapability({
      name: "lookup",
      capability: "search:web",
      params: { query: "apxm" },
      inputs: { answer },
    });
    g.done(invoked);

    expect(g.toGraph().toDict()).toEqual({
      name: "dto_flow",
      nodes: [
        {
          id: answer.nodeId,
          name: "answer",
          op: "ASK",
          attributes: { template_str: "Research: {topic}" },
        },
        {
          id: invoked.nodeId,
          name: "lookup",
          op: "INV_CAP",
          attributes: {
            capability: "search:web",
            params_json: JSON.stringify({ query: "apxm" }),
            input_names: ["answer"],
          },
        },
        {
          id: invoked.nodeId + 1,
          name: "return",
          op: "RETURN",
          attributes: {},
        },
      ],
      edges: [
        { from: answer.nodeId, to: invoked.nodeId, dependency: "Data" },
        { from: invoked.nodeId, to: invoked.nodeId + 1, dependency: "Data" },
      ],
      parameters: [{ name: "topic", type_name: "str" }],
      metadata: { is_entry: true },
    });
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
