// gantt.js -- APXM execution timeline Gantt chart renderer
// Renders parallel execution spans as a horizontal SVG Gantt chart.
// Vanilla JS + SVG, no dependencies.

(function () {
  "use strict";

  // Shared constants from window.APXM (with fallbacks for standalone use)
  var C = window.APXM || {};

  var SVGNS = "http://www.w3.org/2000/svg";

  // Layout constants
  var BAR_HEIGHT = 24;
  var ROW_HEIGHT = 32;
  var LABEL_WIDTH = 200;
  var AXIS_HEIGHT = 32;
  var PARALLELISM_HEIGHT = 60;
  var PADDING_TOP = 8;
  var PADDING_RIGHT = 24;
  var PADDING_BOTTOM = 8;
  var MIN_CHART_WIDTH = 400;

  // ---------------------------------------------------------------------------
  // Helpers — delegate to shared APXM constants with inline fallbacks
  // ---------------------------------------------------------------------------

  function getOpCategory(opType) {
    return (C.getOpCategory || function () { return "constants"; })(opType);
  }

  function getBarColor(opType) {
    var cat = getOpCategory(opType);
    return (C.getCategoryStroke || function () { return "#6e7681"; })(cat);
  }

  function getBarFill(opType) {
    var cat = getOpCategory(opType);
    return (C.getCategoryFill || function () { return "#2a2a2a"; })(cat);
  }

  /** Resolve a theme color from C.colors, falling back to a default value. */
  function themeColor(key, fallback) {
    return (C.colors && C.colors[key]) || fallback;
  }

  function svgEl(tag, attrs) {
    var el = document.createElementNS(SVGNS, tag);
    if (attrs) {
      for (var key in attrs) {
        if (attrs.hasOwnProperty(key)) {
          el.setAttribute(key, attrs[key]);
        }
      }
    }
    return el;
  }

  function htmlEl(tag, className, text) {
    var node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = text;
    return node;
  }

  /**
   * Format a duration in milliseconds to a concise label.
   */
  function formatDuration(ms) {
    if (ms == null) return "--";
    if (ms < 1) return "<1ms";
    if (ms < 1000) return Math.round(ms) + "ms";
    if (ms < 60000) {
      var seconds = ms / 1000;
      return (seconds % 1 === 0 ? seconds.toFixed(0) : seconds.toFixed(1)) + "s";
    }
    var minutes = Math.floor(ms / 60000);
    var secs = Math.round((ms % 60000) / 1000);
    return minutes + "m " + secs + "s";
  }

  /**
   * Format a time value for axis labels.
   */
  function formatTimeLabel(ms) {
    if (ms < 1000) return Math.round(ms) + "ms";
    if (ms < 60000) {
      var s = ms / 1000;
      return (s % 1 === 0 ? s.toFixed(0) : s.toFixed(1)) + "s";
    }
    var min = Math.floor(ms / 60000);
    var sec = Math.round((ms % 60000) / 1000);
    if (sec === 0) return min + "m";
    return min + "m " + sec + "s";
  }

  // ---------------------------------------------------------------------------
  // Span extraction from trace events
  // ---------------------------------------------------------------------------

  /**
   * Build execution spans from trace events.
   *
   * Trace events have the structure:
   *   { meta: { timestamp, seq, ... }, payload: { kind: "operation_start"|"operation_end", node_id, op_type, ... } }
   *
   * Returns an array of span objects sorted by startMs.
   */
  function buildSpans(trace, nodeNames) {
    var openSpans = {}; // keyed by node_id
    var spans = [];
    var baseTime = null;

    // Find the first timestamp as the base
    for (var i = 0; i < trace.length; i++) {
      var meta = trace[i].meta || trace[i];
      var ts = meta.timestamp || trace[i].timestamp;
      if (ts) {
        baseTime = new Date(ts).getTime();
        break;
      }
    }

    if (baseTime == null) return spans;

    for (var j = 0; j < trace.length; j++) {
      var event = trace[j];
      var payload = event.payload || event;
      var eventMeta = event.meta || event;
      var kind = payload.kind;
      var nodeId = payload.node_id;

      if (nodeId == null) continue;

      var eventTime = eventMeta.timestamp || event.timestamp;
      if (!eventTime) continue;
      var eventMs = new Date(eventTime).getTime() - baseTime;

      if (kind === "operation_start") {
        var nameInfo = nodeNames && nodeNames[nodeId];
        var spanName = nameInfo ? (nameInfo.name || "node-" + nodeId) : "node-" + nodeId;
        var spanOp = payload.op_type || (nameInfo && nameInfo.op_type) || "?";

        openSpans[nodeId] = {
          nodeId: nodeId,
          name: spanName,
          op: spanOp,
          startMs: eventMs,
          endMs: null,
          success: true,
        };
      } else if (kind === "operation_end") {
        var open = openSpans[nodeId];
        if (open) {
          open.endMs = eventMs;
          open.success = payload.success !== false;
          if (payload.duration_ms != null && open.startMs != null) {
            // Use the reported duration if available for accuracy
            open.endMs = open.startMs + payload.duration_ms;
          }
          spans.push(open);
          delete openSpans[nodeId];
        } else {
          // End event without start -- synthesize from duration if possible
          var durMs = payload.duration_ms || 0;
          var syntheticStart = eventMs - durMs;
          var ni = nodeNames && nodeNames[nodeId];
          spans.push({
            nodeId: nodeId,
            name: ni ? (ni.name || "node-" + nodeId) : "node-" + nodeId,
            op: payload.op_type || (ni && ni.op_type) || "?",
            startMs: syntheticStart >= 0 ? syntheticStart : 0,
            endMs: eventMs,
            success: payload.success !== false,
          });
        }
      }
    }

    // Any still-open spans are "running"
    for (var nid in openSpans) {
      if (openSpans.hasOwnProperty(nid)) {
        spans.push(openSpans[nid]);
      }
    }

    // Sort by startMs, then by nodeId
    spans.sort(function (a, b) {
      if (a.startMs !== b.startMs) return a.startMs - b.startMs;
      return a.nodeId - b.nodeId;
    });

    return spans;
  }

  // ---------------------------------------------------------------------------
  // Time axis tick calculation
  // ---------------------------------------------------------------------------

  /**
   * Compute adaptive tick intervals based on total duration.
   * Returns { intervalMs, format } where format is a function.
   */
  function computeTickInterval(totalDurationMs) {
    // Choose a nice interval so we get roughly 5-15 ticks
    var candidates = [
      10, 20, 50, 100, 200, 500,
      1000, 2000, 5000, 10000, 15000, 30000,
      60000, 120000, 300000, 600000,
    ];

    var target = totalDurationMs / 8;
    var best = candidates[0];
    for (var i = 0; i < candidates.length; i++) {
      if (Math.abs(candidates[i] - target) < Math.abs(best - target)) {
        best = candidates[i];
      }
    }

    return best;
  }

  // ---------------------------------------------------------------------------
  // Internal state per chart instance
  // ---------------------------------------------------------------------------

  var GANTT_DATA_KEY = "__ganttData";

  function getGanttData(container) {
    return container[GANTT_DATA_KEY] || null;
  }

  function setGanttData(container, data) {
    container[GANTT_DATA_KEY] = data;
  }

  // ---------------------------------------------------------------------------
  // Tooltip management
  // ---------------------------------------------------------------------------

  var activeTooltip = null;

  function showTooltip(svgElement, x, y, html) {
    hideTooltip();
    var tip = document.createElement("div");
    tip.className = "gantt-tooltip";
    tip.innerHTML = html;

    // Position relative to the document
    var rect = svgElement.getBoundingClientRect();
    tip.style.position = "fixed";
    tip.style.left = (rect.left + x + 12) + "px";
    tip.style.top = (rect.top + y - 8) + "px";
    tip.style.zIndex = "9999";
    tip.style.pointerEvents = "none";
    tip.style.background = themeColor("tooltipBg", "#1c2128");
    tip.style.border = "1px solid " + themeColor("border", "#30363d");
    tip.style.borderRadius = "6px";
    tip.style.padding = "8px 12px";
    tip.style.color = themeColor("text", "#c9d1d9");
    tip.style.fontSize = "12px";
    tip.style.fontFamily = "sans-serif";
    tip.style.lineHeight = "1.5";
    tip.style.boxShadow = "0 4px 12px rgba(0,0,0,0.4)";
    tip.style.whiteSpace = "nowrap";
    tip.style.maxWidth = "350px";

    document.body.appendChild(tip);
    activeTooltip = tip;

    // Clamp to viewport
    var tipRect = tip.getBoundingClientRect();
    if (tipRect.right > window.innerWidth - 8) {
      tip.style.left = (rect.left + x - tipRect.width - 12) + "px";
    }
    if (tipRect.bottom > window.innerHeight - 8) {
      tip.style.top = (rect.top + y - tipRect.height - 8) + "px";
    }
  }

  function hideTooltip() {
    if (activeTooltip && activeTooltip.parentNode) {
      activeTooltip.parentNode.removeChild(activeTooltip);
    }
    activeTooltip = null;
  }

  function escHtml(str) {
    var div = document.createElement("div");
    div.textContent = str;
    return div.innerHTML;
  }

  // ---------------------------------------------------------------------------
  // renderBar
  // ---------------------------------------------------------------------------

  /**
   * Draw a single execution bar in the SVG.
   *
   * @param {SVGElement} svg - The root SVG element (for tooltip coordinates)
   * @param {SVGGElement} parentG - Group to append the bar to
   * @param {Object} span - { nodeId, name, op, startMs, endMs, success }
   * @param {number} yPos - Y position for this bar
   * @param {Function} timeScale - Maps ms to x pixel position
   * @param {Object} barColors - { fill, stroke } colors for this bar
   * @param {Object} opts - { onNodeClick, chartWidth, isLive }
   * @returns {SVGGElement} The bar group element
   */
  function renderBar(svg, parentG, span, yPos, timeScale, barColors, opts) {
    opts = opts || {};
    var isRunning = span.endMs == null;
    var startX = timeScale(span.startMs);
    var endX = isRunning ? (opts.chartWidth || timeScale(span.startMs + 1000)) : timeScale(span.endMs);
    var barWidth = Math.max(endX - startX, 2); // minimum visible width
    var barY = yPos + (ROW_HEIGHT - BAR_HEIGHT) / 2;

    var g = svgEl("g", {
      "class": "gantt-bar" + (isRunning ? " gantt-bar--running" : ""),
      "data-node-id": String(span.nodeId),
    });
    g.style.cursor = "pointer";

    // Bar background
    var rect = svgEl("rect", {
      x: String(startX),
      y: String(barY),
      width: String(barWidth),
      height: String(BAR_HEIGHT),
      rx: "4",
      ry: "4",
      fill: barColors.fill,
      stroke: barColors.stroke,
      "stroke-width": "1.5",
    });
    g.appendChild(rect);

    // Failure hatching overlay
    if (!span.success && !isRunning) {
      // Add a hatched pattern overlay
      var hatchId = "gantt-hatch-" + span.nodeId;
      var defs = svg.querySelector("defs");
      if (defs) {
        var pattern = svgEl("pattern", {
          id: hatchId,
          patternUnits: "userSpaceOnUse",
          width: "8",
          height: "8",
          patternTransform: "rotate(45)",
        });
        var hatchLine = svgEl("line", {
          x1: "0", y1: "0", x2: "0", y2: "8",
          stroke: themeColor("error", "#f85149"),
          "stroke-width": "2",
          "stroke-opacity": "0.5",
        });
        pattern.appendChild(hatchLine);
        defs.appendChild(pattern);

        var hatchRect = svgEl("rect", {
          x: String(startX),
          y: String(barY),
          width: String(barWidth),
          height: String(BAR_HEIGHT),
          rx: "4",
          ry: "4",
          fill: "url(#" + hatchId + ")",
        });
        g.appendChild(hatchRect);
      }
    }

    // Running animation: pulsing right edge
    if (isRunning) {
      var animRect = svgEl("rect", {
        x: String(endX - 3),
        y: String(barY),
        width: "3",
        height: String(BAR_HEIGHT),
        fill: barColors.stroke,
        opacity: "0.8",
      });
      var animate = svgEl("animate", {
        attributeName: "opacity",
        values: "0.3;1;0.3",
        dur: "1.2s",
        repeatCount: "indefinite",
      });
      animRect.appendChild(animate);
      g.appendChild(animRect);
    }

    // Duration label
    var durationMs = isRunning ? null : (span.endMs - span.startMs);
    var durationText = isRunning ? "running..." : formatDuration(durationMs);
    var textColor = barColors.stroke;

    // Place label inside bar if it fits, otherwise to the right
    var labelInside = barWidth > 60;
    var labelX = labelInside ? (startX + barWidth / 2) : (startX + barWidth + 6);
    var labelAnchor = labelInside ? "middle" : "start";

    var label = svgEl("text", {
      x: String(labelX),
      y: String(barY + BAR_HEIGHT / 2 + 4),
      fill: labelInside ? themeColor("white", "#ffffff") : textColor,
      "font-size": "10",
      "font-family": "monospace",
      "text-anchor": labelAnchor,
      "pointer-events": "none",
      opacity: labelInside ? "0.9" : "0.7",
    });
    label.textContent = durationText;
    g.appendChild(label);

    // Tooltip on hover
    g.addEventListener("mouseenter", function (e) {
      var svgRect = svg.getBoundingClientRect();
      var mx = e.clientX - svgRect.left;
      var my = e.clientY - svgRect.top;

      var tooltipHtml =
        "<strong>" + escHtml(span.name) + "</strong> &mdash; " + escHtml(span.op) + "<br>" +
        "Node ID: " + span.nodeId + "<br>" +
        "Start: " + formatTimeLabel(span.startMs) + "<br>";
      if (!isRunning) {
        tooltipHtml += "End: " + formatTimeLabel(span.endMs) + "<br>";
        tooltipHtml += "Duration: " + formatDuration(durationMs) + "<br>";
      } else {
        tooltipHtml += "Status: <span style='color:" + themeColor("warning", "#d29922") + "'>running</span><br>";
      }
      if (!span.success && !isRunning) {
        tooltipHtml += "<span style='color:" + themeColor("error", "#f85149") + "'>Failed</span>";
      }
      showTooltip(svg, mx, my, tooltipHtml);
    });

    g.addEventListener("mousemove", function (e) {
      var svgRect = svg.getBoundingClientRect();
      if (activeTooltip) {
        activeTooltip.style.left = (e.clientX + 12) + "px";
        activeTooltip.style.top = (e.clientY - 8) + "px";
      }
    });

    g.addEventListener("mouseleave", function () {
      hideTooltip();
    });

    // Click handler
    if (opts.onNodeClick) {
      g.addEventListener("click", function (e) {
        e.stopPropagation();
        opts.onNodeClick(span.nodeId);
      });
    }

    parentG.appendChild(g);
    return g;
  }

  // ---------------------------------------------------------------------------
  // renderTimeAxis
  // ---------------------------------------------------------------------------

  /**
   * Render the bottom time axis with tick marks and labels.
   *
   * @param {SVGGElement} parentG - Group to append axis elements to
   * @param {number} totalDurationMs - Total duration to display
   * @param {number} chartWidth - Width of the chart area in pixels
   * @param {Function} timeScale - Maps ms to x pixel position
   * @param {boolean} isLive - If true, render a "Now" marker
   */
  function renderTimeAxis(parentG, totalDurationMs, chartWidth, timeScale, isLive) {
    var axisY = 0;

    // Axis line
    var line = svgEl("line", {
      x1: String(timeScale(0)),
      y1: String(axisY),
      x2: String(timeScale(totalDurationMs)),
      y2: String(axisY),
      stroke: themeColor("border", "#30363d"),
      "stroke-width": "1",
    });
    parentG.appendChild(line);

    // Ticks
    var intervalMs = computeTickInterval(totalDurationMs);
    var tickMs = 0;
    while (tickMs <= totalDurationMs) {
      var x = timeScale(tickMs);

      // Tick mark
      var tick = svgEl("line", {
        x1: String(x),
        y1: String(axisY),
        x2: String(x),
        y2: String(axisY + 6),
        stroke: themeColor("subtleBorder", "#484f58"),
        "stroke-width": "1",
      });
      parentG.appendChild(tick);

      // Label
      var tickLabel = svgEl("text", {
        x: String(x),
        y: String(axisY + 20),
        fill: themeColor("muted", "#8b949e"),
        "font-size": "10",
        "font-family": "monospace",
        "text-anchor": "middle",
      });
      tickLabel.textContent = formatTimeLabel(tickMs);
      parentG.appendChild(tickLabel);

      tickMs += intervalMs;
    }

    // "Now" marker for live sessions
    if (isLive) {
      var nowX = timeScale(totalDurationMs);
      var nowLine = svgEl("line", {
        x1: String(nowX),
        y1: String(axisY - 2000), // extend through the whole chart
        x2: String(nowX),
        y2: String(axisY),
        stroke: themeColor("warning", "#d29922"),
        "stroke-width": "1.5",
        "stroke-dasharray": "4,3",
        opacity: "0.8",
      });
      parentG.appendChild(nowLine);

      var nowLabel = svgEl("text", {
        x: String(nowX),
        y: String(axisY + 20),
        fill: themeColor("warning", "#d29922"),
        "font-size": "10",
        "font-family": "monospace",
        "text-anchor": "middle",
        "font-weight": "bold",
      });
      nowLabel.textContent = "Now";
      parentG.appendChild(nowLabel);

      // Animated pulse
      var pulse = svgEl("circle", {
        cx: String(nowX),
        cy: String(axisY),
        r: "3",
        fill: themeColor("warning", "#d29922"),
      });
      var pulseAnim = svgEl("animate", {
        attributeName: "r",
        values: "3;6;3",
        dur: "1.5s",
        repeatCount: "indefinite",
      });
      pulse.appendChild(pulseAnim);
      var pulseOpacity = svgEl("animate", {
        attributeName: "opacity",
        values: "1;0.3;1",
        dur: "1.5s",
        repeatCount: "indefinite",
      });
      pulse.appendChild(pulseOpacity);
      parentG.appendChild(pulse);
    }
  }

  // ---------------------------------------------------------------------------
  // renderParallelismOverlay
  // ---------------------------------------------------------------------------

  /**
   * Render a parallelism-over-time line chart below the Gantt bars.
   *
   * @param {SVGGElement} parentG - Group to append to
   * @param {Array} spans - Array of span objects
   * @param {Function} timeScale - Maps ms to x pixel position
   * @param {number} totalDurationMs - Total chart duration
   */
  function renderParallelismOverlay(parentG, spans, timeScale, totalDurationMs) {
    if (spans.length === 0) return;

    // Collect all start/end events and sort by time
    var events = [];
    for (var i = 0; i < spans.length; i++) {
      events.push({ time: spans[i].startMs, delta: 1 });
      if (spans[i].endMs != null) {
        events.push({ time: spans[i].endMs, delta: -1 });
      }
    }
    events.sort(function (a, b) {
      if (a.time !== b.time) return a.time - b.time;
      // Process ends before starts at same timestamp
      return a.delta - b.delta;
    });

    // Build the parallelism timeline
    var points = []; // { time, level }
    var level = 0;
    var peakLevel = 0;

    for (var j = 0; j < events.length; j++) {
      // Record level before change at this time
      if (points.length === 0 || points[points.length - 1].time !== events[j].time) {
        points.push({ time: events[j].time, level: level });
      }
      level += events[j].delta;
      if (level < 0) level = 0;
      if (level > peakLevel) peakLevel = level;
      // Update or add the point at this time with the new level
      if (points.length > 0 && points[points.length - 1].time === events[j].time) {
        points[points.length - 1].level = level;
      } else {
        points.push({ time: events[j].time, level: level });
      }
    }

    // Add final point at the end
    points.push({ time: totalDurationMs, level: 0 });

    if (peakLevel === 0) return;

    var overlayHeight = PARALLELISM_HEIGHT;
    var yScale = function (lvl) {
      return overlayHeight - (lvl / peakLevel) * (overlayHeight - 8);
    };

    // Build step-function path for the filled area
    var areaPath = "M " + timeScale(0) + " " + overlayHeight;
    var linePath = "M";
    var prevX = timeScale(0);

    for (var k = 0; k < points.length; k++) {
      var px = timeScale(points[k].time);
      var py = yScale(points[k].level);

      // Step function: horizontal then vertical
      if (k > 0) {
        areaPath += " L " + px + " " + yScale(points[k - 1].level);
        linePath += " L " + px + " " + yScale(points[k - 1].level);
      }
      areaPath += " L " + px + " " + py;
      linePath += (k === 0 ? " " : " L ") + px + " " + py;
    }

    areaPath += " L " + timeScale(totalDurationMs) + " " + overlayHeight + " Z";

    // Shaded area
    var area = svgEl("path", {
      d: areaPath,
      fill: themeColor("accent", "#58a6ff"),
      "fill-opacity": "0.1",
    });
    parentG.appendChild(area);

    // Line
    var line = svgEl("path", {
      d: linePath,
      fill: "none",
      stroke: themeColor("accent", "#58a6ff"),
      "stroke-width": "1.5",
      opacity: "0.6",
    });
    parentG.appendChild(line);

    // Y-axis labels for parallelism
    for (var lv = 0; lv <= peakLevel; lv++) {
      var ly = yScale(lv);
      var lvLabel = svgEl("text", {
        x: String(timeScale(0) - 8),
        y: String(ly + 3),
        fill: themeColor("dimmed", "#6e7681"),
        "font-size": "9",
        "font-family": "monospace",
        "text-anchor": "end",
      });
      lvLabel.textContent = String(lv);
      parentG.appendChild(lvLabel);

      // Grid line
      if (lv > 0) {
        var gridLine = svgEl("line", {
          x1: String(timeScale(0)),
          y1: String(ly),
          x2: String(timeScale(totalDurationMs)),
          y2: String(ly),
          stroke: themeColor("gridLine", "#21262d"),
          "stroke-width": "0.5",
          "stroke-dasharray": "3,3",
        });
        parentG.appendChild(gridLine);
      }
    }

    // Peak label
    var peakX = timeScale(0);
    for (var m = 0; m < points.length; m++) {
      if (points[m].level === peakLevel) {
        peakX = timeScale(points[m].time);
        break;
      }
    }

    var peakLabel = svgEl("text", {
      x: String(peakX + 4),
      y: String(yScale(peakLevel) - 4),
      fill: themeColor("accent", "#58a6ff"),
      "font-size": "10",
      "font-family": "sans-serif",
      "font-weight": "bold",
      opacity: "0.8",
    });
    peakLabel.textContent = "Peak: " + peakLevel;
    parentG.appendChild(peakLabel);

    // Section label
    var sectionLabel = svgEl("text", {
      x: String(timeScale(0)),
      y: String(-4),
      fill: themeColor("muted", "#8b949e"),
      "font-size": "10",
      "font-family": "sans-serif",
    });
    sectionLabel.textContent = "Parallelism";
    parentG.appendChild(sectionLabel);
  }

  // ---------------------------------------------------------------------------
  // renderGantt (main entry point)
  // ---------------------------------------------------------------------------

  /**
   * Render a Gantt chart inside the given container.
   *
   * @param {HTMLElement} container - DOM element to render into
   * @param {Array} trace - Array of trace events (ApxmEvent objects)
   * @param {Object} nodeNames - Map of nodeId -> { name, op_type }
   * @param {Object} [options] - { onNodeClick, showParallelism, isLive }
   */
  function renderGantt(container, trace, nodeNames, options) {
    options = options || {};
    var onNodeClick = options.onNodeClick || null;
    var showParallelism = options.showParallelism !== false; // default true
    var isLive = options.isLive || false;

    // Clear container
    while (container.firstChild) {
      container.removeChild(container.firstChild);
    }

    if (!trace || trace.length === 0) {
      container.appendChild(htmlEl("div", "gantt-empty", "No trace events to display."));
      return;
    }

    // Build spans from trace
    var spans = buildSpans(trace, nodeNames);
    if (spans.length === 0) {
      container.appendChild(htmlEl("div", "gantt-empty", "No execution spans found in trace."));
      return;
    }

    // Compute total duration
    var maxEndMs = 0;
    for (var i = 0; i < spans.length; i++) {
      var end = spans[i].endMs != null ? spans[i].endMs : spans[i].startMs + 1000;
      if (end > maxEndMs) maxEndMs = end;
    }
    var totalDurationMs = maxEndMs || 1000;

    // Compute chart dimensions
    var containerWidth = container.clientWidth || 800;
    var chartWidth = Math.max(containerWidth - LABEL_WIDTH - PADDING_RIGHT, MIN_CHART_WIDTH);
    var chartAreaStartX = LABEL_WIDTH;
    var rowCount = spans.length;
    var barsHeight = rowCount * ROW_HEIGHT;
    var parallelismSectionHeight = showParallelism ? (PARALLELISM_HEIGHT + 24) : 0;
    var totalHeight = PADDING_TOP + barsHeight + AXIS_HEIGHT + parallelismSectionHeight + PADDING_BOTTOM;
    var svgWidth = chartAreaStartX + chartWidth + PADDING_RIGHT;

    // Time scale: maps ms to x pixel
    var timeScale = function (ms) {
      return chartAreaStartX + (ms / totalDurationMs) * chartWidth;
    };

    // Zoom/pan state
    var viewState = {
      rangeStartMs: 0,
      rangeEndMs: totalDurationMs,
      panStartX: null,
      panStartRange: null,
      dragging: false,
    };

    // Store data for external interaction
    setGanttData(container, {
      spans: spans,
      totalDurationMs: totalDurationMs,
      viewState: viewState,
      onNodeClick: onNodeClick,
      isLive: isLive,
      showParallelism: showParallelism,
      nodeNames: nodeNames,
      trace: trace,
    });

    // Build time scale from current view state
    function currentTimeScale(ms) {
      var range = viewState.rangeEndMs - viewState.rangeStartMs;
      if (range <= 0) range = 1;
      return chartAreaStartX + ((ms - viewState.rangeStartMs) / range) * chartWidth;
    }

    // Create SVG
    var svg = svgEl("svg", {
      width: String(svgWidth),
      height: String(totalHeight),
      viewBox: "0 0 " + svgWidth + " " + totalHeight,
    });
    svg.style.display = "block";
    svg.style.userSelect = "none";
    svg.style.background = "transparent";

    // Embedded styles
    var style = svgEl("style");
    style.textContent = [
      ".gantt-bar > rect { transition: filter 0.15s ease; }",
      ".gantt-bar:hover > rect { filter: brightness(1.3) drop-shadow(0 0 4px rgba(255,255,255,0.15)); }",
      ".gantt-bar.gantt-bar--selected > rect { stroke-width: 2.5; filter: drop-shadow(0 0 6px rgba(88,166,255,0.5)); }",
      ".gantt-bar.gantt-bar--highlighted > rect { stroke-width: 2.5; filter: drop-shadow(0 0 8px rgba(88,166,255,0.7)); }",
      ".gantt-row-label { cursor: pointer; }",
      ".gantt-row-label:hover { fill: " + themeColor("white", "#ffffff") + "; }",
    ].join("\n");
    svg.appendChild(style);

    // Defs for patterns
    var defs = svgEl("defs");
    svg.appendChild(defs);

    // Background clip rect for chart area
    var clipId = "gantt-clip-" + Math.random().toString(36).slice(2, 8);
    var clipPath = svgEl("clipPath", { id: clipId });
    var clipRect = svgEl("rect", {
      x: String(chartAreaStartX),
      y: "0",
      width: String(chartWidth),
      height: String(totalHeight),
    });
    clipPath.appendChild(clipRect);
    defs.appendChild(clipPath);

    // Root group
    var rootG = svgEl("g");
    svg.appendChild(rootG);

    // Time grid lines (vertical, behind bars)
    var gridG = svgEl("g", { "class": "gantt-grid", "clip-path": "url(#" + clipId + ")" });
    rootG.appendChild(gridG);

    var gridInterval = computeTickInterval(totalDurationMs);
    var gridMs = 0;
    while (gridMs <= totalDurationMs) {
      var gx = currentTimeScale(gridMs);
      var gridLine = svgEl("line", {
        x1: String(gx),
        y1: String(PADDING_TOP),
        x2: String(gx),
        y2: String(PADDING_TOP + barsHeight),
        stroke: themeColor("gridLine", "#21262d"),
        "stroke-width": "0.5",
      });
      gridG.appendChild(gridLine);
      gridMs += gridInterval;
    }

    // Row backgrounds (alternating)
    var rowBgG = svgEl("g", { "class": "gantt-row-backgrounds" });
    rootG.appendChild(rowBgG);

    for (var ri = 0; ri < rowCount; ri++) {
      if (ri % 2 === 1) {
        var rowBg = svgEl("rect", {
          x: "0",
          y: String(PADDING_TOP + ri * ROW_HEIGHT),
          width: String(svgWidth),
          height: String(ROW_HEIGHT),
          fill: themeColor("rowAlt", "#161b22"),
          opacity: "0.4",
        });
        rowBgG.appendChild(rowBg);
      }
    }

    // Row labels (left side)
    var labelsG = svgEl("g", { "class": "gantt-labels" });
    rootG.appendChild(labelsG);

    for (var li = 0; li < spans.length; li++) {
      var span = spans[li];
      var labelY = PADDING_TOP + li * ROW_HEIGHT + ROW_HEIGHT / 2;

      // Op type badge
      var opColor = getBarColor(span.op);
      var opBadge = svgEl("text", {
        x: "8",
        y: String(labelY - 2),
        fill: opColor,
        "font-size": "9",
        "font-family": "monospace",
        "font-weight": "bold",
        "class": "gantt-row-label",
        "data-node-id": String(span.nodeId),
      });
      opBadge.textContent = span.op;
      labelsG.appendChild(opBadge);

      // Node name
      var displayName = span.name;
      if (displayName.length > 20) {
        displayName = displayName.slice(0, 18) + "\u2026";
      }
      var nameLabel = svgEl("text", {
        x: "8",
        y: String(labelY + 10),
        fill: themeColor("muted", "#8b949e"),
        "font-size": "11",
        "font-family": "sans-serif",
        "class": "gantt-row-label",
        "data-node-id": String(span.nodeId),
      });
      nameLabel.textContent = displayName;
      labelsG.appendChild(nameLabel);

      // Click on label selects the node
      if (onNodeClick) {
        (function (nodeId) {
          opBadge.addEventListener("click", function () { onNodeClick(nodeId); });
          nameLabel.addEventListener("click", function () { onNodeClick(nodeId); });
        })(span.nodeId);
      }
    }

    // Separator line between labels and chart
    var separator = svgEl("line", {
      x1: String(chartAreaStartX - 8),
      y1: String(PADDING_TOP),
      x2: String(chartAreaStartX - 8),
      y2: String(PADDING_TOP + barsHeight),
      stroke: themeColor("border", "#30363d"),
      "stroke-width": "1",
    });
    rootG.appendChild(separator);

    // Bars group (clipped to chart area)
    var barsG = svgEl("g", { "class": "gantt-bars", "clip-path": "url(#" + clipId + ")" });
    rootG.appendChild(barsG);

    for (var bi = 0; bi < spans.length; bi++) {
      var barSpan = spans[bi];
      var barY = PADDING_TOP + bi * ROW_HEIGHT;
      var barColor = getBarColor(barSpan.op);
      var barFill = getBarFill(barSpan.op);

      renderBar(svg, barsG, barSpan, barY, currentTimeScale, {
        fill: barFill,
        stroke: barColor,
      }, {
        onNodeClick: onNodeClick,
        chartWidth: chartAreaStartX + chartWidth,
        isLive: isLive,
      });
    }

    // Time axis
    var axisG = svgEl("g", {
      "class": "gantt-axis",
      transform: "translate(0," + (PADDING_TOP + barsHeight) + ")",
    });
    rootG.appendChild(axisG);
    renderTimeAxis(axisG, totalDurationMs, chartWidth, currentTimeScale, isLive);

    // Parallelism overlay
    if (showParallelism && spans.length > 1) {
      var parallelismY = PADDING_TOP + barsHeight + AXIS_HEIGHT;
      var parallelismG = svgEl("g", {
        "class": "gantt-parallelism",
        transform: "translate(0," + parallelismY + ")",
      });
      rootG.appendChild(parallelismG);
      renderParallelismOverlay(parallelismG, spans, currentTimeScale, totalDurationMs);
    }

    // ---------------------------------------------------------------------------
    // Interactions: zoom (wheel) and pan (drag)
    // ---------------------------------------------------------------------------

    svg.addEventListener("wheel", function (e) {
      e.preventDefault();

      var rect = svg.getBoundingClientRect();
      var mx = e.clientX - rect.left;

      // Only zoom if mouse is over the chart area
      if (mx < chartAreaStartX) return;

      var factor = e.deltaY < 0 ? 0.85 : 1.18;
      var range = viewState.rangeEndMs - viewState.rangeStartMs;
      var newRange = range * factor;

      // Clamp to [1ms, totalDurationMs * 1.5]
      if (newRange < 1) newRange = 1;
      if (newRange > totalDurationMs * 1.5) newRange = totalDurationMs * 1.5;

      // Zoom centered on mouse position
      var mouseRatio = (mx - chartAreaStartX) / chartWidth;
      var mouseTimeMs = viewState.rangeStartMs + range * mouseRatio;
      var newStart = mouseTimeMs - newRange * mouseRatio;
      var newEnd = newStart + newRange;

      // Clamp
      if (newStart < -totalDurationMs * 0.1) newStart = -totalDurationMs * 0.1;
      if (newEnd > totalDurationMs * 1.1) newEnd = totalDurationMs * 1.1;

      // Animate zoom
      animateZoom(container, newStart, newEnd);
    }, { passive: false });

    // Pan with drag
    svg.addEventListener("mousedown", function (e) {
      var rect = svg.getBoundingClientRect();
      var mx = e.clientX - rect.left;
      if (mx < chartAreaStartX) return;

      // Only on background, not on bars
      if (e.target.closest && e.target.closest(".gantt-bar")) return;

      viewState.dragging = true;
      viewState.panStartX = e.clientX;
      viewState.panStartRange = {
        start: viewState.rangeStartMs,
        end: viewState.rangeEndMs,
      };
      svg.style.cursor = "grabbing";
    });

    svg.addEventListener("mousemove", function (e) {
      if (!viewState.dragging) return;
      var dx = e.clientX - viewState.panStartX;
      var range = viewState.panStartRange.end - viewState.panStartRange.start;
      var timeDelta = -(dx / chartWidth) * range;

      viewState.rangeStartMs = viewState.panStartRange.start + timeDelta;
      viewState.rangeEndMs = viewState.panStartRange.end + timeDelta;

      rebuildChart(container);
    });

    function stopDrag() {
      if (viewState.dragging) {
        viewState.dragging = false;
        svg.style.cursor = "";
      }
    }

    svg.addEventListener("mouseup", stopDrag);
    svg.addEventListener("mouseleave", function () {
      stopDrag();
      hideTooltip();
    });

    container.appendChild(svg);
  }

  // ---------------------------------------------------------------------------
  // Rebuild chart (after zoom/pan)
  // ---------------------------------------------------------------------------

  function rebuildChart(container) {
    var data = getGanttData(container);
    if (!data) return;

    var vs = data.viewState;

    // Re-render by calling renderGantt but preserving viewState
    var savedView = {
      rangeStartMs: vs.rangeStartMs,
      rangeEndMs: vs.rangeEndMs,
    };

    renderGantt(container, data.trace, data.nodeNames, {
      onNodeClick: data.onNodeClick,
      showParallelism: data.showParallelism,
      isLive: data.isLive,
    });

    // Restore view
    var newData = getGanttData(container);
    if (newData) {
      newData.viewState.rangeStartMs = savedView.rangeStartMs;
      newData.viewState.rangeEndMs = savedView.rangeEndMs;

      // Re-render with the restored zoom
      renderGanttWithView(container, newData);
    }
  }

  /**
   * Internal: re-render the chart SVG contents with the current viewState.
   * Rebuilds the bars and axis at the current zoom level without recreating
   * the entire DOM.
   */
  function renderGanttWithView(container, data) {
    var svg = container.querySelector("svg");
    if (!svg) return;

    var vs = data.viewState;
    var spans = data.spans;
    var totalDurationMs = data.totalDurationMs;

    // Recalculate chart dimensions
    var containerWidth = container.clientWidth || 800;
    var chartWidth = Math.max(containerWidth - LABEL_WIDTH - PADDING_RIGHT, MIN_CHART_WIDTH);
    var chartAreaStartX = LABEL_WIDTH;
    var rowCount = spans.length;
    var barsHeight = rowCount * ROW_HEIGHT;

    var range = vs.rangeEndMs - vs.rangeStartMs;
    if (range <= 0) range = 1;

    var currentTimeScale = function (ms) {
      return chartAreaStartX + ((ms - vs.rangeStartMs) / range) * chartWidth;
    };

    // Update grid lines
    var gridG = svg.querySelector(".gantt-grid");
    if (gridG) {
      while (gridG.firstChild) gridG.removeChild(gridG.firstChild);

      var viewDuration = vs.rangeEndMs - vs.rangeStartMs;
      var gridInterval = computeTickInterval(viewDuration);
      var gridStart = Math.floor(vs.rangeStartMs / gridInterval) * gridInterval;
      var gridMs = gridStart;
      while (gridMs <= vs.rangeEndMs) {
        if (gridMs >= vs.rangeStartMs) {
          var gx = currentTimeScale(gridMs);
          var gridLine = svgEl("line", {
            x1: String(gx),
            y1: String(PADDING_TOP),
            x2: String(gx),
            y2: String(PADDING_TOP + barsHeight),
            stroke: themeColor("gridLine", "#21262d"),
            "stroke-width": "0.5",
          });
          gridG.appendChild(gridLine);
        }
        gridMs += gridInterval;
      }
    }

    // Update bars
    var barsG = svg.querySelector(".gantt-bars");
    if (barsG) {
      while (barsG.firstChild) barsG.removeChild(barsG.firstChild);

      for (var bi = 0; bi < spans.length; bi++) {
        var barSpan = spans[bi];
        var barY = PADDING_TOP + bi * ROW_HEIGHT;
        var barColor = getBarColor(barSpan.op);
        var barFill = getBarFill(barSpan.op);

        renderBar(svg, barsG, barSpan, barY, currentTimeScale, {
          fill: barFill,
          stroke: barColor,
        }, {
          onNodeClick: data.onNodeClick,
          chartWidth: chartAreaStartX + chartWidth,
          isLive: data.isLive,
        });
      }
    }

    // Update axis
    var axisG = svg.querySelector(".gantt-axis");
    if (axisG) {
      while (axisG.firstChild) axisG.removeChild(axisG.firstChild);
      renderTimeAxis(axisG, viewDuration || totalDurationMs, chartWidth, currentTimeScale, data.isLive);
    }

    // Update parallelism overlay
    var parallelismG = svg.querySelector(".gantt-parallelism");
    if (parallelismG) {
      while (parallelismG.firstChild) parallelismG.removeChild(parallelismG.firstChild);
      renderParallelismOverlay(parallelismG, spans, currentTimeScale, totalDurationMs);
    }
  }

  // ---------------------------------------------------------------------------
  // zoomToRange
  // ---------------------------------------------------------------------------

  /**
   * Zoom the timeline to a specific time range with smooth animation.
   *
   * @param {HTMLElement} container - The Gantt chart container
   * @param {number} startMs - Start of the time range in milliseconds
   * @param {number} endMs - End of the time range in milliseconds
   */
  function zoomToRange(container, startMs, endMs) {
    animateZoom(container, startMs, endMs);
  }

  /**
   * Animate a zoom transition over ~300ms.
   */
  function animateZoom(container, targetStartMs, targetEndMs) {
    var data = getGanttData(container);
    if (!data) return;

    var vs = data.viewState;
    var fromStart = vs.rangeStartMs;
    var fromEnd = vs.rangeEndMs;
    var duration = 250; // ms
    var startTime = null;

    function step(timestamp) {
      if (startTime === null) startTime = timestamp;
      var elapsed = timestamp - startTime;
      var t = Math.min(elapsed / duration, 1);
      // Ease out cubic
      var ease = 1 - Math.pow(1 - t, 3);

      vs.rangeStartMs = fromStart + (targetStartMs - fromStart) * ease;
      vs.rangeEndMs = fromEnd + (targetEndMs - fromEnd) * ease;

      renderGanttWithView(container, data);

      if (t < 1) {
        requestAnimationFrame(step);
      }
    }

    requestAnimationFrame(step);
  }

  // ---------------------------------------------------------------------------
  // highlightNode
  // ---------------------------------------------------------------------------

  /**
   * Highlight a specific node's bar in the Gantt chart.
   *
   * @param {HTMLElement} container - The Gantt chart container
   * @param {number} nodeId - The node ID to highlight
   */
  function highlightNode(container, nodeId) {
    var svg = container.querySelector("svg");
    if (!svg) return;

    // Remove existing highlights
    var highlighted = svg.querySelectorAll(".gantt-bar--highlighted");
    for (var i = 0; i < highlighted.length; i++) {
      highlighted[i].classList.remove("gantt-bar--highlighted");
    }

    // Remove existing selections
    var selected = svg.querySelectorAll(".gantt-bar--selected");
    for (var j = 0; j < selected.length; j++) {
      selected[j].classList.remove("gantt-bar--selected");
    }

    // Find and highlight the target bar
    var bars = svg.querySelectorAll(".gantt-bar");
    for (var k = 0; k < bars.length; k++) {
      if (bars[k].getAttribute("data-node-id") === String(nodeId)) {
        bars[k].classList.add("gantt-bar--highlighted");
        break;
      }
    }
  }

  // ---------------------------------------------------------------------------
  // addLiveSpan / completeLiveSpan
  // ---------------------------------------------------------------------------

  /**
   * For live mode: add a new bar that grows in real-time.
   *
   * @param {HTMLElement} container - The Gantt chart container
   * @param {number} nodeId - The node ID
   * @param {number} startTime - Start time as epoch ms or relative ms
   */
  function addLiveSpan(container, nodeId, startTime) {
    var data = getGanttData(container);
    if (!data) return;

    // Calculate relative start time
    var relativeStartMs = startTime;
    if (startTime > 1000000000000) {
      // Epoch timestamp -- make relative to trace start
      var traceStart = 0;
      if (data.trace && data.trace.length > 0) {
        var firstMeta = data.trace[0].meta || data.trace[0];
        var ts = firstMeta.timestamp || data.trace[0].timestamp;
        if (ts) traceStart = new Date(ts).getTime();
      }
      relativeStartMs = startTime - traceStart;
    }

    var nameInfo = data.nodeNames && data.nodeNames[nodeId];
    var newSpan = {
      nodeId: nodeId,
      name: nameInfo ? (nameInfo.name || "node-" + nodeId) : "node-" + nodeId,
      op: nameInfo ? (nameInfo.op_type || "?") : "?",
      startMs: relativeStartMs,
      endMs: null,
      success: true,
    };

    data.spans.push(newSpan);

    // Update total duration if needed
    if (relativeStartMs + 1000 > data.totalDurationMs) {
      data.totalDurationMs = relativeStartMs + 1000;
    }

    // Re-render
    renderGantt(container, data.trace, data.nodeNames, {
      onNodeClick: data.onNodeClick,
      showParallelism: data.showParallelism,
      isLive: true,
    });
  }

  /**
   * Finalize a live span.
   *
   * @param {HTMLElement} container - The Gantt chart container
   * @param {number} nodeId - The node ID
   * @param {number} endTime - End time as epoch ms or relative ms
   * @param {boolean} success - Whether the operation succeeded
   */
  function completeLiveSpan(container, nodeId, endTime, success) {
    var data = getGanttData(container);
    if (!data) return;

    // Calculate relative end time
    var relativeEndMs = endTime;
    if (endTime > 1000000000000) {
      var traceStart = 0;
      if (data.trace && data.trace.length > 0) {
        var firstMeta = data.trace[0].meta || data.trace[0];
        var ts = firstMeta.timestamp || data.trace[0].timestamp;
        if (ts) traceStart = new Date(ts).getTime();
      }
      relativeEndMs = endTime - traceStart;
    }

    // Find and update the span
    for (var i = 0; i < data.spans.length; i++) {
      if (data.spans[i].nodeId === nodeId && data.spans[i].endMs == null) {
        data.spans[i].endMs = relativeEndMs;
        data.spans[i].success = success !== false;
        break;
      }
    }

    // Update total duration
    if (relativeEndMs > data.totalDurationMs) {
      data.totalDurationMs = relativeEndMs;
    }

    // Check if any spans are still running
    var stillLive = false;
    for (var j = 0; j < data.spans.length; j++) {
      if (data.spans[j].endMs == null) {
        stillLive = true;
        break;
      }
    }

    // Re-render
    renderGantt(container, data.trace, data.nodeNames, {
      onNodeClick: data.onNodeClick,
      showParallelism: data.showParallelism,
      isLive: stillLive,
    });
  }

  // ---------------------------------------------------------------------------
  // Export
  // ---------------------------------------------------------------------------

  window.GanttChart = {
    renderGantt: renderGantt,
    renderBar: renderBar,
    renderTimeAxis: renderTimeAxis,
    renderParallelismOverlay: renderParallelismOverlay,
    addLiveSpan: addLiveSpan,
    completeLiveSpan: completeLiveSpan,
    zoomToRange: zoomToRange,
    highlightNode: highlightNode,
  };
})();
