// Packed TypeScript frontend clean-consumer imports and removed-surface absence.

import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { describe, expect, it } from "vitest";

const PACKAGE_DIR = new URL("..", import.meta.url).pathname;

function run(command: string, args: string[], cwd: string) {
  return spawnSync(command, args, { cwd, encoding: "utf8" });
}

describe("packed @apxm/frontend", () => {
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
        'const removed = ["ConversationalAgent", "Gao", "TurnSpec", "SpecialistComposition"];',
        "if (typeof frontend.AgentProgram !== 'function') process.exit(2);",
        "if (removed.some((name) => name in frontend)) process.exit(3);",
        "const program = new frontend.AgentProgram({",
        "  program_id: 'clean',",
        "  input_type_ref: 'Input',",
        "  output_type_ref: 'Output',",
        "});",
        "const empty = () => undefined;",
        "program.branch('region.branch', empty, empty);",
        "program.switch('region.switch', [empty]);",
        "program.loop('region.loop', empty);",
        "program.parallel('region.parallel', empty);",
        "program.tryCatch('region.try', 'region.catch', empty, empty);",
        "program.throwRegion('region.throw');",
        "program.returnRegion('region.return');",
        "program.yieldRegion('region.yield');",
        "if (program.buildGraph().schema_version !== 'apxm.frontend-graph.v1') process.exit(4);",
        "if (program.verifyGraph() !== null) process.exit(5);",
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
  });
});
