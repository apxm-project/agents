import { useCallback, useState } from "react";
import { useAppStore } from "@/store/app-store";
import { executeWorkflow } from "@/api/compile";

export function ExecuteDialog() {
  const open = useAppStore((s) => s.executeDialogOpen);
  const setOpen = useAppStore((s) => s.setExecuteDialogOpen);
  const selectedWorkflow = useAppStore((s) => s.selectedWorkflow);
  const graphPath = useAppStore((s) => s.graphPath);
  const startInlineRun = useAppStore((s) => s.startInlineRun);

  const [params, setParams] = useState<Record<string, string>>({});
  const [executing, setExecuting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const workflowParams = selectedWorkflow?.parameters ?? [];
  const workflowPath = selectedWorkflow?.path ?? graphPath;

  const onClose = useCallback(() => {
    setOpen(false);
    setParams({});
    setError(null);
  }, [setOpen]);

  const onExecute = useCallback(async () => {
    if (!workflowPath) return;
    setExecuting(true);
    setError(null);
    try {
      const result = await executeWorkflow(workflowPath, params);
      onClose();
      startInlineRun(result.session_path);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setExecuting(false);
    }
  }, [workflowPath, params, onClose, startInlineRun]);

  if (!open || !workflowPath) return null;

  const name = selectedWorkflow?.name ?? graphPath?.split("/").pop() ?? "Workflow";

  return (
    <div className="execute-overlay" onClick={onClose}>
      <div className="execute-dialog" onClick={(e) => e.stopPropagation()}>
        <div className="execute-dialog__header">
          <h3>Execute Workflow</h3>
          <button type="button" className="execute-dialog__close" onClick={onClose}>
            &times;
          </button>
        </div>

        <div className="execute-dialog__info">
          <span className="execute-dialog__name">{name}</span>
          <span className="execute-dialog__path">{selectedWorkflow?.relative_path ?? graphPath}</span>
        </div>

        {error && (
          <div className="execute-dialog__error">
            <strong>Execution failed</strong>
            <p>{error}</p>
          </div>
        )}

        {workflowParams.length > 0 && (
          <div className="execute-dialog__params">
            <span className="execute-dialog__section-title">Parameters</span>
            {workflowParams.map((p) => (
              <div key={p.name} className="execute-dialog__param">
                <label className="execute-dialog__param-label">
                  {p.name}
                  <span className="execute-dialog__param-type">{p.type_name}</span>
                </label>
                <input
                  type="text"
                  className="execute-dialog__param-input"
                  placeholder={`Enter ${p.name}...`}
                  value={params[p.name] ?? ""}
                  onChange={(e) =>
                    setParams((prev) => ({ ...prev, [p.name]: e.target.value }))
                  }
                />
              </div>
            ))}
          </div>
        )}

        {workflowParams.length === 0 && !error && (
          <p className="execute-dialog__no-params">
            This workflow has no parameters. It will execute immediately.
          </p>
        )}

        <div className="execute-dialog__actions">
          <button type="button" className="ghost-button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="execute-dialog__run-btn"
            onClick={onExecute}
            disabled={executing}
          >
            {executing ? "Starting..." : error ? "Retry" : "Execute"}
          </button>
        </div>
      </div>
    </div>
  );
}
