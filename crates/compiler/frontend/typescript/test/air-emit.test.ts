import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { GraphBuilder } from "../src/builder.js";

const CHECK_SCRIPT = path.resolve(
  new URL(".", import.meta.url).pathname,
  "../../../../../../../tools/check_air_provenance.py",
);

function runProvenanceCheck(airText: string): { ok: boolean; output: string } {
  const dir = mkdtempSync(path.join(tmpdir(), "apxm-frontend-air-"));
  const file = path.join(dir, "emitted.air");
  writeFileSync(file, airText, "utf-8");
  try {
    const output = execFileSync("python3", [CHECK_SCRIPT, file], { encoding: "utf-8" });
    return { ok: true, output };
  } catch (err: any) {
    return { ok: false, output: `${err.stdout ?? ""}${err.stderr ?? ""}` };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

describe("to_air() emission", () => {
  it("emits a module with an entry func for a simple ask flow", () => {
    const g = new GraphBuilder("simple_ask");
    const answer = g.ask({ prompt: "Say hello" });
    g.done(answer);

    const air = g.toAir();
    expect(air).toContain("module {");
    expect(air).toContain("func.func @simple_ask");
    expect(air).toContain("attributes {ais.entry}");
    expect(air).toContain("ais.ask");
    expect(air).toContain("func.return");
  });

  it("passes tools/check_air_provenance.py grammar validation for a spawn+delegate+communicate flow", () => {
    const g = new GraphBuilder("spawn_delegate_communicate_air");
    const worker = g.spawnAgent({ agentName: "worker" });
    const task = g.delegate({ taskSpec: "research", targetAgent: "worker" });
    g.addEdge(worker, task);
    const reply = g.communicate({ targetAgent: "worker", message: "status?" });
    g.addEdge(task, reply);
    g.done(reply);

    const air = g.toAir();
    const result = runProvenanceCheck(air);
    expect(result.ok, result.output).toBe(true);
  });

  it("passes tools/check_air_provenance.py grammar validation for a capability invocation", () => {
    const g = new GraphBuilder("capability_air");
    const invoked = g.invokeCapability({ capability: "search:web", params: { query: "apxm" } });
    g.done(invoked);

    const result = runProvenanceCheck(g.toAir());
    expect(result.ok, result.output).toBe(true);
  });

  it("passes tools/check_air_provenance.py grammar validation for a checkpoint flow", () => {
    const g = new GraphBuilder("checkpoint_air");
    const cp = g.checkpoint();
    g.done(cp);

    const result = runProvenanceCheck(g.toAir());
    expect(result.ok, result.output).toBe(true);
  });

  it("passes tools/check_air_provenance.py grammar validation with declared parameters", () => {
    const g = new GraphBuilder("param_air");
    g.param("topic", "str");
    const answer = g.ask({ prompt: "Research: {topic}" });
    g.done(answer);

    const result = runProvenanceCheck(g.toAir());
    expect(result.ok, result.output).toBe(true);
  });
});
