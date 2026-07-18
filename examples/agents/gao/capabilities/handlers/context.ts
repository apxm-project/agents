// Resolves Gao's package root for capability-local resource access.
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

let cachedRoot: string | null = null;

export function packageRoot(): string {
  if (cachedRoot) {
    return cachedRoot;
  }
  const here = dirname(fileURLToPath(import.meta.url));
  const root = resolve(here, "..", "..");
  cachedRoot = root;
  return root;
}
