// live.js -- APXM Live Execution Viewer
// Connects to the SSE endpoint to show real-time execution of agent workflows.
// Exports: window.LiveViewer

(function () {
  "use strict";

  // ---------------------------------------------------------------------------
  // Shared constants (populated by constants.js when loaded before this file)
  // ---------------------------------------------------------------------------

  var C  = window.APXM || {};
  var COL = C.colors || {};
  var EK = C.eventKinds || {};
  var NS = C.nodeStatus || {};

  // Event kind shortcuts -- prefer shared constants, fall back to literals
  var OPERATION_START = EK.OPERATION_START || "operation_start";
  var OPERATION_END   = EK.OPERATION_END   || "operation_end";
  var TOKEN           = EK.TOKEN           || "token";
  var THOUGHT         = EK.THOUGHT         || "thought";
  var LLM_DONE        = EK.LLM_DONE        || "llm_done";
  var TOOL_CALL       = EK.TOOL_CALL       || "tool_call";
  var TOOL_START      = EK.TOOL_START      || "tool_start";
  var TOOL_END        = EK.TOOL_END        || "tool_end";
  var USAGE           = EK.USAGE           || "usage";
  var TOKEN_USAGE     = EK.TOKEN_USAGE     || "token_usage";
  var ERROR           = EK.ERROR           || "error";
  var SESSION_START   = EK.SESSION_START   || "session_start";
  var SESSION_END     = EK.SESSION_END     || "session_end";

  var SVGNS = "http://www.w3.org/2000/svg";

  // Node status colors -- prefer shared constants, fall back to inline values
  var STATUS_COLORS = {
    pending:   (NS.pending   && NS.pending.color)   || COL.textSecondary || "#8b949e",
    running:   (NS.running   && NS.running.color)   || COL.blue          || "#58a6ff",
    completed: (NS.completed && NS.completed.color) || COL.green         || "#3fb950",
    failed:    (NS.failed    && NS.failed.color)    || COL.red           || "#f85149",
  };

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  function el(tag, className, text) {
    var node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = text;
    return node;
  }

  function esc(str) {
    var div = document.createElement("div");
    div.textContent = str;
    return div.innerHTML;
  }

  /**
   * Format a duration in milliseconds to a human-readable string.
   * Delegates to shared APXM.formatDuration when available.
   */
  function formatDuration(ms) {
    if (ms == null) return "--";
    if (C.formatDuration) return C.formatDuration(ms);
    if (ms < 1000) return ms + "ms";
    if (ms < 60000) {
      var seconds = ms / 1000;
      return (seconds % 1 === 0 ? seconds.toFixed(0) : seconds.toFixed(1)) + "s";
    }
    var minutes = Math.floor(ms / 60000);
    var secs = Math.round((ms % 60000) / 1000);
    return minutes + "m " + secs + "s";
  }

  function formatTimestamp(isoString, baseTime) {
    if (!isoString || !baseTime) return "";
    var t = new Date(isoString).getTime();
    var base = baseTime instanceof Date ? baseTime.getTime() : baseTime;
    var delta = (t - base) / 1000;
    if (delta < 0) delta = 0;
    return "+" + delta.toFixed(3) + "s";
  }

  /**
   * Truncate a string to a maximum length, appending an ellipsis if needed.
   * Delegates to shared APXM.truncate when available.
   */
  function truncate(str, maxLen) {
    if (str == null) return "";
    if (typeof str !== "string") str = String(str);
    if (C.truncate) return C.truncate(str, maxLen);
    if (str.length <= maxLen) return str;
    return str.slice(0, maxLen) + "\u2026";
  }

  /**
   * Return a color hex string for a given trace event kind.
   * Delegates to shared APXM.getEventColor when available.
   */
  function colorForKind(kind) {
    if (C.getEventColor) return C.getEventColor(kind);
    switch (kind) {
      case OPERATION_START:
      case OPERATION_END:
        return COL.blue || "#58a6ff";
      case TOKEN:
      case THOUGHT:
      case LLM_DONE:
        return COL.purple || "#bc8cff";
      case TOOL_CALL:
      case TOOL_START:
      case TOOL_END:
        return COL.green || "#3fb950";
      case ERROR:
        return COL.red || "#f85149";
      case SESSION_START:
      case SESSION_END:
        return COL.teal || "#39d2c0";
      default:
        return COL.textSecondary || "#8b949e";
    }
  }

  function nowMs() {
    return Date.now();
  }

  // ---------------------------------------------------------------------------
  // CSS Injection
  // ---------------------------------------------------------------------------

  function injectStyles() {
    if (document.getElementById("live-viewer-styles")) return;

    // Color shortcuts from shared constants
    var _COL = C.colors || {};
    var cTextSec   = _COL.textSecondary || "#8b949e";
    var cBlue      = _COL.blue          || "#58a6ff";
    var cGreen     = _COL.green         || "#3fb950";
    var cRed       = _COL.red           || "#f85149";

    var css = [
      // Layout: 2x2 grid
      ".live-dashboard {",
      "  display: grid;",
      "  grid-template-columns: 60% 40%;",
      "  grid-template-rows: 1fr 1fr;",
      "  height: 100%;",
      "  overflow: hidden;",
      "  gap: 1px;",
      "  background: var(--border, #30363d);",
      "}",

      // Quadrant panels
      ".live-graph {",
      "  grid-row: 1;",
      "  grid-column: 1;",
      "  overflow: hidden;",
      "  background: var(--bg, #0d1117);",
      "  position: relative;",
      "}",
      ".live-output {",
      "  grid-row: 1;",
      "  grid-column: 2;",
      "  overflow: hidden;",
      "  background: var(--surface, #161b22);",
      "  display: flex;",
      "  flex-direction: column;",
      "}",
      ".live-feed {",
      "  grid-row: 2;",
      "  grid-column: 1;",
      "  overflow: hidden;",
      "  background: var(--surface, #161b22);",
      "  display: flex;",
      "  flex-direction: column;",
      "}",
      ".live-metrics {",
      "  grid-row: 2;",
      "  grid-column: 2;",
      "  overflow: hidden;",
      "  background: var(--surface, #161b22);",
      "  padding: 12px;",
      "  display: flex;",
      "  flex-direction: column;",
      "  gap: 12px;",
      "}",

      // Panel headers
      ".live-panel-header {",
      "  display: flex;",
      "  align-items: center;",
      "  justify-content: space-between;",
      "  padding: 6px 12px;",
      "  background: var(--surface-elevated, #1c2333);",
      "  border-bottom: 1px solid var(--border, #30363d);",
      "  font-size: 11px;",
      "  font-weight: 600;",
      "  text-transform: uppercase;",
      "  letter-spacing: 0.8px;",
      "  color: var(--text-secondary, #8b949e);",
      "  flex-shrink: 0;",
      "}",

      ".live-panel-body {",
      "  flex: 1;",
      "  overflow: auto;",
      "  min-height: 0;",
      "}",

      // SVG node status overlays
      ".node-pending > rect { stroke: " + cTextSec + " !important; stroke-dasharray: 4 3; stroke-width: 1.5 !important; }",
      ".node-running > rect { stroke: " + cBlue + " !important; stroke-width: 2.5 !important; animation: live-pulse 1.5s ease-in-out infinite; }",
      ".node-completed > rect { stroke: " + cGreen + " !important; stroke-width: 2 !important; }",
      ".node-failed > rect { stroke: " + cRed + " !important; stroke-width: 2.5 !important; }",

      "@keyframes live-pulse {",
      "  0%, 100% { stroke-opacity: 1; filter: drop-shadow(0 0 3px rgba(88, 166, 255, 0.6)); }",
      "  50% { stroke-opacity: 0.5; filter: drop-shadow(0 0 8px rgba(88, 166, 255, 0.3)); }",
      "}",

      // Progress bar under running nodes
      ".node-progress-bar { fill: rgba(88, 166, 255, 0.3); }",
      ".node-progress-fill { fill: " + cBlue + "; transition: width 0.3s ease; }",

      // Error icon in failed nodes
      ".node-error-icon { fill: " + cRed + "; }",

      // Output panel: tab bar
      ".live-output-tabs {",
      "  display: flex;",
      "  gap: 1px;",
      "  background: var(--border, #30363d);",
      "  flex-shrink: 0;",
      "  overflow-x: auto;",
      "}",
      ".live-output-tab {",
      "  padding: 4px 10px;",
      "  font-size: 11px;",
      "  font-family: var(--font-mono, monospace);",
      "  background: var(--surface, #161b22);",
      "  color: var(--text-secondary, #8b949e);",
      "  border: none;",
      "  cursor: pointer;",
      "  white-space: nowrap;",
      "  transition: color 0.15s ease, background 0.15s ease;",
      "}",
      ".live-output-tab:hover { color: var(--text-primary, #e6edf3); }",
      ".live-output-tab.active {",
      "  color: var(--text-primary, #e6edf3);",
      "  background: var(--surface-elevated, #1c2333);",
      "  border-bottom: 2px solid var(--accent-blue, #58a6ff);",
      "}",

      // Token stream
      ".token-stream {",
      "  font-family: var(--font-mono, monospace);",
      "  font-size: 12px;",
      "  line-height: 1.6;",
      "  color: var(--text-primary, #e6edf3);",
      "  white-space: pre-wrap;",
      "  word-break: break-word;",
      "  padding: 8px 12px;",
      "  margin: 0;",
      "}",

      // Thinking blocks
      ".thinking-block {",
      "  margin: 6px 12px;",
      "  border: 1px solid var(--border, #30363d);",
      "  border-radius: 4px;",
      "  background: rgba(139, 148, 158, 0.06);",
      "  overflow: hidden;",
      "}",
      ".thinking-block-header {",
      "  display: flex;",
      "  align-items: center;",
      "  gap: 6px;",
      "  padding: 4px 8px;",
      "  font-size: 10px;",
      "  font-weight: 600;",
      "  text-transform: uppercase;",
      "  letter-spacing: 0.5px;",
      "  color: var(--text-secondary, #8b949e);",
      "  cursor: pointer;",
      "  user-select: none;",
      "}",
      ".thinking-block-header:hover { color: var(--text-primary, #e6edf3); }",
      ".thinking-block-body {",
      "  padding: 6px 10px;",
      "  font-size: 11px;",
      "  line-height: 1.5;",
      "  color: var(--text-secondary, #8b949e);",
      "  white-space: pre-wrap;",
      "  word-break: break-word;",
      "  border-top: 1px solid var(--border, #30363d);",
      "}",

      // Tool blocks
      ".tool-block {",
      "  margin: 6px 12px;",
      "  padding: 6px 10px;",
      "  border: 1px solid rgba(63, 185, 80, 0.2);",
      "  border-left: 3px solid " + cGreen + ";",
      "  border-radius: 0 4px 4px 0;",
      "  background: rgba(63, 185, 80, 0.06);",
      "  font-size: 11px;",
      "  font-family: var(--font-mono, monospace);",
      "}",
      ".tool-block-name {",
      "  font-weight: 700;",
      "  color: " + cGreen + ";",
      "}",
      ".tool-block-args {",
      "  color: var(--text-secondary, #8b949e);",
      "  white-space: pre-wrap;",
      "  word-break: break-word;",
      "  margin-top: 2px;",
      "}",
      ".tool-block-result {",
      "  color: var(--text-primary, #e6edf3);",
      "  white-space: pre-wrap;",
      "  word-break: break-word;",
      "  margin-top: 4px;",
      "  padding-top: 4px;",
      "  border-top: 1px solid rgba(63, 185, 80, 0.15);",
      "}",

      // Event feed
      ".live-feed-list {",
      "  display: flex;",
      "  flex-direction: column;",
      "  gap: 1px;",
      "  padding: 4px 0;",
      "}",
      ".live-feed-item {",
      "  display: flex;",
      "  align-items: baseline;",
      "  gap: 8px;",
      "  padding: 3px 12px;",
      "  font-size: 11px;",
      "  transition: background 0.1s ease;",
      "}",
      ".live-feed-item:hover { background: var(--surface-elevated, #1c2333); }",
      ".live-feed-ts {",
      "  color: var(--text-secondary, #8b949e);",
      "  font-family: var(--font-mono, monospace);",
      "  flex-shrink: 0;",
      "  min-width: 72px;",
      "  font-size: 10px;",
      "}",
      ".live-feed-dot {",
      "  width: 6px;",
      "  height: 6px;",
      "  border-radius: 50%;",
      "  flex-shrink: 0;",
      "  margin-top: 4px;",
      "}",
      ".live-feed-desc {",
      "  color: var(--text-primary, #e6edf3);",
      "  word-break: break-word;",
      "  flex: 1;",
      "}",
      ".live-feed-desc--error { color: " + cRed + "; }",

      // Metrics panel
      ".live-metrics-grid {",
      "  display: grid;",
      "  grid-template-columns: 1fr 1fr;",
      "  gap: 10px;",
      "}",
      ".live-metric-card {",
      "  padding: 10px 12px;",
      "  border: 1px solid var(--border, #30363d);",
      "  border-radius: 6px;",
      "  background: var(--bg, #0d1117);",
      "  display: flex;",
      "  flex-direction: column;",
      "  gap: 4px;",
      "}",
      ".live-metric-label {",
      "  font-size: 10px;",
      "  font-weight: 600;",
      "  text-transform: uppercase;",
      "  letter-spacing: 0.8px;",
      "  color: var(--text-secondary, #8b949e);",
      "}",
      ".live-metric-value {",
      "  font-size: 18px;",
      "  font-weight: 700;",
      "  color: var(--text-primary, #e6edf3);",
      "  line-height: 1.2;",
      "}",

      // Node status summary strip
      ".live-node-summary {",
      "  display: flex;",
      "  flex-wrap: wrap;",
      "  gap: 6px;",
      "}",
      ".live-node-pill {",
      "  display: inline-flex;",
      "  align-items: center;",
      "  gap: 4px;",
      "  padding: 2px 8px;",
      "  border-radius: 10px;",
      "  font-size: 10px;",
      "  font-weight: 600;",
      "}",

      // Session complete overlay
      ".live-complete-banner {",
      "  padding: 8px 12px;",
      "  background: rgba(63, 185, 80, 0.12);",
      "  border: 1px solid rgba(63, 185, 80, 0.3);",
      "  border-radius: 6px;",
      "  color: " + cGreen + ";",
      "  font-size: 12px;",
      "  font-weight: 600;",
      "  text-align: center;",
      "}",
      ".live-complete-banner--failed {",
      "  background: rgba(248, 81, 73, 0.12);",
      "  border-color: rgba(248, 81, 73, 0.3);",
      "  color: " + cRed + ";",
      "}",

      // Empty / waiting state
      ".live-waiting {",
      "  display: flex;",
      "  align-items: center;",
      "  justify-content: center;",
      "  height: 100%;",
      "  color: var(--text-secondary, #8b949e);",
      "  font-size: 12px;",
      "}",
    ].join("\n");

    var style = document.createElement("style");
    style.id = "live-viewer-styles";
    style.textContent = css;
    document.head.appendChild(style);
  }

  // ---------------------------------------------------------------------------
  // Internal state factory
  // ---------------------------------------------------------------------------

  function createLiveState() {
    return {
      eventSource: null,
      nodeStates: {},       // nodeId -> {status, startTime, output, thinking, tools, tokens, opType, duration}
      focusedNode: null,    // which node's output we're watching
      totalTokens: { input: 0, output: 0 },
      startTime: null,
      events: [],           // all events for replay
      nodesCompleted: 0,
      nodesFailed: 0,
      nodesTotal: 0,
      sessionStatus: "running",
    };
  }

  // ---------------------------------------------------------------------------
  // DOM References holder
  // ---------------------------------------------------------------------------

  function createDomRefs() {
    return {
      graphContainer: null,
      outputBody: null,
      outputTabs: null,
      feedList: null,
      metricValues: {},     // label -> element
      nodeSummary: null,
      metricsPanel: null,
    };
  }

  // ---------------------------------------------------------------------------
  // handleTraceEvent
  // ---------------------------------------------------------------------------

  function handleTraceEvent(event, state, dom) {
    var payload = event.payload || {};
    var kind = payload.kind;

    state.events.push(event);

    switch (kind) {
      case OPERATION_START:
        handleOperationStart(payload, event.meta, state, dom);
        break;
      case OPERATION_END:
        handleOperationEnd(payload, event.meta, state, dom);
        break;
      case TOKEN:
        handleToken(payload, event.meta, state, dom);
        break;
      case THOUGHT:
        handleThought(payload, event.meta, state, dom);
        break;
      case LLM_DONE:
        handleLlmDone(payload, event.meta, state, dom);
        break;
      case TOOL_CALL:
        handleToolCall(payload, event.meta, state, dom);
        break;
      case TOOL_START:
        handleToolStart(payload, event.meta, state, dom);
        break;
      case TOOL_END:
        handleToolEnd(payload, event.meta, state, dom);
        break;
      case ERROR:
        handleError(payload, event.meta, state, dom);
        break;
      case USAGE:
      case TOKEN_USAGE:
        handleUsage(payload, event.meta, state, dom);
        break;
      case SESSION_START:
        if (!state.startTime && event.meta && event.meta.timestamp) {
          state.startTime = new Date(event.meta.timestamp);
        }
        break;
      case SESSION_END:
        state.sessionStatus = "completed";
        break;
      default:
        break;
    }

    // Append to event feed
    appendFeedItem(event, state, dom);

    // Update metrics display
    updateMetricsDisplay(state, dom);
  }

  // ---------------------------------------------------------------------------
  // Event handlers
  // ---------------------------------------------------------------------------

  function ensureNodeState(nodeId, state) {
    if (!state.nodeStates[nodeId]) {
      state.nodeStates[nodeId] = {
        status: "pending",
        startTime: null,
        output: "",
        thinking: "",
        tools: [],
        tokens: { input: 0, output: 0 },
        opType: "",
        duration: null,
        errorMessage: null,
      };
    }
    return state.nodeStates[nodeId];
  }

  function handleOperationStart(payload, meta, state, dom) {
    var nodeId = payload.node_id;
    var ns = ensureNodeState(nodeId, state);
    ns.status = "running";
    ns.startTime = meta && meta.timestamp ? new Date(meta.timestamp).getTime() : nowMs();
    ns.opType = payload.op_type || "";

    // Auto-focus on the newly started node
    state.focusedNode = nodeId;
    updateOutputPanel(state, dom);
    updateOutputTabs(state, dom);
    updateGraphOverlay(dom.graphContainer, state.nodeStates);
  }

  function handleOperationEnd(payload, meta, state, dom) {
    var nodeId = payload.node_id;
    var ns = ensureNodeState(nodeId, state);
    var success = payload.success !== false;
    ns.status = success ? "completed" : "failed";
    ns.duration = payload.duration_ms || null;

    if (success) {
      state.nodesCompleted++;
    } else {
      state.nodesFailed++;
    }

    updateGraphOverlay(dom.graphContainer, state.nodeStates);
    updateOutputTabs(state, dom);
  }

  function handleToken(payload, meta, state, dom) {
    // Tokens carry text; find the node from the focused node context
    // or from a node_id field if present
    var nodeId = payload.node_id != null ? payload.node_id : state.focusedNode;
    if (nodeId == null) return;

    var ns = ensureNodeState(nodeId, state);
    ns.output += payload.text || "";

    // If this is the focused node, append to the token stream
    if (nodeId === state.focusedNode) {
      appendToTokenStream(payload.text || "", dom);
    }
  }

  function handleThought(payload, meta, state, dom) {
    var nodeId = payload.node_id != null ? payload.node_id : state.focusedNode;
    if (nodeId == null) return;

    var ns = ensureNodeState(nodeId, state);
    ns.thinking += (payload.text || "") + "\n";

    if (nodeId === state.focusedNode) {
      appendThinkingBlock(payload.text || "", payload.summary, dom);
    }
  }

  function handleLlmDone(payload, meta, state, dom) {
    var nodeId = payload.node_id != null ? payload.node_id : state.focusedNode;
    if (nodeId == null) return;

    var ns = ensureNodeState(nodeId, state);

    // Update token usage from the llm_done usage field
    if (payload.usage) {
      ns.tokens.input += payload.usage.input_tokens || 0;
      ns.tokens.output += payload.usage.output_tokens || 0;
      state.totalTokens.input += payload.usage.input_tokens || 0;
      state.totalTokens.output += payload.usage.output_tokens || 0;
    }

    // If content was provided and output is still empty, set it
    if (payload.content && !ns.output) {
      ns.output = payload.content;
    }

    if (nodeId === state.focusedNode) {
      appendLlmDoneInfo(payload, dom);
    }
  }

  function handleToolCall(payload, meta, state, dom) {
    var nodeId = payload.node_id != null ? payload.node_id : state.focusedNode;
    if (nodeId == null) return;

    var ns = ensureNodeState(nodeId, state);
    ns.tools.push({
      type: "call",
      name: payload.name || "?",
      args: payload.arguments || {},
      result: null,
    });

    if (nodeId === state.focusedNode) {
      appendToolBlock(payload.name || "?", payload.arguments, null, dom);
    }
  }

  function handleToolStart(payload, meta, state, dom) {
    var nodeId = payload.node_id != null ? payload.node_id : state.focusedNode;
    if (nodeId == null) return;

    var ns = ensureNodeState(nodeId, state);
    ns.tools.push({
      type: "start",
      name: payload.name || "?",
      args: payload.args || {},
      result: null,
    });

    if (nodeId === state.focusedNode) {
      appendToolBlock(payload.name || "?", payload.args, null, dom);
    }
  }

  function handleToolEnd(payload, meta, state, dom) {
    var nodeId = payload.node_id != null ? payload.node_id : state.focusedNode;
    if (nodeId == null) return;

    // Update the last tool entry with the result
    var ns = ensureNodeState(nodeId, state);
    for (var i = ns.tools.length - 1; i >= 0; i--) {
      if (ns.tools[i].name === (payload.name || "?") && ns.tools[i].result == null) {
        ns.tools[i].result = payload.result;
        break;
      }
    }

    if (nodeId === state.focusedNode) {
      appendToolResult(payload.name || "?", payload.result, dom);
    }
  }

  function handleError(payload, meta, state, dom) {
    var nodeId = payload.node_id != null ? payload.node_id : state.focusedNode;
    if (nodeId != null) {
      var ns = ensureNodeState(nodeId, state);
      ns.status = "failed";
      ns.errorMessage = payload.message || "Unknown error";
      state.nodesFailed++;
      updateGraphOverlay(dom.graphContainer, state.nodeStates);
    }
  }

  function handleUsage(payload, meta, state, dom) {
    var inputTokens = payload.input_tokens || 0;
    var outputTokens = payload.output_tokens || 0;
    state.totalTokens.input += inputTokens;
    state.totalTokens.output += outputTokens;

    if (payload.node_id != null) {
      var ns = ensureNodeState(payload.node_id, state);
      ns.tokens.input += inputTokens;
      ns.tokens.output += outputTokens;
    }
  }

  // ---------------------------------------------------------------------------
  // Output panel rendering
  // ---------------------------------------------------------------------------

  function createOutputPanel(container) {
    var header = el("div", "live-panel-header", "Live Output");
    container.appendChild(header);

    var tabs = el("div", "live-output-tabs");
    container.appendChild(tabs);

    var body = el("div", "live-panel-body");
    var stream = el("pre", "token-stream");
    body.appendChild(stream);
    container.appendChild(body);

    return {
      tabs: tabs,
      body: body,
      stream: stream,
    };
  }

  function updateOutputTabs(state, dom) {
    if (!dom.outputTabs) return;
    dom.outputTabs.innerHTML = "";

    var nodeIds = Object.keys(state.nodeStates).sort(function (a, b) {
      return Number(a) - Number(b);
    });

    nodeIds.forEach(function (nid) {
      var ns = state.nodeStates[nid];
      var numId = Number(nid);
      var label = "#" + nid;
      if (ns.opType) label += " " + ns.opType;

      var tab = el("button", "live-output-tab", label);
      if (numId === state.focusedNode) {
        tab.classList.add("active");
      }

      // Status color indicator
      var dot = el("span");
      dot.style.display = "inline-block";
      dot.style.width = "6px";
      dot.style.height = "6px";
      dot.style.borderRadius = "50%";
      dot.style.backgroundColor = STATUS_COLORS[ns.status] || STATUS_COLORS.pending;
      dot.style.marginRight = "4px";
      tab.insertBefore(dot, tab.firstChild);

      tab.addEventListener("click", function () {
        state.focusedNode = numId;
        updateOutputTabs(state, dom);
        updateOutputPanel(state, dom);
      });

      dom.outputTabs.appendChild(tab);
    });
  }

  function updateOutputPanel(state, dom) {
    if (!dom.outputBody) return;
    dom.outputBody.innerHTML = "";

    var stream = el("pre", "token-stream");
    dom.outputBody.appendChild(stream);

    if (state.focusedNode == null) {
      stream.textContent = "Waiting for node execution...";
      return;
    }

    var ns = state.nodeStates[state.focusedNode];
    if (!ns) {
      stream.textContent = "No data for node " + state.focusedNode;
      return;
    }

    // Render thinking blocks first
    if (ns.thinking) {
      var thinkBlock = createThinkingBlockElement(ns.thinking);
      dom.outputBody.insertBefore(thinkBlock, stream);
    }

    // Render tool calls
    ns.tools.forEach(function (tool) {
      var toolEl = createToolBlockElement(tool.name, tool.args, tool.result);
      dom.outputBody.insertBefore(toolEl, stream);
    });

    // Render output stream
    stream.textContent = ns.output || "";
    scrollToBottom(dom.outputBody);
  }

  function appendToTokenStream(text, dom) {
    if (!dom.outputBody) return;
    var stream = dom.outputBody.querySelector(".token-stream");
    if (!stream) return;

    // Check if user has scrolled up
    var isAtBottom = isScrolledToBottom(dom.outputBody);

    stream.textContent += text;

    if (isAtBottom) {
      scrollToBottom(dom.outputBody);
    }
  }

  function appendThinkingBlock(text, summary, dom) {
    if (!dom.outputBody) return;
    var stream = dom.outputBody.querySelector(".token-stream");

    // Find or create the current thinking block
    var existing = dom.outputBody.querySelector(".thinking-block:last-of-type");
    if (existing && !existing.dataset.finalized) {
      var body = existing.querySelector(".thinking-block-body");
      if (body) {
        body.textContent += text + "\n";
      }
    } else {
      var block = createThinkingBlockElement(text);
      if (stream) {
        dom.outputBody.insertBefore(block, stream);
      } else {
        dom.outputBody.appendChild(block);
      }
    }

    if (isScrolledToBottom(dom.outputBody)) {
      scrollToBottom(dom.outputBody);
    }
  }

  function createThinkingBlockElement(text) {
    var block = el("div", "thinking-block");
    var header = el("div", "thinking-block-header");
    var arrow = el("span", null, "\u25B6 ");
    var label = el("span", null, "Thinking");
    header.appendChild(arrow);
    header.appendChild(label);

    var body = el("div", "thinking-block-body", text);
    body.style.display = "none";

    header.addEventListener("click", function () {
      var visible = body.style.display !== "none";
      body.style.display = visible ? "none" : "";
      arrow.textContent = visible ? "\u25B6 " : "\u25BC ";
    });

    block.appendChild(header);
    block.appendChild(body);
    return block;
  }

  function appendToolBlock(name, args, result, dom) {
    if (!dom.outputBody) return;
    var stream = dom.outputBody.querySelector(".token-stream");

    var block = createToolBlockElement(name, args, result);
    if (stream) {
      dom.outputBody.insertBefore(block, stream);
    } else {
      dom.outputBody.appendChild(block);
    }

    if (isScrolledToBottom(dom.outputBody)) {
      scrollToBottom(dom.outputBody);
    }
  }

  function createToolBlockElement(name, args, result) {
    var block = el("div", "tool-block");

    var nameEl = el("span", "tool-block-name", name);
    block.appendChild(nameEl);

    if (args) {
      var argsStr = typeof args === "string" ? args : JSON.stringify(args, null, 2);
      if (argsStr && argsStr !== "{}") {
        var argsEl = el("div", "tool-block-args", argsStr);
        block.appendChild(argsEl);
      }
    }

    if (result != null) {
      var resultStr = typeof result === "string" ? result : JSON.stringify(result, null, 2);
      var resultEl = el("div", "tool-block-result", resultStr);
      block.appendChild(resultEl);
    }

    return block;
  }

  function appendToolResult(name, result, dom) {
    if (!dom.outputBody) return;
    // Find the last tool block for this name without a result
    var blocks = dom.outputBody.querySelectorAll(".tool-block");
    for (var i = blocks.length - 1; i >= 0; i--) {
      var nameEl = blocks[i].querySelector(".tool-block-name");
      var resultEl = blocks[i].querySelector(".tool-block-result");
      if (nameEl && nameEl.textContent === name && !resultEl) {
        var str = result != null ? (typeof result === "string" ? result : JSON.stringify(result, null, 2)) : "(no result)";
        var rEl = el("div", "tool-block-result", str);
        blocks[i].appendChild(rEl);
        break;
      }
    }
  }

  function appendLlmDoneInfo(payload, dom) {
    if (!dom.outputBody) return;
    // Mark the current thinking block as finalized
    var thinking = dom.outputBody.querySelector(".thinking-block:last-of-type");
    if (thinking) {
      thinking.dataset.finalized = "true";
    }
  }

  function isScrolledToBottom(container) {
    return container.scrollHeight - container.scrollTop - container.clientHeight < 30;
  }

  function scrollToBottom(container) {
    container.scrollTop = container.scrollHeight;
  }

  // ---------------------------------------------------------------------------
  // Event feed
  // ---------------------------------------------------------------------------

  function appendFeedItem(event, state, dom) {
    if (!dom.feedList) return;
    var payload = event.payload || {};
    var meta = event.meta || {};
    var kind = payload.kind || "event";

    var item = el("div", "live-feed-item");

    // Timestamp
    var ts = el("span", "live-feed-ts", formatTimestamp(meta.timestamp, state.startTime));
    item.appendChild(ts);

    // Colored dot
    var dot = el("span", "live-feed-dot");
    dot.style.backgroundColor = colorForKind(kind);
    item.appendChild(dot);

    // Description
    var desc = el("span", "live-feed-desc", describeEventCompact(payload));
    if (kind === ERROR) {
      desc.classList.add("live-feed-desc--error");
    }
    item.appendChild(desc);

    var isAtBottom = isScrolledToBottom(dom.feedList.parentElement);
    dom.feedList.appendChild(item);

    if (isAtBottom) {
      scrollToBottom(dom.feedList.parentElement);
    }
  }

  function describeEventCompact(payload) {
    if (!payload) return "event";
    var kind = payload.kind;
    switch (kind) {
      case OPERATION_START:
        return "Node " + payload.node_id + " (" + (payload.op_type || "?") + ") started";
      case OPERATION_END: {
        var verb = payload.success === false ? "FAILED" : "done";
        var dur = payload.duration_ms != null ? " [" + formatDuration(payload.duration_ms) + "]" : "";
        return "Node " + payload.node_id + " " + verb + dur;
      }
      case TOKEN:
        return truncate(payload.text || "", 60);
      case THOUGHT:
        return "thinking: " + truncate(payload.text || "", 50);
      case LLM_DONE:
        return "LLM done" + (payload.model ? " (" + payload.model + ")" : "");
      case TOOL_CALL:
        return "tool: " + (payload.name || "?") + "()";
      case TOOL_START:
        return "tool start: " + (payload.name || "?");
      case TOOL_END:
        return "tool end: " + (payload.name || "?");
      case ERROR:
        return "ERROR: " + (payload.message || "unknown");
      case USAGE:
      case TOKEN_USAGE:
        return "tokens: in=" + (payload.input_tokens || 0) + " out=" + (payload.output_tokens || 0);
      case SESSION_START:
        return "session started";
      case SESSION_END:
        return "session ended";
      default:
        return kind || "event";
    }
  }

  // ---------------------------------------------------------------------------
  // Metrics display
  // ---------------------------------------------------------------------------

  function createMetricsPanel(container, state) {
    var header = el("div", "live-panel-header", "Metrics");
    header.style.margin = "-12px -12px 0 -12px";
    header.style.padding = "6px 12px";
    container.appendChild(header);

    var grid = el("div", "live-metrics-grid");

    var cards = [
      { key: "duration", label: "Duration", color: COL.blue || "#58a6ff" },
      { key: "inputTokens", label: "Input Tokens", color: COL.purple || "#bc8cff" },
      { key: "outputTokens", label: "Output Tokens", color: COL.purple || "#bc8cff" },
      { key: "nodesCompleted", label: "Nodes Done", color: COL.green || "#3fb950" },
      { key: "nodesFailed", label: "Failures", color: COL.red || "#f85149" },
      { key: "status", label: "Status", color: COL.blue || "#58a6ff" },
    ];

    var refs = {};

    cards.forEach(function (c) {
      var card = el("div", "live-metric-card");
      var label = el("div", "live-metric-label", c.label);
      card.appendChild(label);

      var value = el("div", "live-metric-value", "--");
      value.style.color = c.color;
      card.appendChild(value);

      refs[c.key] = value;
      grid.appendChild(card);
    });

    container.appendChild(grid);

    // Node summary strip
    var summaryLabel = el("div", "live-metric-label", "Node Status");
    summaryLabel.style.marginTop = "8px";
    container.appendChild(summaryLabel);

    var summary = el("div", "live-node-summary");
    container.appendChild(summary);

    return {
      values: refs,
      summary: summary,
    };
  }

  function updateMetricsDisplay(state, dom) {
    if (!dom.metricValues) return;

    var elapsed = "--";
    if (state.startTime) {
      elapsed = formatDuration(nowMs() - state.startTime.getTime());
    }

    if (dom.metricValues.duration) {
      dom.metricValues.duration.textContent = elapsed;
    }
    if (dom.metricValues.inputTokens) {
      dom.metricValues.inputTokens.textContent = state.totalTokens.input.toLocaleString();
    }
    if (dom.metricValues.outputTokens) {
      dom.metricValues.outputTokens.textContent = state.totalTokens.output.toLocaleString();
    }
    if (dom.metricValues.nodesCompleted) {
      dom.metricValues.nodesCompleted.textContent = String(state.nodesCompleted);
    }
    if (dom.metricValues.nodesFailed) {
      dom.metricValues.nodesFailed.textContent = String(state.nodesFailed);
      dom.metricValues.nodesFailed.style.color = state.nodesFailed > 0 ? STATUS_COLORS.failed : STATUS_COLORS.pending;
    }
    if (dom.metricValues.status) {
      dom.metricValues.status.textContent = state.sessionStatus;
      dom.metricValues.status.style.color =
        state.sessionStatus === "completed" ? STATUS_COLORS.completed :
        state.sessionStatus === "failed" ? STATUS_COLORS.failed :
        STATUS_COLORS.running;
    }

    // Update node summary pills
    updateNodeSummary(state, dom);
  }

  function updateNodeSummary(state, dom) {
    if (!dom.nodeSummary) return;
    dom.nodeSummary.innerHTML = "";

    var nodeIds = Object.keys(state.nodeStates).sort(function (a, b) {
      return Number(a) - Number(b);
    });

    nodeIds.forEach(function (nid) {
      var ns = state.nodeStates[nid];
      var pill = el("span", "live-node-pill", "#" + nid);
      pill.style.background = statusBg(ns.status);
      pill.style.color = STATUS_COLORS[ns.status] || STATUS_COLORS.pending;
      dom.nodeSummary.appendChild(pill);
    });
  }

  function statusBg(status) {
    switch (status) {
      case "running": return "rgba(88, 166, 255, 0.12)";
      case "completed": return "rgba(63, 185, 80, 0.12)";
      case "failed": return "rgba(248, 81, 73, 0.12)";
      default: return "rgba(139, 148, 158, 0.1)";
    }
  }

  // ---------------------------------------------------------------------------
  // Graph overlay
  // ---------------------------------------------------------------------------

  function updateGraphOverlay(graphContainer, nodeStates) {
    if (!graphContainer) return;
    var svg = graphContainer.querySelector("svg");
    if (!svg) return;

    var nodeEls = svg.querySelectorAll(".graph-node, [data-node-id]");
    for (var i = 0; i < nodeEls.length; i++) {
      var nodeEl = nodeEls[i];
      var nid = nodeEl.getAttribute("data-node-id");
      if (nid == null) continue;

      // Remove all status classes
      nodeEl.classList.remove("node-pending", "node-running", "node-completed", "node-failed");

      var ns = nodeStates[nid];
      if (ns) {
        nodeEl.classList.add("node-" + ns.status);

        // Add or update progress bar for running nodes
        updateNodeProgressBar(nodeEl, ns);

        // Add error icon for failed nodes
        updateNodeErrorIcon(nodeEl, ns);
      } else {
        nodeEl.classList.add("node-pending");
      }
    }
  }

  function updateNodeProgressBar(nodeEl, ns) {
    var existing = nodeEl.querySelector(".node-progress-group");
    if (ns.status === "running" && ns.startTime) {
      var elapsed = nowMs() - ns.startTime;
      // Estimate: show indeterminate progress that grows over time
      // Cap visual at 90% since we don't know true duration
      var pct = Math.min(90, (elapsed / 30000) * 100); // ~30s = 90%

      if (!existing) {
        var rect = nodeEl.querySelector("rect");
        if (!rect) return;
        var w = parseFloat(rect.getAttribute("width") || "180");
        var h = parseFloat(rect.getAttribute("height") || "50");

        var g = document.createElementNS(SVGNS, "g");
        g.setAttribute("class", "node-progress-group");

        var bgBar = document.createElementNS(SVGNS, "rect");
        bgBar.setAttribute("x", "2");
        bgBar.setAttribute("y", String(h - 4));
        bgBar.setAttribute("width", String(w - 4));
        bgBar.setAttribute("height", "3");
        bgBar.setAttribute("rx", "1.5");
        bgBar.setAttribute("class", "node-progress-bar");
        g.appendChild(bgBar);

        var fillBar = document.createElementNS(SVGNS, "rect");
        fillBar.setAttribute("x", "2");
        fillBar.setAttribute("y", String(h - 4));
        fillBar.setAttribute("width", String((w - 4) * pct / 100));
        fillBar.setAttribute("height", "3");
        fillBar.setAttribute("rx", "1.5");
        fillBar.setAttribute("class", "node-progress-fill");
        g.appendChild(fillBar);

        nodeEl.appendChild(g);
      } else {
        var fill = existing.querySelector(".node-progress-fill");
        if (fill) {
          var rectEl = nodeEl.querySelector("rect");
          var barW = parseFloat((rectEl && rectEl.getAttribute("width")) || "180") - 4;
          fill.setAttribute("width", String(barW * pct / 100));
        }
      }
    } else if (existing && ns.status !== "running") {
      existing.remove();
    }
  }

  function updateNodeErrorIcon(nodeEl, ns) {
    var existing = nodeEl.querySelector(".node-error-icon");
    if (ns.status === "failed" && !existing) {
      var rect = nodeEl.querySelector("rect");
      if (!rect) return;
      var w = parseFloat(rect.getAttribute("width") || "180");

      var icon = document.createElementNS(SVGNS, "text");
      icon.setAttribute("x", String(w - 14));
      icon.setAttribute("y", "16");
      icon.setAttribute("font-size", "14");
      icon.setAttribute("class", "node-error-icon");
      icon.textContent = "\u26A0";
      nodeEl.appendChild(icon);
    } else if (existing && ns.status !== "failed") {
      existing.remove();
    }
  }

  // ---------------------------------------------------------------------------
  // startLiveSession
  // ---------------------------------------------------------------------------

  function startLiveSession(container, sessionPath, graphData) {
    injectStyles();

    var state = createLiveState();
    var dom = createDomRefs();

    // Initialize node count from graph data
    if (graphData && graphData.nodes) {
      state.nodesTotal = graphData.nodes.length;
      graphData.nodes.forEach(function (node) {
        ensureNodeState(node.id, state);
        state.nodeStates[node.id].opType = node.op || "";
      });
    }

    // Clear container and build layout
    container.innerHTML = "";
    var dashboard = el("div", "live-dashboard");

    // 1. Live graph panel (top left)
    var graphPanel = el("div", "live-graph");
    dom.graphContainer = graphPanel;

    // Render the graph using GraphRenderer if available
    if (graphData && window.GraphRenderer && typeof window.GraphRenderer.renderGraph === "function") {
      window.GraphRenderer.renderGraph(graphPanel, graphData, function (nodeId) {
        state.focusedNode = nodeId;
        updateOutputTabs(state, dom);
        updateOutputPanel(state, dom);
      });
      // Apply initial pending state to all nodes
      updateGraphOverlay(graphPanel, state.nodeStates);
    } else if (graphData) {
      graphPanel.appendChild(el("div", "live-waiting", "Graph renderer not available"));
    } else {
      graphPanel.appendChild(el("div", "live-waiting", "No graph data provided"));
    }
    dashboard.appendChild(graphPanel);

    // 2. Live output panel (top right)
    var outputPanel = el("div", "live-output");
    var outputParts = createOutputPanel(outputPanel);
    dom.outputTabs = outputParts.tabs;
    dom.outputBody = outputParts.body;
    dashboard.appendChild(outputPanel);

    // 3. Event feed (bottom left)
    var feedPanel = el("div", "live-feed");
    var feedHeader = el("div", "live-panel-header", "Event Feed");
    feedPanel.appendChild(feedHeader);

    var feedBody = el("div", "live-panel-body");
    var feedList = el("div", "live-feed-list");
    feedBody.appendChild(feedList);
    feedPanel.appendChild(feedBody);
    dom.feedList = feedList;
    dashboard.appendChild(feedPanel);

    // 4. Metrics panel (bottom right)
    var metricsPanel = el("div", "live-metrics");
    var metricsParts = createMetricsPanel(metricsPanel, state);
    dom.metricValues = metricsParts.values;
    dom.nodeSummary = metricsParts.summary;
    dom.metricsPanel = metricsPanel;
    dashboard.appendChild(metricsPanel);

    container.appendChild(dashboard);

    // Set start time
    state.startTime = new Date();

    // Start duration ticker
    var durationInterval = setInterval(function () {
      if (dom.metricValues.duration && state.startTime) {
        dom.metricValues.duration.textContent = formatDuration(nowMs() - state.startTime.getTime());
      }
      // Update progress bars for running nodes
      Object.keys(state.nodeStates).forEach(function (nid) {
        if (state.nodeStates[nid].status === "running") {
          updateGraphOverlay(dom.graphContainer, state.nodeStates);
        }
      });
    }, 500);

    // Connect to SSE endpoint
    var endpoint = (C.api && C.api.liveSession) || "/api/live/session";
    var url = endpoint + "?path=" + encodeURIComponent(sessionPath);
    var eventSource = new EventSource(url);
    state.eventSource = eventSource;

    // Handle trace events
    eventSource.addEventListener("trace", function (e) {
      try {
        var event = JSON.parse(e.data);
        handleTraceEvent(event, state, dom);
      } catch (err) {
        console.error("Failed to parse trace event:", err, e.data);
      }
    });

    // Handle progress snapshots
    eventSource.addEventListener("progress", function (e) {
      try {
        var progress = JSON.parse(e.data);
        handleProgressUpdate(progress, state, dom);
      } catch (err) {
        console.error("Failed to parse progress event:", err);
      }
    });

    // Handle status updates
    eventSource.addEventListener("status", function (e) {
      try {
        var status = JSON.parse(e.data);
        handleStatusUpdate(status, state, dom);
      } catch (err) {
        console.error("Failed to parse status event:", err);
      }
    });

    // Handle SSE errors
    eventSource.onerror = function () {
      if (eventSource.readyState === EventSource.CLOSED) {
        clearInterval(durationInterval);
        state.sessionStatus = state.nodesFailed > 0 ? "failed" : "completed";
        updateMetricsDisplay(state, dom);
        showCompleteBanner(state, dom);
      }
    };

    // Return handle
    var handle = {
      state: state,
      dom: dom,
      _durationInterval: durationInterval,
      stop: function () {
        stopLiveSession(handle);
      },
    };

    return handle;
  }

  // ---------------------------------------------------------------------------
  // Progress and status handlers
  // ---------------------------------------------------------------------------

  function handleProgressUpdate(progress, state, dom) {
    // Progress snapshots contain node_statuses and potentially other fields
    if (progress.node_statuses) {
      Object.keys(progress.node_statuses).forEach(function (nid) {
        var statusVal = progress.node_statuses[nid];
        var ns = ensureNodeState(Number(nid), state);
        if (typeof statusVal === "string") {
          ns.status = statusVal;
        } else if (statusVal && typeof statusVal === "object" && statusVal.status) {
          ns.status = statusVal.status;
        }
      });
      updateGraphOverlay(dom.graphContainer, state.nodeStates);
      updateNodeSummary(state, dom);
    }

    if (progress.completed_count != null) {
      state.nodesCompleted = progress.completed_count;
    }
    if (progress.failed_count != null) {
      state.nodesFailed = progress.failed_count;
    }

    updateMetricsDisplay(state, dom);
  }

  function handleStatusUpdate(status, state, dom) {
    if (status.status) {
      state.sessionStatus = status.status;
    }
    if (status.status === "completed" || status.status === "failed") {
      showCompleteBanner(state, dom);
    }
    updateMetricsDisplay(state, dom);
  }

  // ---------------------------------------------------------------------------
  // stopLiveSession
  // ---------------------------------------------------------------------------

  function stopLiveSession(handle) {
    if (!handle) return;

    if (handle.state.eventSource) {
      handle.state.eventSource.close();
      handle.state.eventSource = null;
    }

    if (handle._durationInterval) {
      clearInterval(handle._durationInterval);
      handle._durationInterval = null;
    }

    handle.state.sessionStatus = handle.state.nodesFailed > 0 ? "failed" : "completed";
    updateMetricsDisplay(handle.state, handle.dom);
    showCompleteBanner(handle.state, handle.dom);
  }

  // ---------------------------------------------------------------------------
  // Completion banner
  // ---------------------------------------------------------------------------

  function showCompleteBanner(state, dom) {
    if (!dom.metricsPanel) return;
    // Avoid duplicates
    if (dom.metricsPanel.querySelector(".live-complete-banner")) return;

    var failed = state.nodesFailed > 0;
    var banner = el("div",
      "live-complete-banner" + (failed ? " live-complete-banner--failed" : ""),
      failed
        ? "Session ended with " + state.nodesFailed + " failure(s)"
        : "Session completed successfully"
    );

    // Add summary info
    var elapsed = "--";
    if (state.startTime) {
      elapsed = formatDuration(nowMs() - state.startTime.getTime());
    }
    var summary = el("div");
    summary.style.fontSize = "11px";
    summary.style.fontWeight = "400";
    summary.style.marginTop = "4px";
    summary.style.color = "var(--text-secondary, #8b949e)";
    summary.textContent =
      "Duration: " + elapsed +
      " | Nodes: " + state.nodesCompleted + " done" +
      (state.nodesFailed > 0 ? ", " + state.nodesFailed + " failed" : "") +
      " | Tokens: " + state.totalTokens.input.toLocaleString() + " in / " +
      state.totalTokens.output.toLocaleString() + " out";
    banner.appendChild(summary);

    dom.metricsPanel.appendChild(banner);
  }

  // ---------------------------------------------------------------------------
  // Public API
  // ---------------------------------------------------------------------------

  window.LiveViewer = {
    startLiveSession: startLiveSession,
    handleTraceEvent: handleTraceEvent,
    updateGraphOverlay: updateGraphOverlay,
    createOutputPanel: createOutputPanel,
    stopLiveSession: stopLiveSession,
  };
})();
