// Execute a TypeScript frontend source file against this installed frontend package.

import { existsSync } from "node:fs";
import * as nodeModule from "node:module";
import path from "node:path";
import { pathToFileURL } from "node:url";

/** Run a frontend source module with its bare frontend import bound to this package. */
export async function runSource(sourcePath: string): Promise<void> {
  if (typeof nodeModule.registerHooks === "function") {
    const frontendUrl = new URL("./index.js", import.meta.url).href;
    nodeModule.registerHooks({
      resolve(specifier, context, nextResolve) {
        if (specifier === "@apxm/frontend") {
          return { url: frontendUrl, shortCircuit: true };
        }
        if (
          specifier.startsWith(".") &&
          specifier.endsWith(".js") &&
          context.parentURL?.startsWith("file:")
        ) {
          const parentPath = new URL(context.parentURL);
          const tsPath = path.resolve(
            path.dirname(parentPath.pathname),
            `${specifier.slice(0, -".js".length)}.ts`,
          );
          if (existsSync(tsPath)) {
            return { url: pathToFileURL(tsPath).href, shortCircuit: true };
          }
        }
        return nextResolve(specifier, context);
      },
    });
  }

  await import(pathToFileURL(path.resolve(sourcePath)).href);
}
