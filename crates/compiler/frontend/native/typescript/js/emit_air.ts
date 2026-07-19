// Print the canonical AIR for the shared authoring example. Used by the
// cross-language parity harness.

import { canonicalAirJson } from "./index.ts";
import { externalAgentGraph, gaoConversationalGraph, specialistGraph } from "./example.ts";

const which = process.argv[2] ?? "specialist";
const graph =
  which === "external-agent"
    ? externalAgentGraph()
    : which === "gao"
      ? gaoConversationalGraph()
      : specialistGraph();
process.stdout.write(canonicalAirJson(graph));
