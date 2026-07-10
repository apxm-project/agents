import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { GraphBuilder } from "../src/builder.js";
import { ApxmGraph, emitMultiFlowModule, type ApxmGraphData, type GraphEdge, type Parameter } from "../src/graph.js";

// Shared fixtures under the `agents`-owned Rust CLI crate: the same vectors
// the Rust `frontend_air` unit tests and Python's `test_air_parity.py`
// diff against, so all three language surfaces prove parity against one set
// of graphs (docs/plans/tasks/W3.1.md).
const __dirname = dirname(fileURLToPath(import.meta.url));
const FIXTURES_DIR = join(__dirname, "../../../../tools/cli/tests/fixtures/frontend_graph_parity");

interface WireParameter {
  name: string;
  type_name: string;
}

interface WireGraph {
  name: string;
  nodes: ApxmGraphData["nodes"];
  edges: GraphEdge[];
  parameters: WireParameter[];
  metadata: Record<string, unknown>;
}

function readFixture(name: string): WireGraph {
  return JSON.parse(readFileSync(join(FIXTURES_DIR, name), "utf8")) as WireGraph;
}

function readGolden(name: string): string {
  return readFileSync(join(FIXTURES_DIR, name), "utf8");
}

/** Build a live `ApxmGraph` from a shared wire-shape fixture (`type_name` ->
 * the TS `Parameter` interface's `typeName`), the same conversion
 * `ApxmGraph.toDict()` does in reverse. */
function graphFromFixture(wire: WireGraph): ApxmGraph {
  const parameters: Parameter[] = wire.parameters.map((p) => ({ name: p.name, typeName: p.type_name }));
  return new ApxmGraph({
    name: wire.name,
    nodes: wire.nodes,
    edges: wire.edges,
    parameters,
    metadata: wire.metadata,
  });
}

// AIR text is produced by the single Rust printer (`apxm emit-air`), not by this
// package — the TypeScript frontend only builds the frontend-graph DTO (covered
// hermetically in builder.test.ts) and hands it to that printer. These tests
// exercise the actual TS -> emit-air -> printer wiring end to end, so they need
// a resolvable `apxm` binary. They run when APXM_BIN points at one and skip
// otherwise (e.g. the node-only npm-publish CI job), where AIR-text correctness
// is already covered by the Rust `frontend_air`/`frontend_graph` tests. Set
// APXM_BIN to the built binary to run them.
describe.skipIf(!process.env.APXM_BIN)("AIR emission (integration)", () => {
  it("emits AIR for a simple ask flow", () => {
    const g = new GraphBuilder("simple_ask");
    const answer = g.ask({ prompt: "Say hello" });
    g.done(answer);

    const air = g.toAir();
    expect(air).toContain("module {");
    expect(air).toContain("func.func @simple_ask");
    expect(air).toContain("attributes {ais.entry}");
    expect(air).toContain('ais.ask "Say hello"');
    expect(air).toContain("ais.return");
  });

  it("emits AIR with declared parameters", () => {
    const g = new GraphBuilder("param_air");
    g.param("topic", "str");
    const answer = g.ask({ prompt: "Research: {topic}" });
    g.done(answer);

    const air = g.toAir();
    expect(air).toContain('ais.param_name = "topic"');
    expect(air).toContain('ais.ask "Research: {topic}"');
  });

  it("matches golden AIR for ask flow", () => {
    const graph = graphFromFixture(readFixture("ask_flow.json"));
    const air = graph.toAir();
    const golden = readGolden("ask_flow.golden.air");
    expect(air).toBe(golden);
  });

  it("matches golden AIR for multi-flow conversational shape", () => {
    // Cross-plane: TS's `emitMultiFlowModule` must match the same
    // `AirProgram::to_air()` golden output the Rust `frontend_air` tests
    // and Python's `emit_multi_flow_module` also match (multi-flow fixture
    // cross-plane vector, docs/plans/tasks/W3.1.md).
    const wireGraphs = readFixture("multi_flow_conversational.json") as unknown as WireGraph[];
    const graphs = wireGraphs.map(graphFromFixture);
    const air = emitMultiFlowModule(graphs);
    const golden = readGolden("multi_flow_conversational.golden.air");
    expect(air).toBe(golden);
  });
});
