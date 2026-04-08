/**
 * APXM Shared Constants
 *
 * Single source of truth for colors, op categories, event types, edge styles,
 * node statuses, and utility helpers. Load this file BEFORE all other JS.
 */
(function() {
  'use strict';

  var APXM = {};

  // ── 1. Color Palette (mirrors CSS custom properties) ──────────────────────
  APXM.colors = {
    bg:            '#0d1117',
    surface:       '#161b22',
    elevated:      '#1c2333',
    border:        '#30363d',
    borderSubtle:  '#21262d',
    text:          '#e6edf3',
    textSecondary: '#8b949e',
    textMuted:     '#484f58',
    textCode:      '#c9d1d9',
    blue:          '#58a6ff',
    blueBright:    '#79c0ff',
    blueLight:     '#a5d6ff',
    green:         '#3fb950',
    red:           '#f85149',
    redLight:      '#ff7b72',
    yellow:        '#d29922',
    purple:        '#bc8cff',
    purpleLight:   '#d2a8ff',
    orange:        '#f0883e',
    teal:          '#39d2c0',
    white:         '#ffffff',
    blueFocus:     '#1f6feb',
  };

  // ── 2. Operation Categories ───────────────────────────────────────────────
  APXM.opCategories = {
    ASK: 'reasoning', Ask: 'reasoning',
    THINK: 'reasoning', Think: 'reasoning',
    REASON: 'reasoning', Reason: 'reasoning',
    PLAN: 'reasoning', Plan: 'reasoning',
    REFLECT: 'reasoning', Reflect: 'reasoning',
    VERIFY: 'reasoning', Verify: 'reasoning',

    INV: 'tools', Inv: 'tools',
    EXC: 'tools', Exc: 'tools',
    PRINT: 'tools', Print: 'tools',

    JUMP: 'control', Jump: 'control',
    BRANCH_ON_VALUE: 'control', BranchOnValue: 'control',
    LOOP_START: 'control', LoopStart: 'control',
    LOOP_END: 'control', LoopEnd: 'control',
    RETURN: 'control', Return: 'control',
    SWITCH: 'control', Switch: 'control',
    FLOW_CALL: 'control', FlowCall: 'control',

    MERGE: 'sync', Merge: 'sync',
    FENCE: 'sync', Fence: 'sync',
    WAIT_ALL: 'sync', WaitAll: 'sync',
    CHECKPOINT: 'sync', Checkpoint: 'sync',

    COMMUNICATE: 'communication', Communicate: 'communication',
    SPAWN_AGENT: 'communication', SpawnAgent: 'communication',
    DELEGATE: 'communication', Delegate: 'communication',
    NEGOTIATE: 'communication', Negotiate: 'communication',
    REGISTER_CAPABILITY: 'communication', RegisterCapability: 'communication',
    AUTONOMOUS: 'communication', Autonomous: 'communication',

    QMEM: 'memory', Qmem: 'memory',
    UMEM: 'memory', Umem: 'memory',

    UPDATE_GOAL: 'goal', UpdateGoal: 'goal',
    GUARD: 'goal', Guard: 'goal',
    CLAIM: 'goal', Claim: 'goal',
    PAUSE: 'goal', Pause: 'goal',
    RESUME: 'goal', Resume: 'goal',

    TRY_CATCH: 'error', TryCatch: 'error',
    ERR: 'error', Err: 'error',

    NOP: 'identity', Nop: 'identity',
    IDENTITY: 'identity', Identity: 'identity',
    CONST_STR: 'identity', ConstStr: 'identity',
    YIELD: 'identity', Yield: 'identity',

    AGENT: 'meta', Agent: 'meta',
  };

  APXM.getOpCategory = function(opType) {
    return APXM.opCategories[opType] || 'identity';
  };

  // ── 3. Category Display Info ──────────────────────────────────────────────
  APXM.categoryInfo = {
    reasoning:     { label: 'Reasoning',     color: '#58a6ff', fill: '#1a3a5c', stroke: '#58a6ff' },
    tools:         { label: 'Tools',         color: '#3fb950', fill: '#1a3c2a', stroke: '#3fb950' },
    control:       { label: 'Control Flow',  color: '#d29922', fill: '#3c3a1a', stroke: '#d29922' },
    sync:          { label: 'Sync',          color: '#bc8cff', fill: '#2d1a3c', stroke: '#bc8cff' },
    communication: { label: 'Communication', color: '#f0883e', fill: '#3c2a1a', stroke: '#f0883e' },
    memory:        { label: 'Memory',        color: '#39d2c0', fill: '#1a3c3c', stroke: '#39d2c0' },
    goal:          { label: 'Goal/State',    color: '#d29922', fill: '#3c3a1a', stroke: '#d29922' },
    error:         { label: 'Error',         color: '#f85149', fill: '#3c1a1a', stroke: '#f85149' },
    identity:      { label: 'Identity',      color: '#8b949e', fill: '#2a2a2a', stroke: '#6e7681' },
    meta:          { label: 'Metadata',      color: '#8b949e', fill: '#2a2a2a', stroke: '#6e7681' },
  };

  APXM.getCategoryColor = function(category) {
    var info = APXM.categoryInfo[category];
    return info ? info.color : '#8b949e';
  };

  APXM.getCategoryFill = function(category) {
    var info = APXM.categoryInfo[category];
    return info ? info.fill : '#2a2a2a';
  };

  APXM.getCategoryStroke = function(category) {
    var info = APXM.categoryInfo[category];
    return info ? info.stroke : '#6e7681';
  };

  // ── 4. Event Types (matching apxm-events crate) ──────────────────────────
  APXM.eventKinds = {
    // LLM Layer
    TOKEN: 'token',
    THOUGHT: 'thought',
    TOOL_CALL: 'tool_call',
    LLM_DONE: 'llm_done',
    USAGE: 'usage',
    RETRY: 'retry',
    WARNING: 'warning',
    CITATION: 'citation',
    PROVIDER_EVENT: 'provider_event',
    // Runtime Layer
    OPERATION_START: 'operation_start',
    OPERATION_END: 'operation_end',
    TOOL_START: 'tool_start',
    TOOL_END: 'tool_end',
    PLAN_CREATED: 'plan_created',
    PLAN_STEP_STARTED: 'plan_step_started',
    PLAN_STEP_COMPLETED: 'plan_step_completed',
    MEMORY_READ: 'memory_read',
    MEMORY_WRITE: 'memory_write',
    CHECKPOINT_SAVED: 'checkpoint_saved',
    CHECKPOINT_RESTORED: 'checkpoint_restored',
    SCHEDULER_DECISION: 'scheduler_decision',
    GPU_UTILIZATION: 'gpu_utilization',
    TOKEN_USAGE: 'token_usage',
    MEMOIZATION_HIT: 'memoization_hit',
    ERROR: 'error',
    // Session Layer
    CONTEXT_COMPACTED: 'context_compacted',
    MODEL_REROUTED: 'model_rerouted',
    CANCELLED: 'cancelled',
    LOOP_DETECTED: 'loop_detected',
    CONTEXT_WINDOW_WARNING: 'context_window_warning',
    SESSION_START: 'session_start',
    SESSION_END: 'session_end',
    TURN_BOUNDARY: 'turn_boundary',
  };

  APXM.eventColors = {
    operation_start:    '#58a6ff',
    operation_end:      '#58a6ff',
    token:              '#bc8cff',
    thought:            '#bc8cff',
    tool_call:          '#3fb950',
    tool_start:         '#3fb950',
    tool_end:           '#3fb950',
    llm_done:           '#bc8cff',
    usage:              '#8b949e',
    error:              '#f85149',
    session_start:      '#39d2c0',
    session_end:        '#39d2c0',
    scheduler_decision: '#d29922',
    memoization_hit:    '#3fb950',
    _default:           '#8b949e',
  };

  APXM.getEventColor = function(kind) {
    return APXM.eventColors[kind] || APXM.eventColors._default;
  };

  // ── 5. Edge / Dependency Types ────────────────────────────────────────────
  APXM.edgeStyles = {
    Data:    { color: '#58a6ff', dash: '',    label: 'Data' },
    Control: { color: '#bc8cff', dash: '6,3', label: 'Control' },
    Effect:  { color: '#d29922', dash: '2,3', label: 'Effect' },
  };

  // ── 6. Optimization Passes ────────────────────────────────────────────────
  APXM.passes = {
    prompt_caching:     { label: 'Prompt Caching',     color: '#58a6ff', description: 'Detect shared system prompts across ASK/THINK nodes' },
    memoization_hints:  { label: 'Memoization Hints',  color: '#3fb950', description: 'Detect duplicate pure operations with identical attributes' },
    pipeline_detection: { label: 'Pipeline Detection', color: '#bc8cff', description: 'Detect ASK\u2192ASK chains eligible for token pipelining' },
  };

  // ── 7. Node Status Styles ─────────────────────────────────────────────────
  APXM.nodeStatus = {
    pending:   { label: 'Pending',   color: '#8b949e', cssClass: 'node-pending' },
    running:   { label: 'Running',   color: '#58a6ff', cssClass: 'node-running' },
    completed: { label: 'Completed', color: '#3fb950', cssClass: 'node-completed' },
    failed:    { label: 'Failed',    color: '#f85149', cssClass: 'node-failed' },
  };

  // ── 8. API Endpoints ─────────────────────────────────────────────────────
  APXM.api = {
    graph:         '/api/graph',
    graphAnalyze:  '/api/graph/analyze',
    graphOptimized:'/api/graph/optimized',
    ops:           '/api/ops',
    session:       '/api/session',
    sessionNode:   '/api/session/node',   // append /{id}
    config:        '/api/config',
    startup:       '/api/startup',
    examples:      '/api/examples',
    liveSession:   '/api/live/session',
    liveNode:      '/api/live/node',      // append /{id}
    sessions:      '/api/sessions',
  };

  // ── 9. DOM Element IDs ──────────────────────────────────────────────────
  APXM.el = {
    graphCanvas:       'graph-canvas',
    graphCanvasEmpty:  'graph-canvas-empty',
    graphPathInput:    'graph-path-input',
    loadGraphBtn:      'load-graph-btn',
    sessionPathInput:  'session-path-input',
    loadSessionBtn:    'load-session-btn',
    examplesSelect:    'examples-select',
    nodeInspector:     'node-inspector',
    inspectorContent:  'inspector-content',
    inspectorThinking: 'inspector-thinking',
    thinkingContent:   'thinking-content',
    graphAnalysis:     'graph-analysis',
    statusBar:         'status-bar',
    statusMessage:     'status-message',
    statusVersion:     'status-version',
    // Analysis strip stats
    statNodes:         'stat-nodes',
    statEdges:         'stat-edges',
    statParallelism:   'stat-parallelism',
    statCriticalPath:  'stat-critical-path',
    statSpeedup:       'stat-speedup',
  };

  // ── 10. CSS Selectors ───────────────────────────────────────────────────
  APXM.css = {
    tabPanel:   '.tab-panel',
    tabBtn:     '.tab-btn',
    active:     'active',
    emptyHint:  'empty-hint',
    error:      'error',
    placeholder:'placeholder',
  };

  // ── 11. Utilities ──────────────────────────────────────────────────────────
  APXM.formatDuration = function(ms) {
    if (ms < 1000) return ms + 'ms';
    if (ms < 60000) return (ms / 1000).toFixed(1) + 's';
    var m = Math.floor(ms / 60000);
    var s = Math.round((ms % 60000) / 1000);
    return m + 'm ' + s + 's';
  };

  APXM.truncate = function(str, maxLen) {
    if (!str) return '';
    maxLen = maxLen || 80;
    return str.length <= maxLen ? str : str.slice(0, maxLen) + '...';
  };

  // ── Export ────────────────────────────────────────────────────────────────
  window.APXM = APXM;
})();
