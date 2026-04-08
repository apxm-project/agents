// session.js -- APXM Session Viewer
// Renders session execution data: timeline, node statuses, metrics.

(function () {
  "use strict";

  // Shared constants (populated by constants.js when loaded before this file)
  var C = window.APXM || {};
  var COL = C.colors || {};

  // Event kind shortcuts -- prefer shared constants, fall back to literals
  var EK = C.eventKinds || {};
  var OPERATION_START = EK.operationStart || "operation_start";
  var OPERATION_END   = EK.operationEnd   || "operation_end";
  var TOKEN           = EK.token          || "token";
  var THOUGHT         = EK.thought        || "thought";
  var LLM_DONE        = EK.llmDone        || "llm_done";
  var TOOL_START      = EK.toolStart      || "tool_start";
  var TOOL_END        = EK.toolEnd        || "tool_end";
  var TOOL_CALL       = EK.toolCall       || "tool_call";
  var ERROR           = EK.error          || "error";
  var SESSION_START   = EK.sessionStart   || "session_start";
  var SESSION_END     = EK.sessionEnd     || "session_end";

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  /**
   * Format a duration in milliseconds to a human-readable string.
   * Delegates to shared APXM.formatDuration when available.
   */
  function formatDuration(ms) {
    if (C.formatDuration) return C.formatDuration(ms);
    if (ms == null) return "--";
    if (ms < 1000) return ms + "ms";
    if (ms < 60000) {
      var seconds = ms / 1000;
      return (seconds % 1 === 0 ? seconds.toFixed(0) : seconds.toFixed(1)) + "s";
    }
    var minutes = Math.floor(ms / 60000);
    var secs = Math.round((ms % 60000) / 1000);
    return minutes + "m " + secs + "s";
  }

  /**
   * Compute a relative timestamp string from an ISO string and a base Date.
   * Returns "+0.000s", "+1.234s", etc.
   */
  function formatTimestamp(isoString, baseTime) {
    if (!isoString || !baseTime) return "";
    var t = new Date(isoString).getTime();
    var base = baseTime instanceof Date ? baseTime.getTime() : baseTime;
    var delta = (t - base) / 1000;
    if (delta < 0) delta = 0;
    return "+" + delta.toFixed(3) + "s";
  }

  /**
   * Truncate a string to maxLen characters, appending "..." if truncated.
   * Delegates to shared APXM.truncate when available.
   */
  function truncate(str, maxLen) {
    if (C.truncate) return C.truncate(str, maxLen);
    if (str == null) return "";
    if (typeof str !== "string") str = String(str);
    if (str.length <= maxLen) return str;
    return str.slice(0, maxLen) + "...";
  }

  /**
   * Create a DOM element with optional className and text content.
   */
  function el(tag, className, text) {
    var node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = text;
    return node;
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
      case TOOL_START:
      case TOOL_END:
      case TOOL_CALL:
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

  /**
   * Build a human-readable description for a trace event payload.
   */
  function describeEvent(payload) {
    if (!payload) return "Unknown event";
    var kind = payload.kind;
    switch (kind) {
      case OPERATION_START:
        return "Node " + payload.node_id + " (" + (payload.op_type || "?") + ") started";
      case OPERATION_END: {
        var verb = payload.success === false ? "failed" : "completed";
        var dur = payload.duration_ms != null ? " in " + formatDuration(payload.duration_ms) : "";
        return "Node " + payload.node_id + " (" + (payload.op_type || "?") + ") " + verb + dur;
      }
      case TOKEN:
        return truncate(payload.text || "", 50);
      case LLM_DONE: {
        var model = payload.model ? " (" + payload.model + ")" : "";
        var content = payload.content ? ": " + truncate(payload.content, 50) : "";
        return "LLM response" + model + content;
      }
      case TOOL_CALL: {
        var name = payload.name || "?";
        var args = payload.args ? "(" + truncate(JSON.stringify(payload.args), 40) + ")" : "()";
        return "Tool call: " + name + args;
      }
      case ERROR:
        return "Error: " + (payload.message || "unknown");
      case SESSION_START:
        return "Session started";
      case SESSION_END:
        return "Session ended";
      default:
        return kind || "event";
    }
  }

  /**
   * Determine the node_id associated with a trace event, if any.
   */
  function nodeIdFromEvent(event) {
    var p = event && event.payload;
    if (!p) return null;
    if (p.node_id != null) return p.node_id;
    return null;
  }

  // ---------------------------------------------------------------------------
  // Metric Cards
  // ---------------------------------------------------------------------------

  /**
   * Render metric cards as a horizontal flex row inside `container`.
   */
  function renderMetrics(container, metrics) {
    container.innerHTML = "";
    if (!metrics) return;

    var row = el("div", "metric-row");

    // Status color helpers -- prefer shared nodeStatus colors, fall back to literals
    var NS = C.nodeStatus || {};
    var runningColor   = (NS.running   && NS.running.color)   || COL.blue          || "#58a6ff";
    var completedColor = (NS.completed && NS.completed.color) || COL.green         || "#3fb950";
    var failedColor    = (NS.failed    && NS.failed.color)    || COL.red           || "#f85149";
    var pendingColor   = (NS.pending   && NS.pending.color)   || COL.textSecondary || "#8b949e";
    var llmColor       = colorForKind(TOKEN); // purple family

    var cards = [
      {
        label: "Duration",
        value: formatDuration(metrics.total_duration_ms),
        color: runningColor,
      },
      {
        label: "Input Tokens",
        value: metrics.total_input_tokens != null ? metrics.total_input_tokens.toLocaleString() : "--",
        color: llmColor,
      },
      {
        label: "Output Tokens",
        value: metrics.total_output_tokens != null ? metrics.total_output_tokens.toLocaleString() : "--",
        color: llmColor,
      },
      {
        label: "Nodes Run",
        value: metrics.nodes_executed != null ? metrics.nodes_executed : "--",
        color: completedColor,
      },
      {
        label: "Failures",
        value: metrics.nodes_failed != null ? metrics.nodes_failed : "--",
        color: metrics.nodes_failed > 0 ? failedColor : pendingColor,
      },
    ];

    cards.forEach(function (c) {
      var card = el("div", "metric-card");

      var label = el("div", "metric-label", c.label);
      card.appendChild(label);

      var value = el("div", "metric-value", String(c.value));
      value.style.color = c.color;
      card.appendChild(value);

      row.appendChild(card);
    });

    container.appendChild(row);
  }

  // ---------------------------------------------------------------------------
  // Timeline
  // ---------------------------------------------------------------------------

  /**
   * Group trace events into operation spans. Token/thought/llm_done events
   * between an operation_start and operation_end are grouped together and
   * rendered as a collapsible block.
   *
   * Returns an array of items, each being either:
   *   { type: "event", event: <traceEvent> }
   *   { type: "group", startEvent: <traceEvent>, endEvent: <traceEvent>|null, children: [<traceEvent>, ...] }
   */
  function groupTraceEvents(trace) {
    var items = [];
    var i = 0;

    while (i < trace.length) {
      var ev = trace[i];
      var payload = ev.payload || {};

      if (payload.kind === OPERATION_START) {
        var group = {
          type: "group",
          startEvent: ev,
          endEvent: null,
          children: [],
        };
        var nodeId = payload.node_id;
        i++;

        // Collect children until we hit the matching operation_end
        while (i < trace.length) {
          var inner = trace[i];
          var innerPayload = inner.payload || {};

          if (
            innerPayload.kind === OPERATION_END &&
            innerPayload.node_id === nodeId
          ) {
            group.endEvent = inner;
            i++;
            break;
          }

          // Another operation_start means nested — just treat as standalone
          if (innerPayload.kind === OPERATION_START) {
            break;
          }

          group.children.push(inner);
          i++;
        }

        items.push(group);
      } else {
        items.push({ type: "event", event: ev });
        i++;
      }
    }

    return items;
  }

  /**
   * Render a single timeline event row.
   */
  function renderTimelineEvent(event, baseTime, onNodeClick) {
    var row = el("div", "timeline-event");
    var payload = event.payload || {};
    var meta = event.meta || {};

    // Timestamp
    var ts = el("span", "timeline-timestamp", formatTimestamp(meta.timestamp, baseTime));
    row.appendChild(ts);

    // Colored dot
    var dot = el("span", "timeline-dot");
    dot.style.backgroundColor = colorForKind(payload.kind);
    row.appendChild(dot);

    // Description
    var desc = el("span", "timeline-description", describeEvent(payload));

    // Failed operation_end gets special styling
    if (payload.kind === OPERATION_END && payload.success === false) {
      desc.classList.add("timeline-description--failed");
    }

    row.appendChild(desc);

    // Clickable if event has a node_id
    var nid = nodeIdFromEvent(event);
    if (nid != null && onNodeClick) {
      row.classList.add("timeline-event--clickable");
      row.addEventListener("click", function () {
        onNodeClick(nid);
      });
    }

    return row;
  }

  /**
   * Render the full timeline into `container`.
   */
  function renderTimeline(container, trace, onNodeClick) {
    container.innerHTML = "";
    if (!trace || trace.length === 0) {
      container.appendChild(el("div", "timeline-empty", "No trace events."));
      return;
    }

    var list = el("div", "timeline-list");

    // Determine base time from the first event
    var firstMeta = trace[0].meta || {};
    var baseTime = firstMeta.timestamp ? new Date(firstMeta.timestamp) : null;

    var groups = groupTraceEvents(trace);

    groups.forEach(function (item) {
      if (item.type === "event") {
        list.appendChild(renderTimelineEvent(item.event, baseTime, onNodeClick));
        return;
      }

      // Grouped operation span
      var group = item;
      var startPayload = group.startEvent.payload || {};

      // Render the start event
      list.appendChild(renderTimelineEvent(group.startEvent, baseTime, onNodeClick));

      // If there are children (token events, etc.), render a collapsible block
      if (group.children.length > 0) {
        var toggleRow = el("div", "timeline-token-group");
        var toggleBtn = el("button", "timeline-token-toggle");
        toggleBtn.textContent = group.children.length + " token" + (group.children.length !== 1 ? "s" : "") + "...";
        toggleBtn.setAttribute("aria-expanded", "false");

        var childrenContainer = el("div", "timeline-token-children");
        childrenContainer.style.display = "none";

        group.children.forEach(function (childEv) {
          childrenContainer.appendChild(renderTimelineEvent(childEv, baseTime, onNodeClick));
        });

        toggleBtn.addEventListener("click", function () {
          var expanded = toggleBtn.getAttribute("aria-expanded") === "true";
          if (expanded) {
            childrenContainer.style.display = "none";
            toggleBtn.setAttribute("aria-expanded", "false");
            toggleBtn.textContent = group.children.length + " token" + (group.children.length !== 1 ? "s" : "") + "...";
          } else {
            childrenContainer.style.display = "";
            toggleBtn.setAttribute("aria-expanded", "true");
            toggleBtn.textContent = "Collapse tokens";
          }
        });

        toggleRow.appendChild(toggleBtn);
        list.appendChild(toggleRow);
        list.appendChild(childrenContainer);
      }

      // Render the end event if present
      if (group.endEvent) {
        list.appendChild(renderTimelineEvent(group.endEvent, baseTime, onNodeClick));
      }
    });

    container.appendChild(list);
  }

  // ---------------------------------------------------------------------------
  // Node Statuses
  // ---------------------------------------------------------------------------

  /**
   * Return the CSS modifier class for a node status string.
   * Uses shared APXM.nodeStatus keys when available, falls back to literals.
   */
  function statusBadgeClass(status) {
    // If shared constants provide a CSS class for this status, use it
    var NS = C.nodeStatus || {};
    if (NS[status] && NS[status].badgeClass) return NS[status].badgeClass;
    switch (status) {
      case "completed":
        return "session-status-badge--completed";
      case "failed":
        return "session-status-badge--failed";
      case "running":
        return "session-status-badge--running";
      default:
        return "session-status-badge--pending";
    }
  }

  /**
   * Render a grid of small status cards, one per node.
   */
  function renderNodeStatuses(container, nodeStatuses, onNodeClick) {
    container.innerHTML = "";
    if (!nodeStatuses) return;

    var grid = el("div", "session-status-grid");

    var nodeIds = Object.keys(nodeStatuses).sort(function (a, b) {
      return Number(a) - Number(b);
    });

    nodeIds.forEach(function (id) {
      var status = nodeStatuses[id];
      var card = el("div", "session-status-card");

      var idLabel = el("span", "session-status-id", "Node " + id);
      card.appendChild(idLabel);

      var badge = el("span", "session-status-badge " + statusBadgeClass(status), status);
      card.appendChild(badge);

      if (onNodeClick) {
        card.classList.add("session-status-card--clickable");
        card.addEventListener("click", function () {
          onNodeClick(Number(id));
        });
      }

      grid.appendChild(card);
    });

    container.appendChild(grid);
  }

  // ---------------------------------------------------------------------------
  // Node Detail
  // ---------------------------------------------------------------------------

  /**
   * Render detail view for a specific node: its output and filtered trace events.
   */
  function renderNodeDetail(container, nodeId, results, trace) {
    container.innerHTML = "";

    var header = el("h3", "session-detail-header", "Node " + nodeId);
    container.appendChild(header);

    // Output section
    var outputSection = el("div", "session-detail-section");
    var outputLabel = el("div", "session-detail-label", "Output");
    outputSection.appendChild(outputLabel);

    var nodeResult = results && results[String(nodeId)];
    if (nodeResult) {
      var outputText =
        typeof nodeResult.output === "string"
          ? nodeResult.output
          : JSON.stringify(nodeResult.output, null, 2);

      if (outputText && outputText.length > 120) {
        var pre = el("pre", "session-detail-code");
        var code = el("code", null, outputText);
        pre.appendChild(code);
        outputSection.appendChild(pre);
      } else {
        var outputDiv = el("div", "session-detail-output", outputText || "(no output)");
        outputSection.appendChild(outputDiv);
      }
    } else {
      outputSection.appendChild(el("div", "session-detail-output", "(no result)"));
    }

    container.appendChild(outputSection);

    // Filtered trace events for this node
    var traceSection = el("div", "session-detail-section");
    var traceLabel = el("div", "session-detail-label", "Trace Events");
    traceSection.appendChild(traceLabel);

    if (trace && trace.length > 0) {
      var filtered = trace.filter(function (ev) {
        return nodeIdFromEvent(ev) === nodeId;
      });

      if (filtered.length > 0) {
        var baseTime = trace[0].meta ? new Date(trace[0].meta.timestamp) : null;
        var traceList = el("div", "timeline-list");

        filtered.forEach(function (ev) {
          traceList.appendChild(renderTimelineEvent(ev, baseTime, null));
        });

        traceSection.appendChild(traceList);
      } else {
        traceSection.appendChild(el("div", "session-detail-output", "No events for this node."));
      }
    } else {
      traceSection.appendChild(el("div", "session-detail-output", "No trace data available."));
    }

    container.appendChild(traceSection);
  }

  // ---------------------------------------------------------------------------
  // Full Session View
  // ---------------------------------------------------------------------------

  /**
   * Render the complete session view layout inside `container`.
   *
   * Layout:
   *   - Metrics cards row at top
   *   - Node status grid below metrics
   *   - Two-column body:
   *       Left (60%): Timeline
   *       Right (40%): Node detail (appears when a node is selected)
   */
  function renderSession(container, sessionData) {
    container.innerHTML = "";
    if (!sessionData) {
      container.appendChild(el("div", "session-empty", "No session data."));
      return;
    }

    var manifest = sessionData.manifest || {};
    var trace = sessionData.trace || [];
    var results = sessionData.results || {};
    var metrics = sessionData.metrics || {};
    var nodeStatuses = sessionData.node_statuses || {};

    // Session header
    var headerRow = el("div", "session-header");
    var title = el("h2", "session-title", manifest.graph_name || "Session");

    var statusBadge = el(
      "span",
      "session-status-badge " + statusBadgeClass(manifest.status),
      manifest.status || "unknown"
    );
    title.appendChild(document.createTextNode(" "));
    title.appendChild(statusBadge);
    headerRow.appendChild(title);

    if (manifest.execution_id) {
      var execId = el("div", "session-exec-id", "ID: " + manifest.execution_id);
      headerRow.appendChild(execId);
    }

    container.appendChild(headerRow);

    // Metrics row
    var metricsContainer = el("div", "session-metrics");
    renderMetrics(metricsContainer, metrics);
    container.appendChild(metricsContainer);

    // Node statuses row
    var statusContainer = el("div", "session-statuses");
    var statusLabel = el("h3", "session-section-title", "Nodes");
    statusContainer.appendChild(statusLabel);

    var statusGrid = el("div");
    statusContainer.appendChild(statusGrid);
    container.appendChild(statusContainer);

    // Two-column body
    var body = el("div", "session-body");

    var timelinePanel = el("div", "session-timeline-panel");
    var timelineLabel = el("h3", "session-section-title", "Timeline");
    timelinePanel.appendChild(timelineLabel);

    var timelineContent = el("div", "session-timeline-content");
    timelinePanel.appendChild(timelineContent);
    body.appendChild(timelinePanel);

    var detailPanel = el("div", "session-detail-panel");
    var detailContent = el("div", "session-detail-content");
    var detailPlaceholder = el("div", "session-detail-placeholder", "Select a node to view details.");
    detailContent.appendChild(detailPlaceholder);
    detailPanel.appendChild(detailContent);
    body.appendChild(detailPanel);

    container.appendChild(body);

    // Wire up node click handler
    var selectedNodeId = null;

    function onNodeClick(nodeId) {
      selectedNodeId = nodeId;
      renderNodeDetail(detailContent, nodeId, results, trace);

      // Highlight the selected node in the status grid
      var cards = statusGrid.querySelectorAll(".session-status-card");
      cards.forEach(function (card) {
        card.classList.remove("session-status-card--selected");
      });
      var idx = Object.keys(nodeStatuses)
        .sort(function (a, b) { return Number(a) - Number(b); })
        .indexOf(String(nodeId));
      if (idx >= 0 && cards[idx]) {
        cards[idx].classList.add("session-status-card--selected");
      }
    }

    // Render status grid and timeline with the shared click handler
    renderNodeStatuses(statusGrid, nodeStatuses, onNodeClick);
    renderTimeline(timelineContent, trace, onNodeClick);
  }

  // ---------------------------------------------------------------------------
  // Public API
  // ---------------------------------------------------------------------------

  window.SessionViewer = {
    renderSession: renderSession,
    renderMetrics: renderMetrics,
    renderTimeline: renderTimeline,
    renderNodeStatuses: renderNodeStatuses,
    renderNodeDetail: renderNodeDetail,
    formatDuration: formatDuration,
    formatTimestamp: formatTimestamp,
    truncate: truncate,
  };
})();
