// Portable TypeScript handler-manifest coverage.

import { mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import {
  HANDLER_KIND,
  HANDLER_LANGUAGE,
  HANDLER_MANIFEST_VERSION,
  HANDLER_MANIFEST_SOURCE_DIRECTORY,
  compileHandlers,
} from "../src/compile-handlers.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryDirectories.splice(0).map((directory) =>
      rm(directory, { recursive: true, force: true }),
    ),
  );
});

describe("compileHandlers", () => {
  it("embeds bundled artifact-local source in the versioned manifest", async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), "apxm-ts-handler-"));
    temporaryDirectories.push(root);
    const source = path.join(root, "handler.ts");
    await writeFile(
      source,
      [
        'import { tool } from "@apxm/frontend";',
        'export const echo = tool(function echo(args: Record<string, unknown>) { return args; });',
      ].join("\n"),
      "utf8",
    );

    const manifest = await compileHandlers([source], { rootDir: root });

    expect(manifest.version).toBe(HANDLER_MANIFEST_VERSION);
    expect(manifest.handlers).toHaveLength(1);
    expect(manifest.handlers[0]).toMatchObject({
      kind: HANDLER_KIND.TOOL,
      language: HANDLER_LANGUAGE.TYPESCRIPT,
      source: {
        artifact_path: expect.stringMatching(
          new RegExp(`^${HANDLER_MANIFEST_SOURCE_DIRECTORY}/[0-9a-f]{64}\\.mjs$`),
        ),
      },
    });
    expect(manifest.handlers[0]?.source.content).toContain("echo");
    expect(manifest.handlers[0]?.source.content).not.toContain(root);
  });
});
