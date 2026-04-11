import { useState, useMemo, useEffect } from "react";
import { useAppStore } from "@/store/app-store";
import { getCategoryStyle } from "@/lib/category-styles";
import { Section, CodeBlock } from "@/components/inspector/common";
import { fetchAgents } from "@/api/agents";
import type { AgentProfile } from "@/types/api";
import type { OpSpec } from "@/types/ops";

type RefTab = "overview" | "ops" | "compiler" | "runtime" | "agents";

export function ReferenceView() {
  const [refTab, setRefTab] = useState<RefTab>("overview");

  return (
    <div className="view-panel">
      <div className="view-panel__header">
        <h2>APXM Reference</h2>
        <div className="ref-tab-bar">
          {(["overview", "ops", "compiler", "runtime", "agents"] as RefTab[]).map((t) => (
            <button
              key={t}
              type="button"
              className={`ref-tab${refTab === t ? " ref-tab--active" : ""}`}
              onClick={() => setRefTab(t)}
            >
              {t === "overview" ? "Overview" : t === "ops" ? "AIS Operations" : t === "compiler" ? "Compiler" : t === "runtime" ? "Runtime" : "Agents"}
            </button>
          ))}
        </div>
      </div>

      <div className="ref-body">
        {refTab === "overview" && <OverviewTab />}
        {refTab === "ops" && <OpsTab />}
        {refTab === "compiler" && <CompilerTab />}
        {refTab === "runtime" && <RuntimeTab />}
        {refTab === "agents" && <AgentsTab />}
      </div>
    </div>
  );
}

// ─── Overview Tab ──────────────────────────────────────────────────────────

function OverviewTab() {
  const ops = useAppStore((s) => s.ops);
  const passes = useAppStore((s) => s.passes);
  const health = useAppStore((s) => s.health);

  const categories = useMemo(() => {
    const set = new Set(ops.map((o) => o.category));
    return set.size;
  }, [ops]);

  return (
    <div className="ref-overview">
      <div className="ref-overview__hero">
        <h3>Agent Programming eXecution Model</h3>
        <p className="ref-overview__subtitle">
          Full toolchain for building, compiling, and executing autonomous agent workflows.
        </p>
      </div>

      {/* Stats bar */}
      <div className="ref-overview__stats">
        <div className="ref-stat">
          <span className="ref-stat__value">{ops.length}</span>
          <span className="ref-stat__label">Operations</span>
        </div>
        <div className="ref-stat">
          <span className="ref-stat__value">{categories}</span>
          <span className="ref-stat__label">Categories</span>
        </div>
        <div className="ref-stat">
          <span className="ref-stat__value">{passes.length}</span>
          <span className="ref-stat__label">Passes</span>
        </div>
        {health && (
          <>
            <div className="ref-stat">
              <span className="ref-stat__value">{health.backends.length}</span>
              <span className="ref-stat__label">Backends</span>
            </div>
            <div className="ref-stat">
              <span className="ref-stat__value">{health.total_models}</span>
              <span className="ref-stat__label">Models</span>
            </div>
          </>
        )}
      </div>

      {/* Architecture */}
      <div className="ref-overview__section">
        <h4>Architecture</h4>
        <div className="ref-arch">
          <div className="ref-arch__layer ref-arch__layer--tools">
            <span className="ref-arch__label">Tools</span>
            <div className="ref-arch__items">
              <span>CLI</span><span>Studio (GUI)</span><span>HTTP Server</span>
            </div>
          </div>
          <div className="ref-arch__arrow">↕</div>
          <div className="ref-arch__layer ref-arch__layer--orchestration">
            <span className="ref-arch__label">Orchestration</span>
            <div className="ref-arch__items">
              <span>Driver</span><span>ACP</span><span>Artifact</span>
            </div>
          </div>
          <div className="ref-arch__arrow">↕</div>
          <div className="ref-arch__row">
            <div className="ref-arch__layer ref-arch__layer--compiler">
              <span className="ref-arch__label">Compiler</span>
              <div className="ref-arch__items">
                <span>14 passes</span><span>MLIR codegen</span><span>.apxmobj</span>
              </div>
            </div>
            <div className="ref-arch__layer ref-arch__layer--runtime">
              <span className="ref-arch__label">Runtime</span>
              <div className="ref-arch__items">
                <span>Scheduler</span><span>Handlers</span><span>Backends</span>
              </div>
            </div>
          </div>
          <div className="ref-arch__arrow">↕</div>
          <div className="ref-arch__layer ref-arch__layer--core">
            <span className="ref-arch__label">Core (Single Source of Truth)</span>
            <div className="ref-arch__items">
              <span>{ops.length} AIS Operations</span><span>Attributes</span><span>Events</span><span>Types</span>
            </div>
          </div>
        </div>
      </div>

      {/* Key concepts */}
      <div className="ref-overview__section">
        <h4>Key Concepts</h4>
        <div className="ref-concepts">
          {[
            { title: "ApxmGraph", desc: "The canonical IR: a directed acyclic graph of AIS operations with typed edges (Data, Control, Memory)." },
            { title: "AIS Operations", desc: "The Agent Instruction Set — 41 operations spanning reasoning, memory, tools, control flow, synchronization, and coordination." },
            { title: "Compilation", desc: "Graph → AIS MLIR → 14 optimization passes → .apxmobj binary artifact with BLAKE3 integrity." },
            { title: ".apxmobj", desc: "Compiled artifact format: magic bytes, versioned header, bincode payload with multi-DAG support." },
            { title: "Sessions", desc: "Execution traces stored as manifest.json + trace.ndjson + per-node directories with prompts, responses, and outputs." },
            { title: "Dataflow Scheduling", desc: "Token-based parallel scheduler with concurrency limits, cost budgets, latency overrides, and automatic sequential fallback." },
          ].map((c) => (
            <div key={c.title} className="ref-concept-card">
              <strong>{c.title}</strong>
              <p>{c.desc}</p>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

// ─── AIS Operations Tab ────────────────────────────────────────────────────

const CATEGORY_DESCRIPTIONS: Record<string, string> = {
  reasoning: "Interact with LLMs: ASK for simple queries, THINK for multi-step reasoning, REASON for deep analysis, PLAN/REFLECT/VERIFY for structured thinking.",
  memory: "Read and write agent memory: QMEM queries, UMEM updates, UPDATE_GOAL modifies objectives.",
  tools: "Invoke external tools and code: INV_TOOL calls registered tools, EXC executes code, PRINT outputs text.",
  control_flow: "Control execution flow: branching, loops, jumps, function calls, guards, and returns.",
  synchronization: "Coordinate parallel execution: MERGE joins branches, FENCE enforces ordering, WAIT_ALL waits for all inputs.",
  error_handling: "Handle errors: TRY_CATCH wraps operations, ERR raises errors with recovery.",
  communication: "Inter-agent messaging: COMMUNICATE sends messages, CLAIM/PAUSE manage agent state.",
  coordination: "Multi-agent coordination: DELEGATE assigns tasks, NEGOTIATE resolves conflicts, SPAWN creates agents.",
  identity: "No-ops and identity: NOP does nothing, IDENTITY passes values through unchanged.",
  internal: "Compiler-internal operations not used in user-facing graphs.",
  metadata: "Graph metadata: AGENT defines the agent profile for a function.",
};

const LATENCY_COLORS: Record<string, string> = {
  none: "var(--ink-faint)",
  low: "#27ae60",
  medium: "#f39c12",
  high: "#e74c3c",
};

function OpsTab() {
  const ops = useAppStore((s) => s.ops);
  const [search, setSearch] = useState("");
  const [selectedOp, setSelectedOp] = useState<string | null>(null);

  const filtered = useMemo(() => {
    if (!search) return ops;
    const q = search.toLowerCase();
    return ops.filter(
      (op) => op.name.toLowerCase().includes(q) || op.category.toLowerCase().includes(q) || op.description.toLowerCase().includes(q),
    );
  }, [ops, search]);

  const grouped = useMemo(() => {
    const groups: Record<string, OpSpec[]> = {};
    for (const op of filtered) {
      const cat = op.category;
      if (!groups[cat]) groups[cat] = [];
      groups[cat].push(op);
    }
    return Object.entries(groups).sort(([a], [b]) => a.localeCompare(b));
  }, [filtered]);

  const detail = useMemo(() => ops.find((op) => op.name === selectedOp) ?? null, [ops, selectedOp]);

  return (
    <div className="ops-layout">
      <div className="ops-table">
        <div className="ops-table__header">
          <input
            type="text"
            className="search-input"
            placeholder="Search operations..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
          <span className="ops-table__count">{filtered.length} ops</span>
        </div>
        <div className="ops-table__body">
          {grouped.map(([category, categoryOps]) => (
            <div key={category} className="ops-category-group">
              <div className="ops-category-group__header">
                <span className="ops-category-group__label" style={{ background: getCategoryStyle(category).bg, color: getCategoryStyle(category).color }}>
                  {category}
                </span>
                <span className="ops-category-group__count">{categoryOps.length}</span>
              </div>
              {CATEGORY_DESCRIPTIONS[category] && (
                <p className="ops-category-group__desc">{CATEGORY_DESCRIPTIONS[category]}</p>
              )}
              {categoryOps.map((op) => (
                <button
                  key={op.name}
                  type="button"
                  className={`ops-table__row${selectedOp === op.name ? " ops-table__row--selected" : ""}`}
                  onClick={() => setSelectedOp(op.name)}
                >
                  <span className="ops-table__name">{op.name}</span>
                  <span className="ops-table__latency" style={{ color: LATENCY_COLORS[op.latency] ?? "var(--ink-faint)" }}>
                    ● {op.latency}
                  </span>
                  <div className="ops-table__flags">
                    {op.needs_submission && <span className="ops-flag" title="Needs async submission">async</span>}
                    {op.produces_output && <span className="ops-flag" title="Produces output">out</span>}
                  </div>
                </button>
              ))}
            </div>
          ))}
        </div>
      </div>
      <div className="ops-detail">
        {detail ? (
          <div className="ops-detail__content">
            <Section title={detail.name} subtitle={`${detail.category} · ${detail.latency} latency`}>
              <p className="inspector__description">{detail.description}</p>
              {detail.long_description ? <p className="inspector__description">{detail.long_description}</p> : null}
            </Section>
            {detail.fields.length > 0 ? (
              <Section title="Fields">
                <dl className="definition-grid">
                  {detail.fields.map((f) => (
                    <div key={f.name}>
                      <dt>
                        {f.name}{f.required ? " *" : ""}
                        {f.ref_type && <span className="ops-ref-badge">{f.ref_type}</span>}
                      </dt>
                      <dd>{f.description}</dd>
                    </div>
                  ))}
                </dl>
              </Section>
            ) : null}
            {detail.example_json ? (
              <Section title="Example">
                <CodeBlock>{detail.example_json}</CodeBlock>
              </Section>
            ) : null}
          </div>
        ) : (
          <div className="view-panel__empty">Select an operation to see details.</div>
        )}
      </div>
    </div>
  );
}

// ─── Compiler Tab ──────────────────────────────────────────────────────────

const PASS_CATEGORY_COLORS: Record<string, string> = {
  Transform: "#3498db",
  Optimization: "#27ae60",
  Analysis: "#f39c12",
  Lowering: "#9b59b6",
};

const OPT_LEVEL_PASSES: Record<number, string[]> = {
  0: [],
  1: ["normalize", "build-prompt", "dspy-optimize", "unconsumed-value-warning", "scheduling", "fuse-ask-ops", "assign-priority", "canonicalizer", "cse", "symbol-dce"],
  2: ["normalize", "build-prompt", "dspy-optimize", "unconsumed-value-warning", "prompt-canonicalization", "template-specialization", "schema-narrowing", "scheduling", "fuse-ask-ops", "condense-ops", "dead-context-elimination", "assign-priority", "canonicalizer", "cse", "symbol-dce"],
  3: ["normalize", "build-prompt", "dspy-optimize", "unconsumed-value-warning", "prompt-canonicalization", "template-specialization", "schema-narrowing", "scheduling", "fuse-ask-ops", "condense-ops", "dead-context-elimination", "assign-priority", "canonicalizer", "cse", "symbol-dce", "(convergence ×10)"],
};

function CompilerTab() {
  const passes = useAppStore((s) => s.passes);
  const [selectedLevel, setSelectedLevel] = useState(2);

  const levelPasses = OPT_LEVEL_PASSES[selectedLevel] ?? [];

  return (
    <div className="ref-compiler">
      {/* Pipeline */}
      <div className="ref-compiler__section">
        <h4>Compilation Pipeline</h4>
        <p className="ref-compiler__desc">
          APXM compiles agent workflow graphs through a series of optimization passes, transforming the graph IR into an efficient executable artifact.
        </p>
        <div className="ref-compiler__flow">
          <span className="ref-compiler__flow-step">.py / .air</span>
          <span className="ref-compiler__flow-arrow">→</span>
          <span className="ref-compiler__flow-step">ApxmGraph IR</span>
          <span className="ref-compiler__flow-arrow">→</span>
          <span className="ref-compiler__flow-step">AIS MLIR</span>
          <span className="ref-compiler__flow-arrow">→</span>
          <span className="ref-compiler__flow-step">{passes.length} passes</span>
          <span className="ref-compiler__flow-arrow">→</span>
          <span className="ref-compiler__flow-step">.apxmobj</span>
        </div>
      </div>

      {/* Passes */}
      <div className="ref-compiler__section">
        <h4>Pass Pipeline ({passes.length} passes)</h4>
        <div className="ref-compiler__passes">
          {passes.map((p, i) => {
            const color = PASS_CATEGORY_COLORS[p.category] ?? "var(--ink-faint)";
            return (
              <div key={p.name} className="ref-pass-card">
                <div className="ref-pass-card__header">
                  <span className="ref-pass-card__index">{i + 1}</span>
                  <span className="ref-pass-card__name">{p.name}</span>
                  <span className="ref-pass-card__category" style={{ background: color, color: "#fff" }}>{p.category}</span>
                </div>
                <p className="ref-pass-card__summary">{p.summary}</p>
                {p.description !== p.summary && (
                  <p className="ref-pass-card__description">{p.description}</p>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {/* Optimization levels */}
      <div className="ref-compiler__section">
        <h4>Optimization Levels</h4>
        <div className="ref-compiler__levels">
          {[0, 1, 2, 3].map((lvl) => (
            <button
              key={lvl}
              type="button"
              className={`ref-level-btn${selectedLevel === lvl ? " ref-level-btn--active" : ""}`}
              onClick={() => setSelectedLevel(lvl)}
            >
              O{lvl}
            </button>
          ))}
        </div>
        <div className="ref-compiler__level-detail">
          <p className="ref-compiler__level-desc">
            {selectedLevel === 0 && "No optimization passes. Raw graph is emitted directly."}
            {selectedLevel === 1 && "Standard optimization: normalize, prompt building, scheduling, fusion, and cleanup."}
            {selectedLevel === 2 && "Aggressive optimization: adds prompt canonicalization, template specialization, schema narrowing, dead context elimination."}
            {selectedLevel === 3 && "Maximum optimization: O2 passes with convergence loop (up to 10 iterations) for deep optimization."}
          </p>
          <div className="ref-compiler__level-passes">
            {levelPasses.map((p, i) => (
              <span key={i} className="ref-compiler__level-pass">{p}</span>
            ))}
            {levelPasses.length === 0 && <span className="ref-compiler__level-pass ref-compiler__level-pass--none">None</span>}
          </div>
        </div>
      </div>

      {/* Optimization targets */}
      <div className="ref-compiler__section">
        <h4>Optimization Targets</h4>
        <div className="ref-compiler__targets">
          {[
            { name: "Balanced", desc: "Default: equal weight to latency, cost, and throughput." },
            { name: "Latency", desc: "Minimize end-to-end latency with more fusion and parallel scheduling." },
            { name: "Cost", desc: "Minimize LLM API cost via more CSE and model substitution." },
            { name: "Tokens", desc: "Minimize token usage through context compression and dead context elimination." },
            { name: "Parallelism", desc: "Maximize parallel execution with aggressive scheduling." },
          ].map((t) => (
            <div key={t.name} className="ref-target-card">
              <strong>{t.name}</strong>
              <p>{t.desc}</p>
            </div>
          ))}
        </div>
      </div>

      {/* Artifact format */}
      <div className="ref-compiler__section">
        <h4>Artifact Format (.apxmobj)</h4>
        <div className="ref-artifact-spec">
          <div className="ref-artifact-spec__row"><span>Magic</span><code>b"APXM"</code></div>
          <div className="ref-artifact-spec__row"><span>Version</span><code>1</code></div>
          <div className="ref-artifact-spec__row"><span>Header</span><code>52 bytes (magic, version, payload length, BLAKE3 hash, flags)</code></div>
          <div className="ref-artifact-spec__row"><span>Payload</span><code>Bincode: ArtifactMetadata + Vec&lt;WireDag&gt; + optional sections</code></div>
          <div className="ref-artifact-spec__row"><span>Integrity</span><code>BLAKE3 hash verified on load</code></div>
          <div className="ref-artifact-spec__row"><span>Multi-DAG</span><code>Entry DAG (is_entry) + non-entry flows</code></div>
        </div>
      </div>
    </div>
  );
}

// ─── Runtime Tab ───────────────────────────────────────────────────────────

const HANDLER_GROUPS: { group: string; ops: string[] }[] = [
  { group: "LLM", ops: ["Ask", "Think", "Reason"] },
  { group: "Planning", ops: ["Plan", "Reflect", "Verify"] },
  { group: "Memory", ops: ["QueryMemory", "UpdateMemory"] },
  { group: "Tools", ops: ["InvokeTool", "ExecuteCode", "PrintOutput"] },
  { group: "Control Flow", ops: ["Jump", "BranchOnValue", "LoopStart", "LoopEnd", "Return", "Switch", "FlowCall"] },
  { group: "Synchronization", ops: ["Merge", "Fence", "WaitAll", "Checkpoint"] },
  { group: "Error Handling", ops: ["TryCatch", "HandleError"] },
  { group: "Communication", ops: ["Communicate", "Claim", "Pause"] },
  { group: "Coordination", ops: ["UpdateGoal", "Guard", "Resume", "Delegate", "Negotiate"] },
  { group: "Self-Organization", ops: ["SpawnAgent", "SpawnTeam", "RegisterCapability", "Autonomous"] },
  { group: "Identity", ops: ["Nop", "Identity"] },
];

const SESSION_FILES = [
  { file: "manifest.json", desc: "Session metadata: graph name, status, timestamp, duration, node count" },
  { file: "trace.ndjson", desc: "Global execution trace — newline-delimited JSON of ApxmEvent records" },
  { file: "results.json", desc: "Final execution output and return values" },
  { file: "metrics.json", desc: "Performance metrics: token usage, latencies, cost estimates" },
  { file: "node_statuses.json", desc: "Per-node completion status map" },
  { file: "nodes/", desc: "Per-node directories with node.json, output.json, prompt.txt, response.txt, and trace.ndjson" },
];

const EVENT_KINDS = [
  { kind: "operation_start", desc: "Fired when a node begins execution" },
  { kind: "operation_complete", desc: "Fired when a node finishes successfully" },
  { kind: "operation_error", desc: "Fired when a node encounters an error" },
  { kind: "token", desc: "Individual LLM output token during streaming" },
  { kind: "session_start", desc: "Session initialized with graph metadata" },
  { kind: "session_complete", desc: "All nodes finished, session closing" },
  { kind: "session_error", desc: "Unrecoverable session-level error" },
];

function RuntimeTab() {
  return (
    <div className="ref-runtime">
      {/* Execution model */}
      <div className="ref-runtime__section">
        <h4>Execution Model</h4>
        <p>
          APXM uses a <strong>dataflow scheduler</strong> that executes operations as a directed acyclic graph.
          When a DAG has more than one node, the scheduler uses <strong>token-based parallelism</strong> — a node
          becomes ready when all its input dependencies are satisfied.
        </p>
        <div className="ref-runtime__features">
          <div className="ref-feature-card">
            <strong>Concurrency Limits</strong>
            <p>Configurable maximum concurrent LLM calls to avoid rate limits.</p>
          </div>
          <div className="ref-feature-card">
            <strong>Cost Budgets</strong>
            <p>Enforce per-session cost limits; scheduler halts if budget exceeded.</p>
          </div>
          <div className="ref-feature-card">
            <strong>Latency Overrides</strong>
            <p>Per-operation latency tier overrides for scheduling priority.</p>
          </div>
          <div className="ref-feature-card">
            <strong>Sequential Fallback</strong>
            <p>Automatic fallback to sequential execution if parallel scheduling fails.</p>
          </div>
        </div>
      </div>

      {/* Operation handlers */}
      <div className="ref-runtime__section">
        <h4>Operation Handlers</h4>
        <p>Each operation is dispatched to a specialized handler module at runtime.</p>
        <div className="ref-runtime__handlers">
          {HANDLER_GROUPS.map((g) => (
            <div key={g.group} className="ref-handler-group">
              <span className="ref-handler-group__label">{g.group}</span>
              <div className="ref-handler-group__ops">
                {g.ops.map((op) => (
                  <span key={op} className="ref-handler-group__op">{op}</span>
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Session model */}
      <div className="ref-runtime__section">
        <h4>Session Model</h4>
        <p>Each execution creates a session directory under <code>~/.apxm/sessions/</code> containing:</p>
        <div className="ref-runtime__session-files">
          {SESSION_FILES.map((f) => (
            <div key={f.file} className="ref-session-file">
              <code>{f.file}</code>
              <span>{f.desc}</span>
            </div>
          ))}
        </div>
      </div>

      {/* Trace events */}
      <div className="ref-runtime__section">
        <h4>Trace Event Kinds</h4>
        <div className="ref-runtime__events">
          {EVENT_KINDS.map((e) => (
            <div key={e.kind} className="ref-event-kind">
              <code>{e.kind}</code>
              <span>{e.desc}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

// ─── Agents Tab ────────────────────────────────────────────────────────────

function AgentsTab() {
  const [agents, setAgents] = useState<AgentProfile[]>([]);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState("");

  useEffect(() => {
    fetchAgents()
      .then(setAgents)
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  const filtered = useMemo(() => {
    if (!filter) return agents;
    const q = filter.toLowerCase();
    return agents.filter(
      (a) => a.name.includes(q) || a.description.toLowerCase().includes(q) || a.category.includes(q),
    );
  }, [agents, filter]);

  const grouped = useMemo(() => {
    const groups: Record<string, AgentProfile[]> = {};
    for (const a of filtered) {
      const cat = a.category;
      if (!groups[cat]) groups[cat] = [];
      groups[cat].push(a);
    }
    return Object.entries(groups).sort(([a], [b]) => a.localeCompare(b));
  }, [filtered]);

  if (loading) {
    return <div className="view-panel__empty">Loading agents...</div>;
  }

  return (
    <div className="ref-agents">
      <div className="ref-agents__header">
        <input
          type="text"
          className="search-input"
          placeholder="Search agents..."
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <span className="ref-agents__count">{filtered.length} agents</span>
      </div>
      <p className="ref-agents__desc">
        Builtin agent profiles available for use in workflows with <code>SpawnAgent</code> and <code>SpawnTeam</code> operations.
      </p>
      {grouped.map(([category, categoryAgents]) => (
        <div key={category} className="ref-agents__group">
          <span className="ref-agents__group-label">{category}</span>
          <div className="ref-agents__grid">
            {categoryAgents.map((agent) => (
              <div key={agent.name} className="ref-agent-card">
                <div className="ref-agent-card__header">
                  <span className="ref-agent-card__name">{agent.name}</span>
                  <span className="ref-agent-card__category">{agent.category}</span>
                </div>
                <p className="ref-agent-card__desc">{agent.description}</p>
                <div className="ref-agent-card__skills">
                  {agent.skills.map((s) => (
                    <span key={s} className="ref-agent-card__skill">{s}</span>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </div>
      ))}
      {filtered.length === 0 && (
        <div className="view-panel__empty">
          {filter ? "No agents match your search." : "No agents available."}
        </div>
      )}
    </div>
  );
}
