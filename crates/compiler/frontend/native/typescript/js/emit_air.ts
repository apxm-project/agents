// Print the canonical AIR for the shared authoring example. Used by the
// cross-language parity harness.

import { canonicalAirJson } from "./index.ts";
import { specialistGraph } from "./example.ts";

process.stdout.write(canonicalAirJson(specialistGraph()));
