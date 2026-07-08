import { describe, expect, it } from "vitest";
import { GraphBuilder } from "../src/builder.js";

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
});
