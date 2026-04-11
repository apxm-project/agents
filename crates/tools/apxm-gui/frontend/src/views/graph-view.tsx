import { useCallback, useEffect, useRef, useState } from "react";
import { GraphCanvas } from "@/components/graph/graph-canvas";
import { WorkflowSidebar } from "@/components/graph/workflow-sidebar";
import { SourceView } from "@/components/graph/source-view";
import { NodeInspector } from "@/components/inspector/node-inspector";
import { NodeEditor } from "@/components/inspector/node-editor";
import { ExecuteDialog } from "@/components/studio/execute-dialog";
import { CompilePanel } from "@/components/studio/compile-panel";
import { OpPalette } from "@/components/studio/op-palette";
import { NewWorkflowDialog } from "@/components/studio/new-workflow-dialog";
import { StudioLivePanel } from "@/components/studio/studio-live-panel";
import { useAppStore } from "@/store/app-store";
import { ViewMode, LoadKey } from "@/lib/constants";
import { compileWorkflow, validateWorkflow, explainWorkflow } from "@/api/compile";
import type { CompileOptions } from "@/api/compile";
import { saveGraph } from "@/api/save";

const MIN_WIDTH = 160;
const MAX_WIDTH = 500;
const DEFAULT_WIDTH = 260;

const OPT_TARGETS = ["balanced", "latency", "cost", "tokens", "parallelism"] as const;

export function GraphView() {
  const inspectorOpen = useAppStore((s) => s.inspectorOpen);
  const viewMode = useAppStore((s) => s.viewMode);
  const setViewMode = useAppStore((s) => s.setViewMode);
  const graphData = useAppStore((s) => s.graphData);
  const graphPath = useAppStore((s) => s.graphPath);
  const selectedWorkflow = useAppStore((s) => s.selectedWorkflow);
  const compileResult = useAppStore((s) => s.compileResult);
  const setCompileResult = useAppStore((s) => s.setCompileResult);
  const setExecuteDialogOpen = useAppStore((s) => s.setExecuteDialogOpen);
  const setError = useAppStore((s) => s.setError);
  const editMode = useAppStore((s) => s.editMode);
  const setEditMode = useAppStore((s) => s.setEditMode);
  const dirty = useAppStore((s) => s.dirty);
  const setDirty = useAppStore((s) => s.setDirty);
  const inlineRun = useAppStore((s) => s.inlineRun);
  const liveActive = useAppStore((s) => s.liveActive);
  const finishInlineRun = useAppStore((s) => s.finishInlineRun);
  const liveManifest = useAppStore((s) => s.liveManifest);
  const graphAnalysis = useAppStore((s) => s.graphAnalysis);

  const [collapsed, setCollapsed] = useState(false);
  const [width, setWidth] = useState(DEFAULT_WIDTH);
  const [isDragging, setIsDragging] = useState(false);
  const [compiling, setCompiling] = useState(false);
  const [saving, setSaving] = useState(false);
  const [newDialogOpen, setNewDialogOpen] = useState(false);
  const [validating, setValidating] = useState(false);
  const [validateResult, setValidateResult] = useState<{ valid: boolean; stderr: string } | null>(null);
  const [explaining, setExplaining] = useState(false);
  const [explanation, setExplanation] = useState<string | null>(null);
  const [showCompileOpts, setShowCompileOpts] = useState(false);
  const [optLevel, setOptLevel] = useState(1);
  const [optTarget, setOptTarget] = useState("balanced");
  const [noCseLlm, setNoCseLlm] = useState(false);
  const [emitDiag, setEmitDiag] = useState(false);
  const startX = useRef(0);
  const startW = useRef(0);

  useEffect(() => {
    if (!inlineRun) return;
    const status = liveManifest?.status as string | undefined;
    if (status === "completed" || status === "failed") {
      finishInlineRun(status);
    }
  }, [inlineRun, liveManifest, finishInlineRun]);

  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (collapsed) return;
      e.preventDefault();
      startX.current = e.clientX;
      startW.current = width;
      setIsDragging(true);
    },
    [collapsed, width],
  );

  useEffect(() => {
    if (!isDragging) return;
    function onMouseMove(e: MouseEvent) {
      const delta = e.clientX - startX.current;
      setWidth(Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, startW.current + delta)));
    }
    function onMouseUp() { setIsDragging(false); }
    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    return () => {
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
  }, [isDragging]);

  const compilePath = graphPath ?? selectedWorkflow?.path;

  const onCompile = useCallback(async () => {
    if (!compilePath || compiling) return;
    setCompiling(true);
    try {
      const opts: CompileOptions = {
        opt_level: optLevel,
        target: optTarget !== "balanced" ? optTarget : undefined,
        emit_diagnostics: emitDiag,
        no_cse_llm: noCseLlm,
      };
      const result = await compileWorkflow(compilePath, opts);
      setCompileResult(result);
      setError(LoadKey.COMPILE, null);
    } catch (e: unknown) {
      setError(LoadKey.COMPILE, e instanceof Error ? e.message : String(e));
    } finally {
      setCompiling(false);
    }
  }, [compilePath, compiling, setCompileResult, setError, optLevel, optTarget, emitDiag, noCseLlm]);

  const onValidate = useCallback(async () => {
    if (!compilePath || validating) return;
    setValidating(true);
    setValidateResult(null);
    try {
      const res = await validateWorkflow(compilePath);
      setValidateResult({ valid: res.valid, stderr: res.stderr });
    } catch (e: unknown) {
      setValidateResult({ valid: false, stderr: e instanceof Error ? e.message : String(e) });
    }
    setValidating(false);
  }, [compilePath, validating]);

  const onExplain = useCallback(async () => {
    if (!compilePath || explaining) return;
    setExplaining(true);
    try {
      const res = await explainWorkflow(compilePath);
      setExplanation(res.explanation);
    } catch (e: unknown) {
      setExplanation(`Error: ${e instanceof Error ? e.message : String(e)}`);
    }
    setExplaining(false);
  }, [compilePath, explaining]);

  const onSave = useCallback(async () => {
    if (!graphData || saving) return;
    setSaving(true);
    try {
      await saveGraph(graphData, graphPath ?? undefined);
      setDirty(false);
      setError(LoadKey.SAVE, null);
    } catch (e: unknown) {
      setError(LoadKey.SAVE, e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }, [graphData, graphPath, saving, setDirty, setError]);

  const showGraph = viewMode === ViewMode.GRAPH && graphData;
  const showLivePanel = inlineRun && (liveActive || !liveActive);

  return (
    <div className="explorer-layout">
      <div
        className={`explorer-layout__sidebar${collapsed ? " explorer-layout__sidebar--collapsed" : ""}`}
        style={collapsed ? { flex: "0 0 0px", overflow: "hidden", minWidth: 0 } : { flex: `0 0 ${width}px` }}
      >
        {!collapsed && (
          editMode ? <OpPalette /> : <WorkflowSidebar />
        )}
      </div>
      {!collapsed && (
        <div className="explorer-layout__resize-handle" onMouseDown={onMouseDown} />
      )}
      <button
        type="button"
        className="explorer-layout__collapse-btn"
        style={{ left: collapsed ? 4 : width + 8 }}
        onClick={() => setCollapsed((c) => !c)}
        title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
      >
        {collapsed ? "\u25B8" : "\u25C2"}
      </button>
      <div className="explorer-layout__main">
        {showGraph ? (
          <>
            <div className="explorer-layout__view-toggle">
              <div className="explorer-layout__view-toggle-left">
                {!editMode && (
                  <button
                    type="button"
                    className="ghost-button ghost-button--sm"
                    onClick={() => setViewMode(ViewMode.SOURCE)}
                  >
                    ← Source
                  </button>
                )}
                <button
                  type="button"
                  className={`ghost-button ghost-button--sm${editMode ? " ghost-button--active" : ""}`}
                  onClick={() => setEditMode(!editMode)}
                >
                  {editMode ? "Editing" : "Edit"}
                </button>
                {editMode && dirty && (
                  <button
                    type="button"
                    className={`ghost-button ghost-button--sm ghost-button--save${saving ? " ghost-button--loading" : ""}`}
                    onClick={onSave}
                    disabled={saving}
                  >
                    {saving ? "Saving..." : "Save"}
                  </button>
                )}
                {/* Analysis badges */}
                {graphAnalysis && (
                  <div className="studio-badges">
                    <span className="studio-badge" title="Maximum parallelism">
                      ⫴ {graphAnalysis.max_parallelism}
                    </span>
                    <span className="studio-badge" title="Critical path length">
                      ⟶ {graphAnalysis.critical_path_length}
                    </span>
                    {graphAnalysis.max_parallelism > 1 && (
                      <span className="studio-badge studio-badge--accent" title="Estimated speedup">
                        ~{(graphAnalysis.total_nodes / graphAnalysis.critical_path_length).toFixed(1)}× speedup
                      </span>
                    )}
                  </div>
                )}
              </div>
              <div className="explorer-layout__view-toggle-actions">
                <button
                  type="button"
                  className="ghost-button ghost-button--sm"
                  onClick={() => setNewDialogOpen(true)}
                >
                  + New
                </button>
                {compilePath && (
                  <>
                    <button
                      type="button"
                      className={`ghost-button ghost-button--sm${validating ? " ghost-button--loading" : ""}`}
                      onClick={onValidate}
                      disabled={validating}
                      title="Validate workflow"
                    >
                      {validating ? "..." : "Validate"}
                    </button>
                    <button
                      type="button"
                      className={`ghost-button ghost-button--sm${explaining ? " ghost-button--loading" : ""}`}
                      onClick={onExplain}
                      disabled={explaining}
                      title="Explain workflow"
                    >
                      {explaining ? "..." : "Explain"}
                    </button>
                    <div className="compile-dropdown">
                      <button
                        type="button"
                        className={`ghost-button ghost-button--sm${compiling ? " ghost-button--loading" : ""}`}
                        onClick={onCompile}
                        disabled={compiling}
                      >
                        {compiling ? "Compiling..." : "Compile"}
                      </button>
                      <button
                        type="button"
                        className="ghost-button ghost-button--sm compile-dropdown__toggle"
                        onClick={() => setShowCompileOpts(!showCompileOpts)}
                        title="Compile options"
                      >
                        ▾
                      </button>
                      {showCompileOpts && (
                        <div className="compile-dropdown__menu">
                          <label className="compile-dropdown__field">
                            <span>Level</span>
                            <input
                              type="range" min={0} max={3} value={optLevel}
                              onChange={(e) => setOptLevel(Number(e.target.value))}
                            />
                            <span className="compile-dropdown__val">O{optLevel}</span>
                          </label>
                          <label className="compile-dropdown__field">
                            <span>Target</span>
                            <select value={optTarget} onChange={(e) => setOptTarget(e.target.value)}>
                              {OPT_TARGETS.map((t) => (
                                <option key={t} value={t}>{t}</option>
                              ))}
                            </select>
                          </label>
                          <label className="compile-dropdown__field">
                            <input type="checkbox" checked={noCseLlm} onChange={(e) => setNoCseLlm(e.target.checked)} />
                            <span>No CSE for LLM ops</span>
                          </label>
                          <label className="compile-dropdown__field">
                            <input type="checkbox" checked={emitDiag} onChange={(e) => setEmitDiag(e.target.checked)} />
                            <span>Emit diagnostics</span>
                          </label>
                        </div>
                      )}
                    </div>
                  </>
                )}
                <button
                  type="button"
                  className="ghost-button ghost-button--sm ghost-button--execute"
                  onClick={() => setExecuteDialogOpen(true)}
                >
                  Execute
                </button>
              </div>
            </div>

            {/* Validate result banner */}
            {validateResult && (
              <div className={`validate-banner validate-banner--${validateResult.valid ? "valid" : "invalid"}`}>
                <span>{validateResult.valid ? "✓ Valid" : "✗ Invalid"}</span>
                {validateResult.stderr && <span className="validate-banner__msg">{validateResult.stderr.slice(0, 200)}</span>}
                <button type="button" className="validate-banner__close" onClick={() => setValidateResult(null)}>×</button>
              </div>
            )}

            {/* Explain modal */}
            {explanation !== null && (
              <div className="explain-overlay">
                <div className="explain-modal">
                  <div className="explain-modal__header">
                    <h3>Workflow Explanation</h3>
                    <button type="button" className="explain-modal__close" onClick={() => setExplanation(null)}>×</button>
                  </div>
                  <pre className="explain-modal__body">{explanation}</pre>
                </div>
              </div>
            )}

            {compileResult && <CompilePanel />}
            {showLivePanel && <StudioLivePanel />}
            <GraphCanvas />
          </>
        ) : (
          <SourceView />
        )}
      </div>
      {showGraph && inspectorOpen && (
        <div className="explorer-layout__inspector">
          {editMode ? <NodeEditor /> : <NodeInspector />}
        </div>
      )}
      <ExecuteDialog />
      <NewWorkflowDialog open={newDialogOpen} onClose={() => setNewDialogOpen(false)} />
    </div>
  );
}
