import { GraphCanvas } from "@/components/graph/graph-canvas";
import { NodeInspector } from "@/components/inspector/node-inspector";
import { useAppStore } from "@/store/app-store";

export function GraphView() {
  const inspectorOpen = useAppStore((s) => s.inspectorOpen);

  return (
    <div className="view-split">
      <div className="view-split__main">
        <GraphCanvas />
      </div>
      {inspectorOpen ? (
        <div className="view-split__side">
          <NodeInspector />
        </div>
      ) : null}
    </div>
  );
}
