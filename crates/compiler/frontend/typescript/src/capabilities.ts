// Public import path for the generated capability id catalogue.
//
// `import { SEARCH_WEB } from "@apxm/frontend/capabilities"` — the symbols
// themselves are generated from the AIS capability source of truth by `apxm
// codegen capabilities`. This module only gives them a stable path; it is
// deliberately not re-exported from the package root, which carries exactly the
// authoring surface manifest.

export * from "./generated/capabilities.js";
