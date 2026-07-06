#!/usr/bin/env node
/**
 * CLI for compiling TypeScript handler entry files into a tools.json manifest.
 *
 * Usage:
 *   npm run build && node scripts/compile-handlers.mjs --root /path/to/package --out tools.json handler1.ts handler2.ts
 */

import { writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PKG_ROOT = path.resolve(__dirname, "..");
const COMPILE_HANDLERS_PATH = path.join(PKG_ROOT, "dist/compile-handlers.js");

async function loadCompileHandlers() {
  try {
    return await import(pathToFileURL(COMPILE_HANDLERS_PATH).href);
  } catch {
    throw new Error(
      "dist/compile-handlers.js not found — run `npm run build` in this package first",
    );
  }
}

function parseArgs(argv) {
  let outPath = "tools.json";
  let rootDir = undefined;
  const entryPaths = [];

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--out" || arg === "-o") {
      const next = argv[i + 1];
      if (!next) {
        throw new Error("missing value for --out");
      }
      outPath = next;
      i += 1;
      continue;
    }
    if (arg === "--root") {
      const next = argv[i + 1];
      if (!next) {
        throw new Error("missing value for --root");
      }
      rootDir = next;
      i += 1;
      continue;
    }
    if (arg.startsWith("-")) {
      throw new Error(`unknown flag: ${arg}`);
    }
    entryPaths.push(arg);
  }

  if (entryPaths.length === 0) {
    throw new Error(
      "usage: node scripts/compile-handlers.mjs --root /path/to/package --out tools.json handler1.ts [handler2.ts ...]",
    );
  }

  return { outPath, rootDir, entryPaths };
}

async function main() {
  const { outPath, rootDir, entryPaths } = parseArgs(process.argv.slice(2));
  const { compileHandlers } = await loadCompileHandlers();
  const resolvedOut = path.resolve(outPath);
  const manifest = await compileHandlers(entryPaths, {
    rootDir,
    manifestDir: path.dirname(resolvedOut),
  });
  writeFileSync(resolvedOut, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  console.log(`wrote ${manifest.length} handler(s) to ${resolvedOut}`);
}

main().catch((err) => {
  console.error(err instanceof Error ? err.message : err);
  process.exit(1);
});
