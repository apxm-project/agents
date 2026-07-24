// Node-only compiler-service bridge kept outside the browser authoring entrypoint.

import { canonicalAirJson, compileArtifact, verifyGraph } from "./bridge.js";
import { installCompilerService } from "./compiler-service.js";

installCompilerService({
  verifyGraph,
  canonicalAir: canonicalAirJson,
  artifact: compileArtifact,
});
