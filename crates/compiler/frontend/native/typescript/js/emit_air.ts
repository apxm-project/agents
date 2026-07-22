// Print the canonical AIR for the shared authoring example. Used by the
// cross-language parity harness.

import { canonicalAirJson, compileArtifact } from "./index.ts";
import { externalAgentGraph, specialistGraph } from "./example.ts";
import { Gao } from "./gao.ts";

const which = process.argv[2] ?? "specialist";
if (which === "gao-artifact") {
  process.stdout.write(JSON.stringify(compileArtifact(Gao.build().buildGraph())));
  process.exit(0);
}
const graph =
  which === "external-agent"
    ? externalAgentGraph()
    : which === "gao"
      ? Gao.build().buildGraph()
      : specialistGraph();
process.stdout.write(canonicalAirJson(graph));
