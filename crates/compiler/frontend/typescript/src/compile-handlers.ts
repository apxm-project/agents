import { mkdtemp, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { build } from "esbuild";

import {
  isFunctionTool,
  isHookFn,
  makeHandlerId,
} from "./handlers/index.js";
import type { FunctionTool, HookFn, JsonSchema } from "./handlers/index.js";

/** Manifest entry matching the Python tools manifest shape. */
export interface HandlerManifestEntry {
  handler_id: string;
  module: string;
  qualname: string;
  name: string;
  source_file?: string;
  description?: string;
  schema?: JsonSchema;
  event?: string;
  match?: string;
  mode?: string;
}

export interface CompileHandlersOptions {
  rootDir?: string;
  manifestDir?: string;
}

function normalizeModulePath(value: string): string {
  return value.split(path.sep).join("/");
}

function withoutExtension(value: string): string {
  return value.replace(/\.[^.]+$/, "");
}

function moduleIdFromPath(entryPath: string, rootDir?: string): string {
  const absPath = path.resolve(entryPath);
  if (!rootDir) {
    return normalizeModulePath(withoutExtension(absPath));
  }
  const root = path.resolve(rootDir);
  const rel = path.relative(root, absPath);
  if (rel.startsWith("..") || path.isAbsolute(rel)) {
    throw new Error(
      `handler ${entryPath} is outside package root ${rootDir}`,
    );
  }
  return normalizeModulePath(withoutExtension(rel));
}

function sourceFileFromPath(entryPath: string, rootDir?: string): string {
  const absPath = path.resolve(entryPath);
  if (!rootDir) {
    return normalizeModulePath(absPath);
  }
  const root = path.resolve(rootDir);
  const rel = path.relative(root, absPath);
  if (rel.startsWith("..") || path.isAbsolute(rel)) {
    throw new Error(
      `handler ${entryPath} is outside package root ${rootDir}`,
    );
  }
  return normalizeModulePath(rel);
}

function resolveQualname(exportName: string, fallback: string): string {
  return exportName === "default" ? fallback : exportName;
}

function toolManifest(
  tool: FunctionTool,
  moduleName: string,
  qualname: string,
  sourceFile: string,
): HandlerManifestEntry {
  const handler_id = makeHandlerId(moduleName, qualname);
  return {
    handler_id,
    module: moduleName,
    qualname,
    name: tool.name,
    description: tool.description,
    schema: tool.schema,
    source_file: sourceFile,
  };
}

function hookManifest(
  hookFn: HookFn,
  moduleName: string,
  qualname: string,
  sourceFile: string,
): HandlerManifestEntry {
  const handler_id = makeHandlerId(moduleName, qualname);
  return {
    handler_id,
    module: moduleName,
    qualname,
    name: hookFn.name,
    event: hookFn.event,
    match: hookFn.match,
    mode: hookFn.mode,
    source_file: sourceFile,
  };
}

const PKG_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const FRONTEND_ENTRY = path.join(PKG_ROOT, "dist", "index.js");

async function loadTsModule(
  absPath: string,
  moduleId: string,
): Promise<Record<string, unknown>> {
  const dir = await mkdtemp(path.join(PKG_ROOT, ".handler-build-"));
  const outfile = path.join(dir, "handler.mjs");

  try {
    await build({
      entryPoints: [absPath],
      outfile,
      bundle: true,
      platform: "node",
      format: "esm",
      packages: "external",
      alias: {
        "@apxm/frontend": FRONTEND_ENTRY,
      },
    });

    process.env.APXM_HANDLER_MODULE = moduleId;

    const mod = await import(`${pathToFileURL(outfile).href}?t=${Date.now()}`);
    return mod as Record<string, unknown>;
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

/**
 * Dynamically import handler entry modules and emit a tools.json manifest array
 * matching the Python tools manifest format.
 */
export async function compileHandlers(
  entryPaths: string[],
  options: CompileHandlersOptions = {},
): Promise<HandlerManifestEntry[]> {
  const manifest: HandlerManifestEntry[] = [];
  const seen = new Set<string>();

  for (const entryPath of entryPaths) {
    const absPath = path.resolve(entryPath);
    const moduleName = moduleIdFromPath(absPath, options.rootDir);
    const sourceFile = sourceFileFromPath(absPath, options.rootDir);
    const mod = await loadTsModule(absPath, moduleName);

    for (const [exportName, value] of Object.entries(mod)) {
      if (isFunctionTool(value)) {
        const qualname = resolveQualname(exportName, value.qualname);
        const entry = toolManifest(value, moduleName, qualname, sourceFile);
        if (!seen.has(entry.handler_id)) {
          seen.add(entry.handler_id);
          manifest.push(entry);
        }
        continue;
      }

      if (isHookFn(value)) {
        const qualname = resolveQualname(exportName, value.qualname);
        const entry = hookManifest(value, moduleName, qualname, sourceFile);
        if (!seen.has(entry.handler_id)) {
          seen.add(entry.handler_id);
          manifest.push(entry);
        }
      }
    }
  }

  return manifest;
}
