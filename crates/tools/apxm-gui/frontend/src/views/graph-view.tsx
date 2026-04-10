import { GraphCanvas } from "@/components/graph/graph-canvas";
import { FileSidebar } from "@/components/graph/file-sidebar";
import { NodeInspector } from "@/components/inspector/node-inspector";
import { useAppStore } from "@/store/app-store";

export function GraphView() {
  const inspectorOpen = useAppStore((s) => s.inspectorOpen);

  return (
    <div className="explorer-layout">
      <div className="explorer-layout__sidebar">
        <FileSidebar />
      </div>
      <div className="explorer-layout__main">
        <GraphCanvas />
      </div>
      {inspectorOpen ? (
        <div className="explorer-layout__inspector">
          <NodeInspector />
        </div>
      ) : null}
    </div>
  );
}
