import { useState, useCallback } from "react";
import { useAppStore } from "@/store/app-store";

type Props = {
  open: boolean;
  onClose: () => void;
};

export function NewWorkflowDialog({ open, onClose }: Props) {
  const newGraph = useAppStore((s) => s.newGraph);
  const [name, setName] = useState("my_workflow");

  const handleCreate = useCallback(() => {
    if (!name.trim()) return;
    newGraph(name.trim());
    onClose();
    setName("my_workflow");
  }, [name, newGraph, onClose]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") handleCreate();
      if (e.key === "Escape") onClose();
    },
    [handleCreate, onClose],
  );

  if (!open) return null;

  return (
    <div className="execute-overlay" onClick={onClose}>
      <div className="execute-dialog" onClick={(e) => e.stopPropagation()}>
        <div className="execute-dialog__header">
          <h3>New Workflow</h3>
          <button type="button" className="execute-dialog__close" onClick={onClose}>×</button>
        </div>
        <div className="execute-dialog__info">
          Create a new empty workflow graph. You can add nodes from the operation
          palette and connect them visually.
        </div>
        <div className="execute-dialog__params">
          <div className="execute-dialog__param">
            <label className="execute-dialog__param-label">Workflow Name</label>
            <input
              className="execute-dialog__param-input"
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={handleKeyDown}
              autoFocus
              placeholder="my_workflow"
            />
          </div>
        </div>
        <div className="execute-dialog__actions">
          <button type="button" className="ghost-button" onClick={onClose}>Cancel</button>
          <button
            type="button"
            className="execute-dialog__run-btn"
            onClick={handleCreate}
            disabled={!name.trim()}
          >
            Create
          </button>
        </div>
      </div>
    </div>
  );
}
