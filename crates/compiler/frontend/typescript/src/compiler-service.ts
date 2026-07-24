// Optional compiler-service registration for graph verification and artifact output.

import type { Json } from "./contract.js";

type CompilerService = {
  verifyGraph(graph: Json): string | null;
  canonicalAir(graph: Json): string;
  artifact(graph: Json): Json;
};

let installed: CompilerService | undefined;

/** Install the host-owned compiler service for this authoring environment. */
export function installCompilerService(service: CompilerService): void {
  installed = service;
}

/** Return the installed compiler service or fail before compilation begins. */
export function compilerService(): CompilerService {
  if (installed === undefined) {
    throw new Error("a compiler service must be installed before graph compilation");
  }
  return installed;
}
