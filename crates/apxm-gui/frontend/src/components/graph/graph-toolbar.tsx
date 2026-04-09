import { useState, useEffect, useCallback, useRef } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchGraph, fetchGraphAnalysis } from "@/api/graph";
import { fetchFileTree } from "@/api/filetree";
import { getErrorMessage } from "@/lib/format";
import type { FileTreeNode } from "@/types/api";

function FileTreeItem({ node, onSelect, depth }: {
  node: FileTreeNode;
  onSelect: (path: string) => void;
  depth: number;
}) {
  const [open, setOpen] = useState(depth < 1);

  if (node.is_dir) {
    return (
      <div className="filetree-dir">
        <button
          type="button"
          className="filetree-dir__toggle"
          onClick={() => setOpen(!open)}
          style={{ paddingLeft: `${depth * 14 + 8}px` }}
        >
          <span className="filetree-dir__arrow">{open ? "\u25BE" : "\u25B8"}</span>
          <span className="filetree-dir__name">{node.name}/</span>
        </button>
        {open && node.children ? (
          <div className="filetree-dir__children">
            {node.children.map((child) => (
              <FileTreeItem key={child.path} node={child} onSelect={onSelect} depth={depth + 1} />
            ))}
          </div>
        ) : null}
      </div>
    );
  }

  const isApxm = node.name.endsWith(".apxm");
  const meta = node.apxm_meta;

  return (
    <button
      type="button"
      className={`filetree-file${isApxm ? " filetree-file--apxm" : ""}`}
      style={{ paddingLeft: `${depth * 14 + 8}px` }}
      onClick={() => onSelect(node.path)}
      disabled={!isApxm}
      title={isApxm ? `Load ${meta?.name ?? node.name}` : node.name}
    >
      <span className="filetree-file__icon">{isApxm ? "\u25C6" : "\u25CB"}</span>
      <span className="filetree-file__name">{node.name}</span>
      {meta?.node_count != null ? (
        <span className="filetree-file__meta">{meta.node_count}n</span>
      ) : null}
    </button>
  );
}

export function GraphToolbar() {
  const graphData = useAppStore((s) => s.graphData);
  const graphPath = useAppStore((s) => s.graphPath);
  const layoutDirection = useAppStore((s) => s.layoutDirection);
  const setLayoutDirection = useAppStore((s) => s.setLayoutDirection);
  const setGraphData = useAppStore((s) => s.setGraphData);
  const setGraphAnalysis = useAppStore((s) => s.setGraphAnalysis);
  const setError = useAppStore((s) => s.setError);

  const [browserOpen, setBrowserOpen] = useState(false);
  const [tree, setTree] = useState<FileTreeNode[]>([]);
  const [rootName, setRootName] = useState("");
  const [loading, setLoading] = useState(false);
  const browserRef = useRef<HTMLDivElement>(null);

  // Close dropdown on click outside
  useEffect(() => {
    if (!browserOpen) return;
    function handleMouseDown(e: MouseEvent) {
      if (browserRef.current && !browserRef.current.contains(e.target as Node)) {
        setBrowserOpen(false);
      }
    }
    document.addEventListener("mousedown", handleMouseDown);
    return () => document.removeEventListener("mousedown", handleMouseDown);
  }, [browserOpen]);

  // Close dropdown on Escape key
  useEffect(() => {
    if (!browserOpen) return;
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        setBrowserOpen(false);
      }
    }
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [browserOpen]);

  const loadTree = useCallback(async () => {
    setLoading(true);
    try {
      const data = await fetchFileTree();
      setTree(data.tree);
      setRootName(data.root);
    } catch {
      setTree([]);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    if (browserOpen && tree.length === 0) {
      loadTree();
    }
  }, [browserOpen, tree.length, loadTree]);

  function handleToggleDirection() {
    setLayoutDirection(layoutDirection === "DOWN" ? "RIGHT" : "DOWN");
  }

  async function handleSelectFile(path: string) {
    setBrowserOpen(false);
    try {
      const graph = await fetchGraph(path);
      setGraphData(graph, path);
      setError("graph", null);
    } catch (e) {
      setError("graph", getErrorMessage(e));
    }
  }

  async function handleAnalyze() {
    if (!graphPath) return;
    try {
      const analysis = await fetchGraphAnalysis(graphPath);
      setGraphAnalysis(analysis);
    } catch (e) {
      setError("analysis", getErrorMessage(e));
    }
  }

  return (
    <div className="graph-toolbar">
      <div className="graph-toolbar__info">
        <span className="graph-toolbar__name">{graphData?.name ?? "No graph loaded"}</span>
        {graphData ? (
          <span className="graph-toolbar__stats">
            {graphData.nodes.length} node{graphData.nodes.length !== 1 ? "s" : ""} &middot;{" "}
            {graphData.edges.length} edge{graphData.edges.length !== 1 ? "s" : ""}
          </span>
        ) : null}
      </div>
      <div className="graph-toolbar__actions">
        <div className="graph-toolbar__browser-wrap" ref={browserRef}>
          <button
            type="button"
            className={`ghost-button${browserOpen ? " ghost-button--active" : ""}`}
            onClick={() => setBrowserOpen(!browserOpen)}
          >
            Browse Server
          </button>
          {browserOpen ? (
            <div className="filetree-dropdown">
              <div className="filetree-dropdown__header">
                <strong>{rootName}/</strong>
                <button type="button" className="filetree-dropdown__close" onClick={() => setBrowserOpen(false)}>&times;</button>
              </div>
              <div className="filetree-dropdown__body">
                {loading ? (
                  <div className="filetree-dropdown__loading">Loading...</div>
                ) : tree.length === 0 ? (
                  <div className="filetree-dropdown__empty">No workflow files found</div>
                ) : (
                  tree.map((node) => (
                    <FileTreeItem key={node.path} node={node} onSelect={handleSelectFile} depth={0} />
                  ))
                )}
              </div>
            </div>
          ) : null}
        </div>
        <button type="button" className="ghost-button" onClick={handleToggleDirection}>
          {layoutDirection === "DOWN" ? "\u2195 Vertical" : "\u2194 Horizontal"}
        </button>
        {graphPath ? (
          <button type="button" className="ghost-button" onClick={handleAnalyze}>Analyze</button>
        ) : null}
      </div>
    </div>
  );
}
