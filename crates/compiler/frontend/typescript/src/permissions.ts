// Public import path for the generated permission decision vocabulary.
//
// `import { Allow, Ask } from "@apxm/frontend/permissions"` — the decisions
// themselves are generated from the AIS permission source of truth by `apxm
// codegen permissions`. This module only gives them a stable path; it is
// deliberately not re-exported from the package root, which carries exactly the
// authoring surface manifest.

export * from "./generated/permissions.js";
