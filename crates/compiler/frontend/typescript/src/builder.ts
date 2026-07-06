/**
 * `GraphBuilder` is the TypeScript recording surface: each method appends one
 * `GraphNode` (an AIS op) to an internal graph and returns a `NodeRef` handle
 * other calls can wire up as a Data/Control/Effect dependency.
 *
 * Deviation from Python: `GraphRecorder` auto-wires `{name}` placeholders in
 * template strings by inspecting the caller's Python stack frames for a
 * local variable bound to a `NodeRef`. TypeScript has no equivalent to frame
 * introspection, so this builder takes dependency bindings explicitly via an
 * `inputs: Record<string, NodeRef>` map — the placeholder name is the key,
 * the dependency is the value. The resulting graph shape (`template_str` /
 * `message` attribute plus an `input_names` array and matching Data edges)
 * is identical to what `GraphRecorder` produces once auto-wiring resolves.
 */
import type { GraphEdge, GraphNode, Parameter } from "./graph.js";
import { ApxmGraph, makeEdge } from "./graph.js";
import type { OpName } from "./generated/ops.js";
import type { DependencyType, ParamType } from "./types.js";

export class NodeRef {
  constructor(
    readonly builder: GraphBuilder,
    readonly nodeId: number,
    readonly name: string,
  ) {}

  toString(): string {
    return `NodeRef(name=${this.name}, id=${this.nodeId})`;
  }
}

type Attrs = Record<string, unknown>;

interface TemplateOpOptions {
  name?: string;
  inputs?: Record<string, NodeRef>;
  [extra: string]: unknown;
}

export interface AskOptions extends TemplateOpOptions {
  prompt: string;
  model?: string;
  temperature?: number;
  systemPrompt?: string;
}

export interface CommunicateOptions extends TemplateOpOptions {
  targetAgent: string;
  message: string;
  protocol?: string;
}

export interface DelegateOptions {
  name?: string;
  taskSpec: string;
  targetAgent: string;
  inputs?: Record<string, NodeRef>;
  [extra: string]: unknown;
}

export interface SpawnAgentOptions {
  name?: string;
  agentName: string;
  profile?: string;
  agentRoute?: string;
  requiredCapabilities?: string[];
  preferredProfiles?: string[];
  mode?: string;
  model?: string;
  cwd?: string;
  capabilities?: string[];
  goals?: string[];
  inputs?: Record<string, NodeRef>;
  [extra: string]: unknown;
}

export interface InvokeCapabilityOptions {
  name?: string;
  capability: string;
  params?: string | Record<string, unknown>;
  inputs?: Record<string, NodeRef>;
  [extra: string]: unknown;
}

export interface RegisterCapabilityOptions {
  name?: string;
  capabilityName: string;
  description?: string;
  parametersSchema?: string | Record<string, unknown>;
  [extra: string]: unknown;
}

export interface PauseOptions {
  name?: string;
  message: string;
  checkpointId?: string;
  timeoutMs?: number;
  inputs?: Record<string, NodeRef>;
  [extra: string]: unknown;
}

export interface ResumeOptions {
  name?: string;
  checkpoint: string;
  pollMaxAttempts?: number;
  pollIntervalMs?: number;
  serverUrl?: string;
  inputs?: Record<string, NodeRef>;
  [extra: string]: unknown;
}

export interface AutonomousOptions {
  name?: string;
  prompt: string;
  maxIterations?: number;
  model?: string;
  provider?: string;
  backend?: string;
  systemPrompt?: string;
  temperature?: number;
  inputs?: Record<string, NodeRef>;
  [extra: string]: unknown;
}

function dropUndefined(attrs: Attrs): Attrs {
  const out: Attrs = {};
  for (const [k, v] of Object.entries(attrs)) {
    if (v !== undefined) out[k] = v;
  }
  return out;
}

/** Records builder calls into an `ApxmGraph`. */
export class GraphBuilder {
  private readonly name: string;
  private nextId = 1;
  private readonly nodes: GraphNode[] = [];
  private readonly edges: GraphEdge[] = [];
  private readonly parameters: Parameter[] = [];
  private readonly nodeIds = new Set<string>();
  private readonly nameCounters = new Map<string, number>();
  private readonly metadata: Record<string, unknown>;
  private readonly agentSessionNodes = new Map<string, NodeRef>();

  constructor(name: string, options: { metadata?: Record<string, unknown> } = {}) {
    this.name = name;
    this.metadata = options.metadata ?? { is_entry: true };
  }

  private autoName(opType: OpName): string {
    const count = this.nameCounters.get(opType) ?? 0;
    this.nameCounters.set(opType, count + 1);
    return count === 0 ? opType.toLowerCase() : `${opType.toLowerCase()}_${count}`;
  }

  private addNode(name: string, op: OpName, attributes: Attrs): NodeRef {
    if (this.nodeIds.has(name)) {
      throw new Error(`workflow node '${name}' already exists`);
    }
    const id = this.nextId;
    this.nextId += 1;
    this.nodes.push({ id, name, op, attributes: dropUndefined(attributes) });
    this.nodeIds.add(name);
    return new NodeRef(this, id, name);
  }

  /** Wire an explicit dependency edge between two previously recorded nodes. */
  addEdge(from: NodeRef, to: NodeRef, dependency: DependencyType = "Data"): void {
    this.edges.push(makeEdge(from.nodeId, to.nodeId, dependency));
  }

  /** Declare a compile-time flow parameter. */
  param(name: string, typeName: ParamType | string = "str"): this {
    if (this.parameters.some((p) => p.name === name)) {
      throw new Error(`parameter '${name}' already exists`);
    }
    this.parameters.push({ name, typeName });
    return this;
  }

  /** Wire the `inputs` map's edges onto an already-recorded node. Explicit
   * TypeScript analogue of `GraphRecorder._resolve_template_refs`. */
  private wireInputs(node: NodeRef, inputs: Record<string, NodeRef> | undefined): void {
    if (!inputs) return;
    for (const key of Object.keys(inputs)) {
      this.addEdge(inputs[key], node);
    }
  }

  /** Simple Q&A with an LLM, no extended thinking (ASK). */
  ask(options: AskOptions): NodeRef {
    const { name, prompt, model, temperature, systemPrompt, inputs, ...rest } = options;
    const inputNames = inputs ? Object.keys(inputs) : [];
    const attrs: Attrs = {
      template_str: prompt,
      model,
      temperature,
      system_prompt: systemPrompt,
      input_names: inputNames.length > 0 ? inputNames : undefined,
      ...rest,
    };
    const node = this.addNode(name ?? this.autoName("ASK"), "ASK", attrs);
    this.wireInputs(node, inputs);
    return node;
  }

  /** Extended chain-of-thought reasoning turn (THINK). */
  think(options: AskOptions): NodeRef {
    const { name, prompt, model, temperature, systemPrompt: _systemPrompt, inputs, ...rest } = options;
    const inputNames = inputs ? Object.keys(inputs) : [];
    const attrs: Attrs = {
      template_str: prompt,
      model,
      temperature,
      input_names: inputNames.length > 0 ? inputNames : undefined,
      ...rest,
    };
    const node = this.addNode(name ?? this.autoName("THINK"), "THINK", attrs);
    this.wireInputs(node, inputs);
    return node;
  }

  /** Send a message to another agent (COMMUNICATE). */
  communicate(options: CommunicateOptions): NodeRef {
    const name = options.name ?? this.autoName("COMMUNICATE");
    const inputNames = options.inputs ? Object.keys(options.inputs) : [];
    const attrs: Attrs = {
      recipient: options.targetAgent,
      message: options.message,
      protocol: options.protocol,
      input_names: inputNames.length > 0 ? inputNames : undefined,
    };
    const node = this.addNode(name, "COMMUNICATE", attrs);
    this.wireInputs(node, options.inputs);

    const sessionNode = this.agentSessionNodes.get(options.targetAgent);
    if (sessionNode) {
      this.addEdge(sessionNode, node, "Data");
      this.agentSessionNodes.set(options.targetAgent, node);
    }
    return node;
  }

  /** Delegate a task to a sub-agent for execution (DELEGATE). */
  delegate(options: DelegateOptions): NodeRef {
    const { name, taskSpec, targetAgent, inputs, ...rest } = options;
    const attrs: Attrs = { task_spec: taskSpec, target_agent: targetAgent, ...rest };
    const node = this.addNode(name ?? this.autoName("DELEGATE"), "DELEGATE", attrs);
    this.wireInputs(node, inputs);
    return node;
  }

  /** Create a new agent instance at runtime (SPAWN_AGENT). */
  spawnAgent(options: SpawnAgentOptions): NodeRef {
    const {
      name,
      agentName,
      profile,
      agentRoute,
      requiredCapabilities,
      preferredProfiles,
      mode,
      model,
      cwd,
      capabilities,
      goals,
      inputs,
      ...rest
    } = options;
    const attrs: Attrs = {
      agent_name: agentName,
      profile,
      agent_route: agentRoute,
      required_capabilities: requiredCapabilities,
      preferred_profiles: preferredProfiles,
      mode,
      model,
      cwd,
      capabilities,
      goals,
      ...rest,
    };
    const node = this.addNode(name ?? this.autoName("SPAWN_AGENT"), "SPAWN_AGENT", attrs);
    this.wireInputs(node, inputs);
    this.agentSessionNodes.set(agentName, node);
    return node;
  }

  /** Invoke a runtime capability/tool (INV_CAP). Python's `invoke()`. */
  invokeCapability(options: InvokeCapabilityOptions): NodeRef {
    const { name, capability, params, inputs, ...rest } = options;
    const paramsStr = typeof params === "object" && params !== null ? JSON.stringify(params) : params;
    const inputNames = inputs ? Object.keys(inputs) : [];
    const attrs: Attrs = {
      capability,
      params_json: paramsStr,
      input_names: inputNames.length > 0 ? inputNames : undefined,
      ...rest,
    };
    const node = this.addNode(name ?? this.autoName("INV_CAP"), "INV_CAP", attrs);
    this.wireInputs(node, inputs);
    return node;
  }

  /** Register a runtime capability (REGISTER_CAPABILITY). Python's `register_capability()`. */
  registerCapability(options: RegisterCapabilityOptions): NodeRef {
    const { name, capabilityName, description, parametersSchema, ...rest } = options;
    const schemaStr =
      typeof parametersSchema === "object" && parametersSchema !== null
        ? JSON.stringify(parametersSchema)
        : parametersSchema;
    const attrs: Attrs = {
      capability_name: capabilityName,
      description,
      parameters_schema: schemaStr,
      ...rest,
    };
    return this.addNode(name ?? this.autoName("REGISTER_CAPABILITY"), "REGISTER_CAPABILITY", attrs);
  }

  /** Suspend execution pending human-in-the-loop review (PAUSE). Attributes
   * (message/checkpointId/timeoutMs) are runtime-only metadata — the current
   * op-spec emits no text for them, only the node's Data-dependency operands. */
  pause(options: PauseOptions): NodeRef {
    const { name, message, checkpointId, timeoutMs, inputs, ...rest } = options;
    const attrs: Attrs = { message, checkpoint_id: checkpointId, timeout_ms: timeoutMs, ...rest };
    const node = this.addNode(name ?? this.autoName("PAUSE"), "PAUSE", attrs);
    this.wireInputs(node, inputs);
    return node;
  }

  /** Resume a suspended PAUSE checkpoint (RESUME). Attributes (checkpoint id,
   * polling config) are runtime-only metadata — the current op-spec emits no
   * text for them, only the node's Data-dependency operands. */
  resume(options: ResumeOptions): NodeRef {
    const { name, checkpoint, pollMaxAttempts, pollIntervalMs, serverUrl, inputs, ...rest } = options;
    const attrs: Attrs = {
      checkpoint,
      poll_max_attempts: pollMaxAttempts,
      poll_interval_ms: pollIntervalMs,
      server_url: serverUrl,
      ...rest,
    };
    const node = this.addNode(name ?? this.autoName("RESUME"), "RESUME", attrs);
    this.wireInputs(node, inputs);
    return node;
  }

  /** Run a goal-directed autonomous loop (AUTONOMOUS). */
  autonomous(options: AutonomousOptions): NodeRef {
    const { name, prompt, maxIterations, model, provider, backend, systemPrompt, temperature, inputs, ...rest } =
      options;
    const attrs: Attrs = {
      prompt,
      max_iterations: maxIterations,
      model,
      provider,
      backend,
      system_prompt: systemPrompt,
      temperature,
      ...rest,
    };
    const node = this.addNode(name ?? this.autoName("AUTONOMOUS"), "AUTONOMOUS", attrs);
    this.wireInputs(node, inputs);
    return node;
  }

  /** Insert a checkpoint barrier (fence with checkpoint semantics). */
  checkpoint(name?: string, attributes: Attrs = {}): NodeRef {
    const attrs: Attrs = { checkpoint: true, ...attributes };
    return this.addNode(name ?? this.autoName("FENCE"), "FENCE", attrs);
  }

  /** Synchronize on every dependency completing (WAIT_ALL). */
  waitAll(name: string | undefined, ...dependencies: NodeRef[]): NodeRef {
    const node = this.addNode(name ?? this.autoName("WAIT_ALL"), "WAIT_ALL", {});
    for (const dep of dependencies) this.addEdge(dep, node);
    return node;
  }

  /** Merge multiple dependency results into one token (MERGE). */
  merge(name: string | undefined, ...dependencies: NodeRef[]): NodeRef {
    const node = this.addNode(name ?? this.autoName("MERGE"), "MERGE", {});
    for (const dep of dependencies) this.addEdge(dep, node);
    return node;
  }

  /** Return from the flow with a result token (RETURN). */
  done(source?: NodeRef, name?: string): NodeRef {
    const node = this.addNode(name ?? this.autoName("RETURN"), "RETURN", {});
    if (source) this.addEdge(source, node);
    return node;
  }

  /** No-op passthrough (NOP). */
  nop(name?: string): NodeRef {
    return this.addNode(name ?? this.autoName("NOP"), "NOP", {});
  }

  /** Materialize the recorded graph. */
  toGraph(): ApxmGraph {
    return new ApxmGraph({
      name: this.name,
      nodes: [...this.nodes],
      edges: [...this.edges],
      parameters: [...this.parameters],
      metadata: { ...this.metadata },
    });
  }

  /** Emit AIR for the recorded graph. */
  toAir(): string {
    return this.toGraph().toAir();
  }
}
