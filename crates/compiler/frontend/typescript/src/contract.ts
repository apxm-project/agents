// Browser-safe FrontendGraph contract constants shared by authoring capture.

/** The sole FrontendGraph contract accepted by the compiler. */
export const FRONTEND_GRAPH_VERSION = "apxm.frontend-graph";
/** The source-map contract paired with FrontendGraph. */
export const SOURCE_MAP_VERSION = "apxm.source-map";

/** Plain JSON data submitted to the compiler service. */
export type Json = Record<string, unknown>;
