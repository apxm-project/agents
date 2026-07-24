// Browser-safe FrontendGraph contract constants shared by authoring capture.

/** The sole FrontendGraph contract accepted by the compiler. */
export const FRONTEND_GRAPH_VERSION = "apxm.frontend-graph.v1";
/** The source-map contract paired with FrontendGraph v1. */
export const SOURCE_MAP_VERSION = "apxm.source-map.v1";

/** Plain JSON data submitted to the compiler service. */
export type Json = Record<string, unknown>;
