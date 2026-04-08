// ── APXM Compiler Optimization Viewer ────────────────────────────────
// Vanilla JS module for before/after comparison of graph optimization passes.
// Exports: window.CompilerViewer

(function () {
  "use strict";

  // ── Shared constants guard ───────────────────────────────────────────
  var C = window.APXM || {};

  // ── Local fallback maps (used when APXM constants are not loaded) ──

  var LOCAL_PASS_META = {
    prompt_caching: {
      label: "Prompt Caching",
      color: "#5c80d6",
      bg: "rgba(92, 128, 214, 0.16)",
      description:
        "Detects shared system prompts across reasoning nodes and marks them with cached_system_prompt to avoid redundant token usage.",
    },
    memoization_hints: {
      label: "Memoization Hints",
      color: "#58bb81",
      bg: "rgba(88, 187, 129, 0.14)",
      description:
        "Identifies duplicate pure operations and marks them as memoizable so the runtime can reuse prior results.",
    },
    pipeline_detection: {
      label: "Pipeline Detection",
      color: "#9f84e0",
      bg: "rgba(159, 132, 224, 0.14)",
      description:
        "Detects sequential ASK chains and marks them as pipeline_candidate for streaming optimizations.",
    },
  };

  var LOCAL_FALLBACK_PASS = {
    label: "Unknown Pass",
    color: "#8d99a7",
    bg: "rgba(141, 153, 167, 0.12)",
    description: "",
  };

  // ── Helpers ──────────────────────────────────────────────────────────

  function getPassMeta(passId) {
    // Prefer shared APXM.passes, fall back to local map
    if (C.passes && C.passes[passId]) return C.passes[passId];
    return LOCAL_PASS_META[passId] || null;
  }

  function passDisplayName(passId) {
    var meta = getPassMeta(passId);
    if (meta && meta.label) return meta.label;
    return passId.replace(/_/g, " ").replace(/\b\w/g, function (c) {
      return c.toUpperCase();
    });
  }

  function passColor(passId) {
    var meta = getPassMeta(passId);
    return (meta && meta.color) || (C.colors && C.colors.muted) || LOCAL_FALLBACK_PASS.color;
  }

  function passBg(passId) {
    var meta = getPassMeta(passId);
    return (meta && meta.bg) || LOCAL_FALLBACK_PASS.bg;
  }

  function passDescription(passId) {
    var meta = getPassMeta(passId);
    return (meta && meta.description) || LOCAL_FALLBACK_PASS.description;
  }

  function col(name, fallback) {
    return (C.colors && C.colors[name]) || fallback;
  }

  function el(tag, attrs, children) {
    var elem = document.createElement(tag);
    if (attrs) {
      Object.keys(attrs).forEach(function (key) {
        if (key === "className") {
          elem.className = attrs[key];
        } else if (key === "style" && typeof attrs[key] === "object") {
          Object.assign(elem.style, attrs[key]);
        } else if (key.indexOf("on") === 0) {
          elem.addEventListener(key.slice(2).toLowerCase(), attrs[key]);
        } else {
          elem.setAttribute(key, attrs[key]);
        }
      });
    }
    if (children != null) {
      if (typeof children === "string") {
        elem.textContent = children;
      } else if (Array.isArray(children)) {
        children.forEach(function (child) {
          if (child == null) return;
          if (typeof child === "string") {
            elem.appendChild(document.createTextNode(child));
          } else {
            elem.appendChild(child);
          }
        });
      } else {
        elem.appendChild(children);
      }
    }
    return elem;
  }

  function svgEl(tag, attrs) {
    var elem = document.createElementNS("http://www.w3.org/2000/svg", tag);
    if (attrs) {
      Object.keys(attrs).forEach(function (key) {
        if (key === "textContent") {
          elem.textContent = attrs[key];
        } else {
          elem.setAttribute(key, attrs[key]);
        }
      });
    }
    return elem;
  }

  function truncate(str, maxLen) {
    if (str.length <= maxLen) return str;
    return str.slice(0, maxLen - 1) + "\u2026";
  }

  function formatValue(val) {
    if (typeof val === "boolean") return val ? "true" : "false";
    if (typeof val === "string") return '"' + truncate(val, 40) + '"';
    return String(val);
  }

  // ── Category / op helpers (delegate to APXM when available) ────────

  function getOpCategory(op) {
    if (typeof C.getOpCategory === "function") return C.getOpCategory(op);
    // Minimal local fallback
    return "constants";
  }

  function getCategoryFill(category) {
    if (typeof C.getCategoryFill === "function") return C.getCategoryFill(category);
    return col("nodeBg", "rgba(18, 25, 35, 0.98)");
  }

  function getCategoryStroke(category) {
    if (typeof C.getCategoryStroke === "function") return C.getCategoryStroke(category);
    return col("muted", "#8d99a7");
  }

  // ── SVG Graph Renderer ──────────────────────────────────────────────
  // Simple layered layout for APXM graphs rendered as SVG.

  var NODE_W = 200;
  var NODE_H = 56;
  var H_GAP = 40;
  var V_GAP = 80;

  // Dependency edge colors (prefer APXM.colors.*)
  function edgeColor(dep) {
    switch (dep) {
      case "Data":    return col("data",    "#3ea6c6");
      case "Control": return col("control", "#d86b6b");
      case "Effect":  return col("effect",  "#d49b45");
      default:        return col("muted",   "#8d99a7");
    }
  }

  function computeLayout(graph) {
    // Topological layering via longest-path
    var nodeMap = {};
    graph.nodes.forEach(function (n) {
      nodeMap[n.id] = n;
    });

    var inDegree = {};
    var outEdges = {};
    graph.nodes.forEach(function (n) {
      inDegree[n.id] = 0;
      outEdges[n.id] = [];
    });
    graph.edges.forEach(function (e) {
      if (inDegree[e.to] !== undefined) {
        inDegree[e.to]++;
      }
      if (outEdges[e.from]) {
        outEdges[e.from].push(e.to);
      }
    });

    // BFS layers
    var layers = [];
    var nodeLayer = {};
    var queue = [];
    graph.nodes.forEach(function (n) {
      if (inDegree[n.id] === 0) {
        queue.push(n.id);
        nodeLayer[n.id] = 0;
      }
    });

    // Fallback: if no roots found (cycle), put all nodes in layer 0
    if (queue.length === 0) {
      graph.nodes.forEach(function (n) {
        queue.push(n.id);
        nodeLayer[n.id] = 0;
      });
    }

    while (queue.length > 0) {
      var id = queue.shift();
      var layer = nodeLayer[id];
      if (!layers[layer]) layers[layer] = [];
      layers[layer].push(id);

      (outEdges[id] || []).forEach(function (toId) {
        var nextLayer = layer + 1;
        if (nodeLayer[toId] === undefined || nodeLayer[toId] < nextLayer) {
          nodeLayer[toId] = nextLayer;
        }
        inDegree[toId]--;
        if (inDegree[toId] === 0) {
          queue.push(toId);
        }
      });
    }

    // Rebuild layers from nodeLayer (longest path may have moved nodes forward)
    layers = [];
    Object.keys(nodeLayer).forEach(function (nid) {
      var l = nodeLayer[nid];
      if (!layers[l]) layers[l] = [];
      layers[l].push(parseInt(nid, 10));
    });

    // Handle nodes not visited (disconnected)
    graph.nodes.forEach(function (n) {
      if (nodeLayer[n.id] === undefined) {
        nodeLayer[n.id] = 0;
        if (!layers[0]) layers[0] = [];
        layers[0].push(n.id);
      }
    });

    // Clean up sparse layers
    layers = layers.filter(function (l) { return l && l.length > 0; });

    // Compute positions
    var positions = {};
    var maxLayerWidth = 0;
    layers.forEach(function (layer) {
      if (layer.length > maxLayerWidth) maxLayerWidth = layer.length;
    });

    layers.forEach(function (layer, layerIdx) {
      var totalWidth = layer.length * NODE_W + (layer.length - 1) * H_GAP;
      var startX = (maxLayerWidth * (NODE_W + H_GAP) - H_GAP - totalWidth) / 2;
      layer.forEach(function (nid, posIdx) {
        positions[nid] = {
          x: startX + posIdx * (NODE_W + H_GAP),
          y: layerIdx * (NODE_H + V_GAP),
        };
      });
    });

    var svgWidth = maxLayerWidth * (NODE_W + H_GAP) - H_GAP + 80;
    var svgHeight = layers.length * (NODE_H + V_GAP) - V_GAP + 80;

    return { positions: positions, width: svgWidth, height: svgHeight };
  }

  function renderGraphSVG(graph, modifiedNodeIds, highlightNodeId) {
    var layout = computeLayout(graph);
    var positions = layout.positions;
    var padX = 40;
    var padY = 40;

    var svg = svgEl("svg", {
      width: "100%",
      height: "100%",
      viewBox: "0 0 " + (layout.width + padX) + " " + (layout.height + padY),
      preserveAspectRatio: "xMidYMid meet",
    });

    // Defs for arrowheads
    var defs = svgEl("defs");
    ["Data", "Control", "Effect"].forEach(function (dep) {
      var marker = svgEl("marker", {
        id: "arrow-" + dep,
        viewBox: "0 0 10 10",
        refX: "10",
        refY: "5",
        markerWidth: "8",
        markerHeight: "8",
        orient: "auto-start-reverse",
      });
      var path = svgEl("path", {
        d: "M 0 0 L 10 5 L 0 10 z",
        fill: edgeColor(dep),
      });
      marker.appendChild(path);
      defs.appendChild(marker);
    });
    svg.appendChild(defs);

    // Background grid dots
    var bgPattern = svgEl("pattern", {
      id: "grid-dots",
      width: "40",
      height: "40",
      patternUnits: "userSpaceOnUse",
    });
    bgPattern.appendChild(svgEl("circle", {
      cx: "20",
      cy: "20",
      r: "1",
      fill: col("gridDot", "rgba(148, 163, 184, 0.08)"),
    }));
    defs.appendChild(bgPattern);

    svg.appendChild(svgEl("rect", {
      width: "100%",
      height: "100%",
      fill: "url(#grid-dots)",
    }));

    // Render edges
    var edgeGroup = svgEl("g", { class: "compiler-edges" });
    graph.edges.forEach(function (edge) {
      var from = positions[edge.from];
      var to = positions[edge.to];
      if (!from || !to) return;

      var x1 = from.x + padX / 2 + NODE_W / 2;
      var y1 = from.y + padY / 2 + NODE_H;
      var x2 = to.x + padX / 2 + NODE_W / 2;
      var y2 = to.y + padY / 2;

      var midY = (y1 + y2) / 2;
      var pathD = "M " + x1 + " " + y1 + " C " + x1 + " " + midY + " " + x2 + " " + midY + " " + x2 + " " + y2;

      var dep = edge.dependency || "Data";
      var color = edgeColor(dep);
      var dashArray = dep === "Control" ? "8 5" : dep === "Effect" ? "3 3" : "none";

      var pathEl = svgEl("path", {
        d: pathD,
        stroke: color,
        "stroke-width": "1.8",
        fill: "none",
        "stroke-dasharray": dashArray,
        "marker-end": "url(#arrow-" + dep + ")",
      });
      edgeGroup.appendChild(pathEl);
    });
    svg.appendChild(edgeGroup);

    // Render nodes
    var nodeGroup = svgEl("g", { class: "compiler-nodes" });
    var modifiedSet = {};
    if (modifiedNodeIds) {
      modifiedNodeIds.forEach(function (nid) { modifiedSet[nid] = true; });
    }

    graph.nodes.forEach(function (node) {
      var pos = positions[node.id];
      if (!pos) return;

      var nx = pos.x + padX / 2;
      var ny = pos.y + padY / 2;
      var isModified = !!modifiedSet[node.id];
      var isHighlighted = highlightNodeId === node.id;

      // Determine colors from category via APXM helpers
      var cat = getOpCategory(node.op);
      var catFill = getCategoryFill(cat);
      var catStroke = getCategoryStroke(cat);

      var g = svgEl("g", {
        transform: "translate(" + nx + "," + ny + ")",
        "data-node-id": String(node.id),
      });

      // Node background
      var accentColor = col("accent", "#3ea6c6");
      var modifiedColor = col("effect", "#d49b45");
      var borderDefault = col("border", "rgba(133, 146, 166, 0.14)");

      var rect = svgEl("rect", {
        width: String(NODE_W),
        height: String(NODE_H),
        rx: "8",
        ry: "8",
        fill: catFill,
        stroke: isHighlighted
          ? accentColor
          : isModified
          ? modifiedColor
          : borderDefault,
        "stroke-width": isHighlighted ? "2.5" : isModified ? "2" : "1",
      });
      g.appendChild(rect);

      // Top accent bar
      g.appendChild(svgEl("rect", {
        width: String(NODE_W),
        height: "3",
        rx: "8",
        fill: isModified ? modifiedColor : catStroke,
      }));

      // Op label
      var textMuted = col("textMuted", "rgba(232, 238, 245, 0.5)");
      var opLabel = svgEl("text", {
        x: "10",
        y: "22",
        fill: textMuted,
        "font-size": "9",
        "font-weight": "700",
        "font-family": "'IBM Plex Mono', monospace",
        "letter-spacing": "0.08em",
        textContent: node.op,
      });
      g.appendChild(opLabel);

      // Node name
      var textPrimary = col("text", "#e8eef5");
      var nameLabel = svgEl("text", {
        x: "10",
        y: "40",
        fill: textPrimary,
        "font-size": "12",
        "font-weight": "700",
        "font-family": "'Manrope', 'Inter', sans-serif",
        textContent: truncate(node.name, 22),
      });
      g.appendChild(nameLabel);

      // ID badge
      var idLabel = svgEl("text", {
        x: String(NODE_W - 10),
        y: "22",
        fill: textMuted,
        "font-size": "9",
        "font-family": "'IBM Plex Mono', monospace",
        "text-anchor": "end",
        textContent: "#" + node.id,
      });
      g.appendChild(idLabel);

      // Modified indicator (gold dot)
      if (isModified) {
        g.appendChild(svgEl("circle", {
          cx: String(NODE_W - 10),
          cy: "38",
          r: "4",
          fill: modifiedColor,
        }));
      }

      nodeGroup.appendChild(g);
    });
    svg.appendChild(nodeGroup);

    return svg;
  }

  // ── renderGraph ─────────────────────────────────────────────────────
  // Uses window.GraphRenderer.renderGraph if available, otherwise our SVG.

  function renderGraph(container, graph, modifiedNodeIds, highlightNodeId) {
    container.innerHTML = "";

    if (window.GraphRenderer && typeof window.GraphRenderer.renderGraph === "function") {
      window.GraphRenderer.renderGraph(container, graph, {
        modifiedNodeIds: modifiedNodeIds,
        highlightNodeId: highlightNodeId,
      });
      return;
    }

    var wrapper = el("div", {
      className: "compiler-graph-svg-wrap",
      style: {
        width: "100%",
        height: "100%",
        overflow: "auto",
        position: "relative",
      },
    });
    wrapper.appendChild(renderGraphSVG(graph, modifiedNodeIds, highlightNodeId));
    container.appendChild(wrapper);
  }

  // ── renderStats ─────────────────────────────────────────────────────

  function renderStats(container, stats) {
    container.innerHTML = "";

    var bar = el("div", { className: "compiler-stats" });

    var items = [
      {
        label: "Nodes Modified",
        value: String(stats.nodes_modified),
        colorClass: "",
      },
      {
        label: "Attributes Added",
        value: String(stats.attributes_added),
        colorClass: "",
      },
      {
        label: "Est. Token Savings",
        value: stats.estimated_token_savings_pct.toFixed(1) + "%",
        colorClass: "compiler-stats__value--savings",
      },
    ];

    items.forEach(function (item) {
      var card = el("div", { className: "compiler-stats__card" }, [
        el("span", { className: "compiler-stats__label" }, item.label),
        el(
          "span",
          {
            className:
              "compiler-stats__value" +
              (item.colorClass ? " " + item.colorClass : ""),
          },
          item.value
        ),
      ]);
      bar.appendChild(card);
    });

    container.appendChild(bar);
  }

  // ── renderPassSelector ──────────────────────────────────────────────

  function renderPassSelector(container, passes, onOptimize) {
    container.innerHTML = "";

    var row = el("div", { className: "compiler-pass-selector" });

    var checkboxes = {};
    var checkboxList = el("div", { className: "compiler-pass-selector__list" });

    passes.forEach(function (passId) {
      var id = "pass-" + passId;
      var color = passColor(passId);
      var desc = passDescription(passId);

      var checkbox = el("input", {
        type: "checkbox",
        id: id,
        className: "compiler-pass-selector__checkbox",
      });
      checkbox.checked = true;
      checkboxes[passId] = checkbox;

      var pill = el(
        "span",
        {
          className: "compiler-pass-pill",
          style: {
            background: passBg(passId),
            color: color,
            borderColor: color,
          },
        },
        passDisplayName(passId)
      );

      var descEl = el(
        "span",
        { className: "compiler-pass-selector__desc" },
        desc
      );

      var label = el("label", {
        className: "compiler-pass-selector__label",
        htmlFor: id,
      }, [checkbox, pill, descEl]);

      checkboxList.appendChild(label);
    });

    row.appendChild(checkboxList);

    var btn = el(
      "button",
      {
        className: "ghost-button compiler-pass-selector__apply",
        onClick: function () {
          var selected = [];
          Object.keys(checkboxes).forEach(function (passId) {
            if (checkboxes[passId].checked) {
              selected.push(passId);
            }
          });
          onOptimize(selected);
        },
      },
      "Apply Passes"
    );
    row.appendChild(btn);

    container.appendChild(row);
  }

  // ── renderSideBySide ────────────────────────────────────────────────

  function renderSideBySide(container, original, optimized, changes) {
    container.innerHTML = "";

    var modifiedNodeIds = changes.map(function (c) {
      return c.node_id;
    });

    var wrapper = el("div", { className: "compiler-side-by-side" });

    // Original panel
    var leftPanel = el("div", { className: "compiler-side-by-side__panel" });
    leftPanel.appendChild(
      el("div", { className: "compiler-side-by-side__title" }, "Original Graph")
    );
    var leftCanvas = el("div", {
      className: "compiler-side-by-side__canvas canvas-card",
    });
    leftPanel.appendChild(leftCanvas);
    wrapper.appendChild(leftPanel);

    // Optimized panel
    var rightPanel = el("div", { className: "compiler-side-by-side__panel" });
    rightPanel.appendChild(
      el(
        "div",
        { className: "compiler-side-by-side__title" },
        "Optimized Graph"
      )
    );
    var rightCanvas = el("div", {
      className: "compiler-side-by-side__canvas canvas-card",
    });
    rightPanel.appendChild(rightCanvas);
    wrapper.appendChild(rightPanel);

    container.appendChild(wrapper);

    // Render graphs into canvases
    renderGraph(leftCanvas, original, [], null);
    renderGraph(rightCanvas, optimized, modifiedNodeIds, null);

    // Return handle for highlight updates
    return {
      highlightNode: function (nodeId) {
        rightCanvas.innerHTML = "";
        renderGraph(rightCanvas, optimized, modifiedNodeIds, nodeId);
      },
    };
  }

  // ── renderChangesList ───────────────────────────────────────────────

  function renderChangesList(container, changes, onHighlight) {
    container.innerHTML = "";

    if (!changes || changes.length === 0) {
      container.appendChild(
        el(
          "div",
          { className: "compiler-changes__empty" },
          "No changes applied."
        )
      );
      return;
    }

    // Group by pass
    var groups = {};
    var groupOrder = [];
    changes.forEach(function (change) {
      var pass = change.pass;
      if (!groups[pass]) {
        groups[pass] = [];
        groupOrder.push(pass);
      }
      groups[pass].push(change);
    });

    var listEl = el("div", { className: "compiler-changes" });

    groupOrder.forEach(function (pass) {
      var section = el("details", {
        className: "panel-disclosure compiler-changes__group",
      });
      section.open = true;

      var color = passColor(pass);

      // Summary header
      var summary = el("summary", null, [
        el(
          "span",
          {
            className: "compiler-pass-pill",
            style: {
              background: passBg(pass),
              color: color,
              borderColor: color,
            },
          },
          passDisplayName(pass)
        ),
        el(
          "span",
          { className: "compiler-changes__count" },
          " (" + groups[pass].length + ")"
        ),
      ]);
      section.appendChild(summary);

      var body = el("div", { className: "panel-disclosure__body" });

      groups[pass].forEach(function (change) {
        var card = el("div", {
          className: "compiler-changes__item",
          onClick: function () {
            if (onHighlight) onHighlight(change.node_id);
          },
        });

        // Node badge
        var badge = el("span", { className: "compiler-changes__node-badge" }, [
          el("span", { className: "compiler-changes__node-id" }, "#" + change.node_id),
          el("span", { className: "compiler-changes__node-name" }, change.node_name),
        ]);

        // Attribute
        var attr = el(
          "span",
          { className: "compiler-changes__attr" },
          change.attribute + " = " + formatValue(change.value)
        );

        // Description
        var desc = el(
          "span",
          { className: "compiler-changes__desc" },
          change.description
        );

        card.appendChild(badge);
        card.appendChild(attr);
        card.appendChild(desc);
        body.appendChild(card);
      });

      section.appendChild(body);
      listEl.appendChild(section);
    });

    container.appendChild(listEl);
  }

  // ── renderCompilerView ──────────────────────────────────────────────

  function renderCompilerView(container, data) {
    container.innerHTML = "";

    var root = el("div", { className: "compiler-view" });

    // 1. Pass selector
    var passContainer = el("div", { className: "compiler-view__section" });
    renderPassSelector(passContainer, data.passes_applied, function (selectedPasses) {
      // Re-fetch with selected passes -- dispatch custom event for host to handle
      var event = new CustomEvent("apxm:optimize", {
        detail: { passes: selectedPasses },
        bubbles: true,
      });
      container.dispatchEvent(event);
    });
    root.appendChild(passContainer);

    // 2. Stats bar
    var statsContainer = el("div", { className: "compiler-view__section" });
    renderStats(statsContainer, data.stats);
    root.appendChild(statsContainer);

    // 3. Side-by-side graphs
    var graphContainer = el("div", {
      className: "compiler-view__section compiler-view__section--graphs",
    });
    var sideBySide = renderSideBySide(
      graphContainer,
      data.original,
      data.optimized,
      data.changes
    );
    root.appendChild(graphContainer);

    // 4. Changes list
    var changesContainer = el("div", { className: "compiler-view__section" });
    renderChangesList(changesContainer, data.changes, function (nodeId) {
      if (sideBySide) sideBySide.highlightNode(nodeId);
    });
    root.appendChild(changesContainer);

    container.appendChild(root);
  }

  // ── Inject scoped styles ────────────────────────────────────────────

  function injectStyles() {
    if (document.getElementById("compiler-viewer-styles")) return;

    // Resolve dynamic color values for CSS
    var accentColor = col("accent", "#3ea6c6");
    var savingsColor = col("success", "#58bb81");
    var textColor = col("text", "#e8eef5");
    var textMuted = col("textMuted", "rgba(232, 238, 245, 0.5)");
    var border = col("border", "rgba(133, 146, 166, 0.14)");
    var surfaceBg = col("surface", "rgba(255, 255, 255, 0.03)");
    var surfaceHover = col("surfaceHover", "rgba(255, 255, 255, 0.05)");
    var accentSubtle = col("accentSubtle", "rgba(62, 166, 198, 0.08)");
    var accentHoverBg = col("accentHoverBg", "rgba(62, 166, 198, 0.06)");
    var accentHoverBorder = col("accentHoverBorder", "rgba(62, 166, 198, 0.2)");
    var canvasGradientStart = col("canvasGradientStart", "rgba(62, 166, 198, 0.08)");
    var canvasBgTop = col("canvasBgTop", "rgba(12, 19, 27, 0.98)");
    var canvasBgBottom = col("canvasBgBottom", "rgba(10, 16, 24, 0.98)");

    var css = [
      // Layout
      ".compiler-view {",
      "  display: flex;",
      "  flex-direction: column;",
      "  gap: 12px;",
      "  height: 100%;",
      "  overflow: auto;",
      "  padding: 10px;",
      "}",

      ".compiler-view__section {",
      "  flex-shrink: 0;",
      "}",

      ".compiler-view__section--graphs {",
      "  flex: 1 1 auto;",
      "  min-height: 320px;",
      "}",

      // Pass selector
      ".compiler-pass-selector {",
      "  display: flex;",
      "  align-items: flex-start;",
      "  justify-content: space-between;",
      "  gap: 12px;",
      "  padding: 10px 12px;",
      "  border-radius: 10px;",
      "  border: 1px solid " + border + ";",
      "  background: " + surfaceBg + ";",
      "}",

      ".compiler-pass-selector__list {",
      "  display: flex;",
      "  flex-wrap: wrap;",
      "  gap: 10px;",
      "}",

      ".compiler-pass-selector__label {",
      "  display: flex;",
      "  align-items: flex-start;",
      "  gap: 6px;",
      "  cursor: pointer;",
      "  padding: 6px 10px;",
      "  border-radius: 8px;",
      "  border: 1px solid rgba(133, 146, 166, 0.1);",
      "  background: rgba(255, 255, 255, 0.02);",
      "  transition: background 140ms ease;",
      "  max-width: 260px;",
      "}",

      ".compiler-pass-selector__label:hover {",
      "  background: " + surfaceHover + ";",
      "}",

      ".compiler-pass-selector__checkbox {",
      "  margin-top: 3px;",
      "  accent-color: var(--accent, " + accentColor + ");",
      "  flex-shrink: 0;",
      "}",

      ".compiler-pass-selector__desc {",
      "  display: block;",
      "  font-size: 0.68rem;",
      "  color: " + textMuted + ";",
      "  line-height: 1.4;",
      "  margin-top: 3px;",
      "}",

      ".compiler-pass-selector__apply {",
      "  align-self: center;",
      "  flex-shrink: 0;",
      "}",

      // Pass pill (reused in selector and changes)
      ".compiler-pass-pill {",
      "  display: inline-flex;",
      "  align-items: center;",
      "  border-radius: 999px;",
      "  min-height: 22px;",
      "  padding: 0 8px;",
      "  font-size: 0.68rem;",
      "  font-weight: 700;",
      "  letter-spacing: 0.06em;",
      "  white-space: nowrap;",
      "  border: 1px solid currentColor;",
      "  border-width: 0;",
      "}",

      // Stats bar
      ".compiler-stats {",
      "  display: flex;",
      "  gap: 10px;",
      "}",

      ".compiler-stats__card {",
      "  flex: 1;",
      "  display: flex;",
      "  flex-direction: column;",
      "  gap: 4px;",
      "  padding: 10px 14px;",
      "  border-radius: 10px;",
      "  border: 1px solid " + border + ";",
      "  background: " + surfaceBg + ";",
      "}",

      ".compiler-stats__label {",
      "  font-size: 0.66rem;",
      "  font-weight: 700;",
      "  text-transform: uppercase;",
      "  letter-spacing: 0.12em;",
      "  color: " + textMuted + ";",
      "}",

      ".compiler-stats__value {",
      "  font-family: 'Manrope', 'Inter', sans-serif;",
      "  font-size: 1.4rem;",
      "  font-weight: 800;",
      "  color: " + textColor + ";",
      "}",

      ".compiler-stats__value--savings {",
      "  color: " + savingsColor + ";",
      "}",

      // Side-by-side
      ".compiler-side-by-side {",
      "  display: grid;",
      "  grid-template-columns: 1fr 1fr;",
      "  gap: 10px;",
      "  height: 100%;",
      "  min-height: 0;",
      "}",

      ".compiler-side-by-side__panel {",
      "  display: flex;",
      "  flex-direction: column;",
      "  min-height: 0;",
      "  gap: 6px;",
      "}",

      ".compiler-side-by-side__title {",
      "  font-family: 'Manrope', 'Inter', sans-serif;",
      "  font-size: 0.82rem;",
      "  font-weight: 700;",
      "  letter-spacing: 0.06em;",
      "  text-transform: uppercase;",
      "  color: " + textMuted + ";",
      "  padding: 0 4px;",
      "}",

      ".compiler-side-by-side__canvas {",
      "  flex: 1 1 auto;",
      "  min-height: 0;",
      "  overflow: hidden;",
      "  background:",
      "    radial-gradient(circle at top left, " + canvasGradientStart + ", transparent 26%),",
      "    linear-gradient(180deg, " + canvasBgTop + ", " + canvasBgBottom + ");",
      "}",

      ".compiler-graph-svg-wrap {",
      "  padding: 8px;",
      "}",

      ".compiler-graph-svg-wrap svg {",
      "  display: block;",
      "}",

      // Changes list
      ".compiler-changes {",
      "  display: flex;",
      "  flex-direction: column;",
      "  gap: 8px;",
      "}",

      ".compiler-changes__empty {",
      "  padding: 20px;",
      "  text-align: center;",
      "  color: " + textMuted + ";",
      "  font-size: 0.84rem;",
      "}",

      ".compiler-changes__group summary {",
      "  display: flex;",
      "  align-items: center;",
      "  gap: 8px;",
      "}",

      ".compiler-changes__count {",
      "  font-size: 0.76rem;",
      "  color: " + textMuted + ";",
      "}",

      ".compiler-changes__item {",
      "  display: flex;",
      "  flex-wrap: wrap;",
      "  align-items: center;",
      "  gap: 8px;",
      "  padding: 8px 10px;",
      "  border-radius: 8px;",
      "  border: 1px solid rgba(133, 146, 166, 0.1);",
      "  background: rgba(255, 255, 255, 0.02);",
      "  cursor: pointer;",
      "  transition: background 140ms ease, border-color 140ms ease;",
      "}",

      ".compiler-changes__item:hover {",
      "  background: " + accentHoverBg + ";",
      "  border-color: " + accentHoverBorder + ";",
      "}",

      ".compiler-changes__node-badge {",
      "  display: inline-flex;",
      "  align-items: center;",
      "  gap: 4px;",
      "  padding: 2px 8px;",
      "  border-radius: 6px;",
      "  background: rgba(255, 255, 255, 0.06);",
      "  font-size: 0.76rem;",
      "  font-weight: 600;",
      "}",

      ".compiler-changes__node-id {",
      "  font-family: 'IBM Plex Mono', monospace;",
      "  font-size: 0.68rem;",
      "  color: " + textMuted + ";",
      "}",

      ".compiler-changes__node-name {",
      "  color: " + textColor + ";",
      "}",

      ".compiler-changes__attr {",
      "  font-family: 'IBM Plex Mono', monospace;",
      "  font-size: 0.74rem;",
      "  color: var(--accent, " + accentColor + ");",
      "  padding: 2px 6px;",
      "  border-radius: 4px;",
      "  background: " + accentSubtle + ";",
      "}",

      ".compiler-changes__desc {",
      "  font-size: 0.76rem;",
      "  color: rgba(232, 238, 245, 0.6);",
      "  flex-basis: 100%;",
      "  line-height: 1.4;",
      "}",

      // Responsive
      "@media (max-width: 860px) {",
      "  .compiler-side-by-side {",
      "    grid-template-columns: 1fr;",
      "  }",
      "  .compiler-pass-selector {",
      "    flex-direction: column;",
      "  }",
      "  .compiler-stats {",
      "    flex-direction: column;",
      "  }",
      "}",
    ].join("\n");

    var style = document.createElement("style");
    style.id = "compiler-viewer-styles";
    style.textContent = css;
    document.head.appendChild(style);
  }

  // ── Public API ──────────────────────────────────────────────────────

  injectStyles();

  window.CompilerViewer = {
    renderCompilerView: renderCompilerView,
    renderPassSelector: renderPassSelector,
    renderSideBySide: renderSideBySide,
    renderChangesList: renderChangesList,
    renderStats: renderStats,
    passDisplayName: passDisplayName,
    passColor: passColor,
  };
})();
