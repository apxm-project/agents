// Bundles TypeScript Capability handlers into the portable handler manifest.

import { mkdtemp, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { build } from "esbuild";

import { isFunctionTool, makeHandlerId } from "./index.mjs";

const MANIFEST_VERSION = "apxm.handler-manifest.v1";
const SOURCE_DIRECTORY = "handlers";
const HANDLER_ID_PREFIX = "sha256:";
const PACKAGE_ROOT = path.dirname(fileURLToPath(import.meta.url));
const AUTHORING_ENTRY = path.join(PACKAGE_ROOT, "index.mjs");

function moduleIdFromPath(entryPath, rootDir) {
  const absolute = path.resolve(entryPath);
  const root = path.resolve(rootDir);
  const relative = path.relative(root, absolute);
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error(`handler ${entryPath} is outside package root ${rootDir}`);
  }
  return relative.split(path.sep).join("/").replace(/\.[^.]+$/, "");
}

async function loadModule(sourcePath, moduleId) {
  const directory = await mkdtemp(path.join(PACKAGE_ROOT, ".handler-build-"));
  const output = path.join(directory, "handler.mjs");
  try {
    await build({
      entryPoints: [sourcePath],
      outfile: output,
      bundle: true,
      platform: "node",
      format: "esm",
      alias: { "@apxm/agent-packaging": AUTHORING_ENTRY },
    });
    process.env.APXM_HANDLER_MODULE = moduleId;
    return await import(`${pathToFileURL(output).href}?t=${Date.now()}`);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

async function bundledSource(sourcePath, handlerId) {
  const result = await build({
    absWorkingDir: path.dirname(sourcePath),
    entryPoints: [path.basename(sourcePath)],
    bundle: true,
    platform: "node",
    format: "esm",
    write: false,
    alias: { "@apxm/agent-packaging": AUTHORING_ENTRY },
  });
  const content = result.outputFiles[0]?.text;
  if (!content) throw new Error(`failed to bundle handler source ${sourcePath}`);
  return {
    artifact_path: `${SOURCE_DIRECTORY}/${handlerId.slice(HANDLER_ID_PREFIX.length)}.mjs`,
    content,
  };
}

/** Compile package-local handlers into the canonical sidecar. */
export async function compileHandlers(entryPaths, options = {}) {
  if (!options.rootDir) throw new Error("rootDir is required");
  const handlers = [];
  const seen = new Set();
  for (const entryPath of entryPaths) {
    const absolute = path.resolve(entryPath);
    const module = moduleIdFromPath(absolute, options.rootDir);
    const exports = await loadModule(absolute, module);
    for (const [exportName, value] of Object.entries(exports)) {
      if (!isFunctionTool(value)) continue;
      const qualname = exportName === "default" ? value.qualname : exportName;
      const handler_id = makeHandlerId(module, qualname);
      if (seen.has(handler_id)) continue;
      seen.add(handler_id);
      handlers.push({
        kind: "tool",
        language: "typescript",
        handler_id,
        module,
        qualname,
        name: value.name,
        source: await bundledSource(absolute, handler_id),
        description: value.description,
        schema: value.schema,
      });
    }
  }
  return { version: MANIFEST_VERSION, handlers };
}
