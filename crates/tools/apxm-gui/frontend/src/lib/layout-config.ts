export const ELK_NODE_WIDTH = 264;
export const ELK_NODE_HEIGHT = 120;

const ELK_BASE_OPTIONS: Record<string, string> = {
  "elk.algorithm": "layered",
  "elk.edgeRouting": "ORTHOGONAL",
  "elk.layered.crossingMinimization.strategy": "LAYER_SWEEP",
  "elk.layered.considerModelOrder.strategy": "PREFER_NODES",
  "elk.layered.nodePlacement.strategy": "NETWORK_SIMPLEX",
  "elk.layered.nodePlacement.bk.fixedAlignment": "BALANCED",
  "elk.layered.unnecessaryBendpoints": "true",
  "elk.padding": "[top=48,left=72,bottom=72,right=72]",
  "elk.spacing.nodeNode": "56",
  "elk.layered.spacing.nodeNodeBetweenLayers": "96",
  "elk.spacing.edgeNode": "42",
  "elk.spacing.edgeEdge": "24",
};

export const ELK_LAYOUT_DOWN: Record<string, string> = { ...ELK_BASE_OPTIONS, "elk.direction": "DOWN" };
export const ELK_LAYOUT_RIGHT: Record<string, string> = { ...ELK_BASE_OPTIONS, "elk.direction": "RIGHT" };

export function elkLayoutOptionsForDirection(direction: "DOWN" | "RIGHT"): Record<string, string> {
  return direction === "DOWN" ? ELK_LAYOUT_DOWN : ELK_LAYOUT_RIGHT;
}

export const FALLBACK_GRID_SPACING = { x: 332, y: 236 };
