// Packed TypeScript frontend clean-consumer imports and removed-surface absence.

import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { describe, expect, it } from "vitest";

const PACKAGE_DIR = new URL("..", import.meta.url).pathname;
const PACKED_PACKAGE_INSTALL_TIMEOUT_MS = 30_000;

function run(command: string, args: string[], cwd: string) {
  return spawnSync(command, args, { cwd, encoding: "utf8" });
}

describe("packed @apxm/frontend", () => {
  it("keeps the Node compiler bridge outside the root authoring bundle", () => {
    const rootModules = [
      "index.ts",
      "agent.ts",
      "capture.ts",
      "markers.ts",
      "advanced.ts",
      "contract.ts",
      "compiler-service.ts",
    ];
    for (const module of rootModules) {
      const source = readFileSync(join(PACKAGE_DIR, "src", module), "utf8");
      expect(source, module).not.toContain('from "node:');
      expect(source, module).not.toContain("from 'node:");
    }
  });

  it("installs as a generic-only clean-consumer surface", () => {
    const temp = mkdtempSync(join(tmpdir(), "apxm-frontend-package-"));
    const packed = run(
      "npm",
      ["pack", "--ignore-scripts", "--json", "--pack-destination", temp],
      PACKAGE_DIR,
    );
    expect(packed.status, packed.stderr).toBe(0);
    const [{ filename }] = JSON.parse(packed.stdout) as Array<{ filename: string }>;

    writeFileSync(
      join(temp, "package.json"),
      JSON.stringify({ private: true, type: "module" }),
    );
    const installed = run(
      "npm",
      [
        "install",
        "--ignore-scripts",
        "--no-audit",
        "--no-fund",
        "--no-package-lock",
        join(temp, filename),
      ],
      temp,
    );
    expect(installed.status, installed.stderr).toBe(0);

    const genericScript = join(temp, "generic.mjs");
    writeFileSync(
      genericScript,
      [
        'import * as frontend from "@apxm/frontend";',
        'import "@apxm/frontend/node";',
        'import { readFileSync } from "node:fs";',
        'import { fileURLToPath } from "node:url";',
        'const removed = ["AgentProgram", "AgentFacade", "FIVE_OPS", "OP_MODEL_CALL",',
        '  "OP_CAPABILITY_INVOKE", "canonicalAirJson", "lower", "verify",',
        '  "ConversationalAgent", "Gao", "StructuredTaskScope", "AgentConfig",',
        '  "AgentDefinition", "ProgramInstance", "decodeFact", "RuntimeFact"];',
        "if (typeof frontend.Agent !== 'function') process.exit(2);",
        "if (removed.some((name) => name in frontend)) process.exit(3);",
        'const expected = ["Agent", "Capability", "Context", "Event", "Hook", "Model", "TaskGroup", "Tool"];',
        "if (JSON.stringify(Object.keys(frontend).sort()) !== JSON.stringify(expected)) process.exit(7);",
        "const { Agent, Context, Model, Tool } = frontend;",
        "const Weather = Tool('weather.capability');",
        "const Planner = Model('planner.model');",
        "const TripCtx = Context({ legs: [] });",
        "const sourceFile = fileURLToPath(import.meta.url);",
        "const source = { fileName: sourceFile, text: readFileSync(sourceFile, 'utf8') };",
        "const Plan = Agent({",
        "  name: 'Plan', source, context: TripCtx, use: { Weather, Planner },",
        "  async run(agent, request) {",
        "    while (true) {",
        "      if (request !== null) { await Weather(request); }",
        "      const plan = await Planner(request);",
        "      agent.context = { legs: [] };",
        "      return plan;",
        "    }",
        "  },",
        "});",
        "const graph = Plan.frontendGraph();",
        "if (graph.schema_version !== 'apxm.frontend-graph') process.exit(4);",
        "if (Plan.diagnostics() !== null) process.exit(5);",
        "const air = JSON.parse(Plan.canonicalAir());",
        "const ops = new Set(air.semantic_operations.map((o) => o.op));",
        "if (!ops.has('capability.invoke') || !ops.has('model.call')) process.exit(6);",
      ].join("\n"),
    );
    const generic = run("node", [genericScript], temp);
    expect(generic.status, generic.stderr).toBe(0);

    for (const subpath of ["conversational", "gao"]) {
      const absentScript = join(temp, `absent-${subpath}.mjs`);
      writeFileSync(absentScript, `import "@apxm/frontend/${subpath}";\n`);
      const absent = run("node", [absentScript], temp);
      expect(absent.status).not.toBe(0);
      expect(readFileSync(absentScript, "utf8")).toContain(subpath);
    }
  }, PACKED_PACKAGE_INSTALL_TIMEOUT_MS);
});
