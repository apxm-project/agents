// Public import path for the generated Hook scope vocabulary.
//
// `import { CAPABILITY, MODEL } from "@apxm/frontend/scopes"` — the scopes
// themselves are generated from the `apxm.frontend-graph` contract by `apxm
// codegen frontend-vocabulary`. This module only gives them a stable path; it is
// deliberately not re-exported from the package root, which carries exactly the
// authoring surface manifest.

export * from "./generated/scopes.js";
