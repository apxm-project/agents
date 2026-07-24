// In-process compiler bridge over the native Node-API extension.
//
// Submits an emitted FrontendGraph for verification, canonical AIR lowering, and
// artifact compilation. These are compiler results, not authoring APIs: authors
// write declarations and control flow, never a graph object.

import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));

export const FRONTEND_GRAPH_VERSION = "apxm.frontend-graph.v1";
export const SOURCE_MAP_VERSION = "apxm.source-map.v1";

export type Json = Record<string, unknown>;

type NativeBridge = {
  compileFrontendGraphArtifact(json: string): string;
  lowerFrontendGraph(json: string): string;
  verifyFrontendGraph(json: string): string | null;
};

let loadedBridge: NativeBridge | undefined;

function nativeBridge(): NativeBridge {
  if (loadedBridge === undefined) {
    // The compiled bridge and the native extension are colocated; from source
    // (test runs) the extension is in the sibling build output.
    const candidates = [
      join(here, "_native.node"),
      join(here, "..", "dist", "_native.node"),
    ];
    let lastCause: unknown;
    for (const candidate of candidates) {
      try {
        loadedBridge = require(candidate) as NativeBridge;
        return loadedBridge;
      } catch (cause) {
        lastCause = cause;
      }
    }
    throw new Error(
      "The @apxm/frontend native compiler bridge is not installed for this platform",
      { cause: lastCause },
    );
  }
  return loadedBridge;
}

export function verifyGraph(graph: Json): string | null {
  return nativeBridge().verifyFrontendGraph(JSON.stringify(graph));
}

export function lowerGraph(graph: Json): Json {
  return JSON.parse(nativeBridge().lowerFrontendGraph(JSON.stringify(graph))) as Json;
}

export function canonicalAirJson(graph: Json): string {
  return nativeBridge().lowerFrontendGraph(JSON.stringify(graph));
}

export function compileArtifact(graph: Json): Json {
  return JSON.parse(
    nativeBridge().compileFrontendGraphArtifact(JSON.stringify(graph)),
  ) as Json;
}
