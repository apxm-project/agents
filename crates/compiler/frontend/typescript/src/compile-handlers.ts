import { mkdtemp, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { build } from "esbuild";

import {
  isFunctionTool,
  isHookFn,
  makeHandlerId,
} from "./handlers/index.js";
import type { FunctionTool, HookFn } from "./handlers/index.js";

/** Version identifier for the portable frontend handler sidecar. */
export const HANDLER_MANIFEST_VERSION = "apxm.handler-manifest.v1";
/** Mirror of crates/machine/contracts/src/types/handler_manifest.rs. */
export const HANDLER_MANIFEST_SOURCE_DIRECTORY = "handlers";
/** Prefix for content-addressed handler identities. */
export const HANDLER_MANIFEST_HANDLER_ID_PREFIX = "sha256:";

/** Runtime roles supported by the portable handler-manifest contract. */
export const HANDLER_KIND = {
  TOOL: "tool",
  HOOK: "hook",
} as const;
export type HandlerKind = (typeof HANDLER_KIND)[keyof typeof HANDLER_KIND];

/** Authoring languages supported by the portable handler-manifest contract. */
export const HANDLER_LANGUAGE = {
  TYPESCRIPT: "typescript",
} as const;
export type HandlerLanguage = (typeof HANDLER_LANGUAGE)[keyof typeof HANDLER_LANGUAGE];

/** Artifact-local source transported with a handler descriptor. */
export interface HandlerSource {
  artifact_path: string;
  content: string;
}

/** One tool or hook descriptor in the portable handler sidecar. */
export interface HandlerManifestEntry {
  kind: HandlerKind;
  language: HandlerLanguage;
  handler_id: string;
  module: string;
  qualname: string;
  name: string;
  source: HandlerSource;
  description: string;
  schema: Record<string, unknown>;
  event?: string;
  match?: string;
  mode?: string;
}

/** The only serialized handler sidecar shape emitted by TypeScript. */
export interface HandlerManifest {
  version: typeof HANDLER_MANIFEST_VERSION;
  handlers: HandlerManifestEntry[];
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

function resolveQualname(exportName: string, defaultQualname: string): string {
  return exportName === "default" ? defaultQualname : exportName;
}

async function sourceForHandler(
  absPath: string,
  handlerId: string,
): Promise<HandlerSource> {
  const result = await build({
    absWorkingDir: path.dirname(absPath),
    entryPoints: [path.basename(absPath)],
    bundle: true,
    platform: "node",
    format: "esm",
    packages: "external",
    write: false,
    alias: {
      "@apxm/frontend": FRONTEND_ENTRY,
    },
  });
  const content = result.outputFiles[0]?.text;
  if (!content) {
    throw new Error(`failed to bundle handler source ${absPath}`);
  }
  return {
    artifact_path:
      HANDLER_MANIFEST_SOURCE_DIRECTORY +
      "/" +
      handlerId.slice(HANDLER_MANIFEST_HANDLER_ID_PREFIX.length) +
      ".mjs",
    content,
  };
}

async function toolManifest(
  tool: FunctionTool,
  moduleName: string,
  qualname: string,
  sourcePath: string,
): Promise<HandlerManifestEntry> {
  const handler_id = makeHandlerId(moduleName, qualname);
  return {
    kind: HANDLER_KIND.TOOL,
    language: HANDLER_LANGUAGE.TYPESCRIPT,
    handler_id,
    module: moduleName,
    qualname,
    name: tool.name,
    source: await sourceForHandler(sourcePath, handler_id),
    description: tool.description,
    schema: {},
  };
}

async function hookManifest(
  hookFn: HookFn,
  moduleName: string,
  qualname: string,
  sourcePath: string,
): Promise<HandlerManifestEntry> {
  const handler_id = makeHandlerId(moduleName, qualname);
  return {
    kind: HANDLER_KIND.HOOK,
    language: HANDLER_LANGUAGE.TYPESCRIPT,
    handler_id,
    module: moduleName,
    qualname,
    name: hookFn.name,
    source: await sourceForHandler(sourcePath, handler_id),
    description: "",
    schema: {},
    event: hookFn.event,
    match: hookFn.match,
    mode: hookFn.mode,
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
 * Dynamically import handler entry modules and emit the portable handler
 * manifest consumed by the artifact runtime.
 */
export async function compileHandlers(
  entryPaths: string[],
  options: CompileHandlersOptions = {},
): Promise<HandlerManifest> {
  const handlers: HandlerManifestEntry[] = [];
  const seen = new Set<string>();

  for (const entryPath of entryPaths) {
    const absPath = path.resolve(entryPath);
    const moduleName = moduleIdFromPath(absPath, options.rootDir);
    const mod = await loadTsModule(absPath, moduleName);

    for (const [exportName, value] of Object.entries(mod)) {
      if (isFunctionTool(value)) {
        const qualname = resolveQualname(exportName, value.qualname);
        const entry = await toolManifest(value, moduleName, qualname, absPath);
        if (!seen.has(entry.handler_id)) {
          seen.add(entry.handler_id);
          handlers.push(entry);
        }
        continue;
      }

      if (isHookFn(value)) {
        const qualname = resolveQualname(exportName, value.qualname);
        const entry = await hookManifest(value, moduleName, qualname, absPath);
        if (!seen.has(entry.handler_id)) {
          seen.add(entry.handler_id);
          handlers.push(entry);
        }
      }
    }
  }

  return { version: HANDLER_MANIFEST_VERSION, handlers };
}
