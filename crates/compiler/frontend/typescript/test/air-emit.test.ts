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

  // WF-3 manual cross-check: the Python reference emitter's new vector
  // fixtures (workspace/contracts/vectors/air/{multi_agent_delegate,
  // capability_pipeline, checkpoint_pipeline}.air) each cover a flow shape;
  // these three cases build the same shape with the TS builder and confirm
  // it independently produces grammar-valid AIR too. Byte-identical output
  // between the two frontends is a separate, later milestone (TSF-4) — this
  // only proves both frontends can express the shape and pass the shared
  // grammar gate.
  describe("WF-3 cross-check against the Python vector fixture shapes", () => {
    it("multi_agent_delegate: spawn_agent + delegate + communicate", () => {
      const g = new GraphBuilder("multi_agent_delegate");
      const worker = g.spawnAgent({ agentName: "worker", agentRoute: "auto" });
      const task = g.delegate({
        name: "delegate_task",
        taskSpec: "Summarize the quarterly report.",
        targetAgent: "worker",
      });
      g.addEdge(worker, task, "Control");
      const reply = g.communicate({
        name: "collect_result",
        targetAgent: "worker",
        message: "Findings so far: {task}. Send back your final summary.",
        inputs: { task },
      });
      g.done(reply);

      const result = runProvenanceCheck(g.toAir());
      expect(result.ok, result.output).toBe(true);
    });

    it("capability_pipeline: chained capability invocations", () => {
      // Note: unlike ask()/communicate(), invokeCapability() does not accept
      // an `inputs` map (its options type has no such field and its `...rest`
      // spread does not filter one out — passing one would leak a NodeRef
      // into node attributes). Wire the dependency explicitly instead.
      const g = new GraphBuilder("capability_pipeline");
      const lookup = g.invokeCapability({
        name: "invoke_lookup",
        capability: "lookup_docs",
        params: { query: "release checklist" },
      });
      const digest = g.invokeCapability({
        name: "invoke_digest",
        capability: "summarize_text",
        params: "{\"text\": \"{lookup}\"}",
      });
      g.addEdge(lookup, digest);
      g.done(digest);

      const result = runProvenanceCheck(g.toAir());
      expect(result.ok, result.output).toBe(true);
    });

    it("checkpoint_pipeline: ask -> checkpoint -> ask", () => {
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

      const result = runProvenanceCheck(g.toAir());
      expect(result.ok, result.output).toBe(true);
    });
  });
});
