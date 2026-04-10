import { useState, useEffect, useCallback, useMemo } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchGraph, fetchGraphAnalysis } from "@/api/graph";
import { fetchFileTree } from "@/api/filetree";
import { getErrorMessage } from "@/lib/format";
import type { FileTreeNode } from "@/types/api";

function FileTreeItem({ node, onSelect, depth, activePath }: {
  node: FileTreeNode;
  onSelect: (path: string) => void;
  depth: number;
  activePath: string | null;
}) {
  const [open, setOpen] = useState(depth < 2);

  if (node.is_dir) {
    return (
      <div className="fside-dir">
        <button
          type="button"
          className="fside-dir__toggle"
          onClick={() => setOpen(!open)}
          style={{ paddingLeft: `${depth * 16 + 10}px` }}
        >
          <span className="fside-dir__arrow">{open ? "\u25BE" : "\u25B8"}</span>
          <span className="fside-dir__name">{node.name}</span>
        </button>
        {open && node.children ? (
          <div className="fside-dir__children">
            {node.children.map((child) => (
              <FileTreeItem key={child.path} node={child} onSelect={onSelect} depth={depth + 1} activePath={activePath} />
            ))}
          </div>
        ) : null}
      </div>
    );
  }

  const isApxm = node.name.endsWith(".apxm");
  const meta = node.apxm_meta;
  const isActive = activePath === node.path;

  return (
    <button
      type="button"
      className={`fside-file${isApxm ? " fside-file--apxm" : ""}${isActive ? " fside-file--active" : ""}`}
      style={{ paddingLeft: `${depth * 16 + 10}px` }}
      onClick={() => isApxm && onSelect(node.path)}
      disabled={!isApxm}
      title={isApxm ? `Load ${meta?.name ?? node.name}` : node.name}
    >
      <span className="fside-file__icon">{isApxm ? "\u25C6" : "\u25CB"}</span>
      <span className="fside-file__name">{node.name}</span>
      {meta?.node_count != null ? (
        <span className="fside-file__badge">{meta.node_count}n</span>
      ) : null}
    </button>
  );
}

function filterTree(nodes: FileTreeNode[], query: string): FileTreeNode[] {
  if (!query) return nodes;
  const q = query.toLowerCase();
  const results: FileTreeNode[] = [];
  for (const node of nodes) {
    if (node.is_dir) {
      const filteredChildren = filterTree(node.children ?? [], q);
      if (filteredChildren.length > 0) {
        results.push({ ...node, children: filteredChildren });
      }
    } else if (node.name.toLowerCase().includes(q)) {
      results.push(node);
    }
  }
  return results;
}

export function FileSidebar() {
  const graphPath = useAppStore((s) => s.graphPath);
  const setGraphData = useAppStore((s) => s.setGraphData);
  const setGraphAnalysis = useAppStore((s) => s.setGraphAnalysis);
  const setError = useAppStore((s) => s.setError);

  const [tree, setTree] = useState<FileTreeNode[]>([]);
  const [rootName, setRootName] = useState("");
  const [loading, setLoading] = useState(true);
  const [filterQuery, setFilterQuery] = useState("");

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

  useEffect(() => { loadTree(); }, [loadTree]);

  async function handleSelectFile(path: string) {
    try {
      const graph = await fetchGraph(path);
      setGraphData(graph, path);
      setError("graph", null);
      fetchGraphAnalysis(path).then(setGraphAnalysis).catch(() => {});
    } catch (e) {
      setError("graph", getErrorMessage(e));
    }
  }

  const filteredTree = useMemo(() => filterTree(tree, filterQuery), [tree, filterQuery]);

  const apxmCount = useMemo(() => {
    function countApxm(nodes: FileTreeNode[]): number {
      let count = 0;
      for (const n of nodes) {
        if (n.is_dir) count += countApxm(n.children ?? []);
        else if (n.name.endsWith(".apxm")) count++;
      }
      return count;
    }
    return countApxm(tree);
  }, [tree]);

  return (
    <div className="fside">
      <div className="fside__header">
        <span className="fside__title">Explorer</span>
        <span className="fside__count">{apxmCount}</span>
        <button type="button" className="fside__refresh" onClick={loadTree} title="Refresh">&#x21bb;</button>
      </div>
      <div className="fside__search">
        <input
          type="text"
          className="fside__search-input"
          placeholder="Filter files..."
          value={filterQuery}
          onChange={(e) => setFilterQuery(e.target.value)}
        />
      </div>
      <div className="fside__tree">
        {loading ? (
          <div className="fside__empty">Loading...</div>
        ) : filteredTree.length === 0 ? (
          <div className="fside__empty">
            {apxmCount === 0
              ? "No .apxm workflow files found. Generate one from Python:\napxm execute example.py"
              : "No matching files"}
          </div>
        ) : (
          <>
            <div className="fside__root">{rootName}/</div>
            {filteredTree.map((node) => (
              <FileTreeItem key={node.path} node={node} onSelect={handleSelectFile} depth={0} activePath={graphPath} />
            ))}
          </>
        )}
      </div>
    </div>
  );
}
