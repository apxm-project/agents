// graph.js -- APXM workflow graph renderer (interactive SVG DAG)
// Vanilla JS, no dependencies. Renders graph JSON into pan/zoom SVG.

(function () {
  "use strict";

  // Shared constants from window.APXM (with fallbacks for standalone use)
  var C = window.APXM || {};
  var COL = C.colors || {};

  // Module-level color shortcuts
  var _cBlue         = COL.blue          || "#58a6ff";
  var _cPurple       = COL.purple        || "#bc8cff";
  var _cYellow       = COL.yellow        || "#d29922";
  var _cWhite        = COL.white         || "#ffffff";

  var SVGNS = "http://www.w3.org/2000/svg";

  // Layout constants
  var NODE_WIDTH = 180;
  var NODE_HEIGHT = 50;
  var H_SPACING = 220;
  var V_SPACING = 90;
  var PADDING = 40;

  // Edge helpers — delegate to APXM.edgeStyles when available
  function edgeColor(dep) {
    var styles = C.edgeStyles;
    if (styles && styles[dep]) return styles[dep].color;
    var fallback = { Data: _cBlue, Control: _cPurple, Effect: _cYellow };
    return fallback[dep] || _cBlue;
  }

  function edgeDash(dep) {
    var styles = C.edgeStyles;
    if (styles && styles[dep]) return styles[dep].dash || null;
    var fallback = { Data: null, Control: "6,4", Effect: "2,4" };
    return fallback.hasOwnProperty(dep) ? fallback[dep] : null;
  }

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  function getOpCategory(opType) {
    return (C.getOpCategory || function () { return "constants"; })(opType);
  }

  function getCategoryFill(cat) {
    return (C.getCategoryFill || function () { return COL.elevated || "#2a2a2a"; })(cat);
  }

  function getCategoryStroke(cat) {
    return (C.getCategoryStroke || function () { return COL.textMuted || "#6e7681"; })(cat);
  }

  function svgEl(tag, attrs) {
    const el = document.createElementNS(SVGNS, tag);
    if (attrs) {
      for (const [k, v] of Object.entries(attrs)) {
        el.setAttribute(k, v);
      }
    }
    return el;
  }

  // ---------------------------------------------------------------------------
  // Layout: topological level assignment via BFS
  // ---------------------------------------------------------------------------

  function computeLevels(nodes, edges) {
    const idSet = new Set(nodes.map((n) => n.id));
    const incoming = new Map(); // nodeId -> Set of parent ids
    const outgoing = new Map(); // nodeId -> [child ids]

    for (const n of nodes) {
      incoming.set(n.id, new Set());
      outgoing.set(n.id, []);
    }

    for (const e of edges) {
      if (idSet.has(e.from) && idSet.has(e.to)) {
        incoming.get(e.to).add(e.from);
        outgoing.get(e.from).push(e.to);
      }
    }

    // Entry nodes have no incoming edges
    const levels = new Map();
    const queue = [];
    for (const n of nodes) {
      if (incoming.get(n.id).size === 0) {
        levels.set(n.id, 0);
        queue.push(n.id);
      }
    }

    // BFS: each child level = max(parent levels) + 1
    let head = 0;
    while (head < queue.length) {
      const cur = queue[head++];
      const curLevel = levels.get(cur);
      for (const child of outgoing.get(cur)) {
        const prev = levels.get(child);
        const newLevel = curLevel + 1;
        if (prev === undefined || newLevel > prev) {
          levels.set(child, newLevel);
        }
        // Push child once all its parents have been visited.
        // We re-push on upgrades so downstream levels update too.
        queue.push(child);
      }
    }

    // Safety: assign level 0 to any node not reached (disconnected)
    for (const n of nodes) {
      if (!levels.has(n.id)) {
        levels.set(n.id, 0);
      }
    }

    return levels;
  }

  function layoutNodes(nodes, levels) {
    // Group nodes by level
    const byLevel = new Map();
    let maxLevel = 0;
    for (const n of nodes) {
      const lvl = levels.get(n.id);
      if (lvl > maxLevel) maxLevel = lvl;
      if (!byLevel.has(lvl)) byLevel.set(lvl, []);
      byLevel.get(lvl).push(n);
    }

    // Determine maximum width (number of nodes at any level)
    let maxWidth = 0;
    for (const [, group] of byLevel) {
      if (group.length > maxWidth) maxWidth = group.length;
    }

    const positions = new Map(); // nodeId -> {x, y}

    for (let lvl = 0; lvl <= maxLevel; lvl++) {
      const group = byLevel.get(lvl) || [];
      const totalWidth = group.length * H_SPACING;
      const maxTotalWidth = maxWidth * H_SPACING;
      const offsetX = (maxTotalWidth - totalWidth) / 2;

      for (let i = 0; i < group.length; i++) {
        const x = PADDING + offsetX + i * H_SPACING + (H_SPACING - NODE_WIDTH) / 2;
        const y = PADDING + lvl * V_SPACING;
        positions.set(group[i].id, { x, y });
      }
    }

    // Compute total SVG dimensions
    let svgW = 0;
    let svgH = 0;
    for (const [, pos] of positions) {
      const right = pos.x + NODE_WIDTH + PADDING;
      const bottom = pos.y + NODE_HEIGHT + PADDING;
      if (right > svgW) svgW = right;
      if (bottom > svgH) svgH = bottom;
    }

    return { positions, svgW, svgH };
  }

  // ---------------------------------------------------------------------------
  // Pan / Zoom state
  // ---------------------------------------------------------------------------

  function createTransformState() {
    return { scale: 1, tx: 0, ty: 0, dragging: false, startX: 0, startY: 0 };
  }

  function applyTransform(g, state) {
    g.setAttribute(
      "transform",
      "translate(" + state.tx + "," + state.ty + ") scale(" + state.scale + ")"
    );
  }

  // ---------------------------------------------------------------------------
  // Arrowhead marker defs
  // ---------------------------------------------------------------------------

  function createDefs() {
    var defs = svgEl("defs");
    var depTypes = ["Data", "Control", "Effect"];

    for (var i = 0; i < depTypes.length; i++) {
      var dep = depTypes[i];
      var color = edgeColor(dep);
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
        fill: color,
      });
      marker.appendChild(path);
      defs.appendChild(marker);
    }

    return defs;
  }

  // ---------------------------------------------------------------------------
  // Edge drawing
  // ---------------------------------------------------------------------------

  function drawEdge(parentG, fromPos, toPos, dep) {
    var color = edgeColor(dep || "Data");
    var dash = edgeDash(dep || "Data");
    var markerId = "arrow-" + (dep || "Data");

    // Source: bottom center of the from-node
    const x1 = fromPos.x + NODE_WIDTH / 2;
    const y1 = fromPos.y + NODE_HEIGHT;

    // Target: top center of the to-node
    const x2 = toPos.x + NODE_WIDTH / 2;
    const y2 = toPos.y;

    // Cubic bezier with control points offset vertically
    const dy = Math.abs(y2 - y1);
    const cpOffset = Math.max(dy * 0.4, 20);
    const cp1y = y1 + cpOffset;
    const cp2y = y2 - cpOffset;

    const d =
      "M " + x1 + " " + y1 +
      " C " + x1 + " " + cp1y +
      ", " + x2 + " " + cp2y +
      ", " + x2 + " " + y2;

    const path = svgEl("path", {
      d: d,
      fill: "none",
      stroke: color,
      "stroke-width": "1.5",
      "marker-end": "url(#" + markerId + ")",
    });

    if (dash) {
      path.setAttribute("stroke-dasharray", dash);
    }

    path.setAttribute("class", "graph-edge");
    path.dataset.from = fromPos._id;
    path.dataset.to = toPos._id;

    parentG.appendChild(path);
  }

  // ---------------------------------------------------------------------------
  // Node drawing
  // ---------------------------------------------------------------------------

  function drawNode(parentG, node, pos, onNodeClick) {
    var cat = getOpCategory(node.op);
    var fill = getCategoryFill(cat);
    var stroke = getCategoryStroke(cat);

    const g = svgEl("g", {
      class: "graph-node",
      "data-node-id": String(node.id),
    });
    g.setAttribute("transform", "translate(" + pos.x + "," + pos.y + ")");

    // Rounded rect background
    const rect = svgEl("rect", {
      x: "0",
      y: "0",
      width: String(NODE_WIDTH),
      height: String(NODE_HEIGHT),
      rx: "8",
      ry: "8",
      fill: fill,
      stroke: stroke,
      "stroke-width": "1.5",
    });
    g.appendChild(rect);

    // Op label (top, smaller, category color)
    const opLabel = svgEl("text", {
      x: String(NODE_WIDTH / 2),
      y: "16",
      fill: stroke,
      "font-size": "10",
      "font-family": "monospace",
      "font-weight": "bold",
      "text-anchor": "middle",
      "pointer-events": "none",
    });
    opLabel.textContent = node.op || "";
    g.appendChild(opLabel);

    // Node name label (center, white)
    const nameLabel = svgEl("text", {
      x: String(NODE_WIDTH / 2),
      y: "34",
      fill: _cWhite,
      "font-size": "13",
      "font-family": "sans-serif",
      "text-anchor": "middle",
      "pointer-events": "none",
    });
    // Truncate long names
    let displayName = node.name || "";
    if (displayName.length > 22) {
      displayName = displayName.slice(0, 20) + "\u2026";
    }
    nameLabel.textContent = displayName;
    g.appendChild(nameLabel);

    // ID badge (small circle, top-left)
    const badgeR = 10;
    const badge = svgEl("circle", {
      cx: String(badgeR),
      cy: String(badgeR),
      r: String(badgeR),
      fill: stroke,
    });
    g.appendChild(badge);

    const badgeText = svgEl("text", {
      x: String(badgeR),
      y: String(badgeR + 4),
      fill: _cWhite,
      "font-size": "10",
      "font-family": "monospace",
      "font-weight": "bold",
      "text-anchor": "middle",
      "pointer-events": "none",
    });
    badgeText.textContent = String(node.id);
    g.appendChild(badgeText);

    // Click handler
    g.style.cursor = "pointer";
    g.addEventListener("click", function (e) {
      e.stopPropagation();
      // Remove .selected from all nodes in the same SVG
      const svg = g.closest("svg");
      if (svg) {
        const prev = svg.querySelectorAll(".graph-node.selected");
        for (let i = 0; i < prev.length; i++) {
          prev[i].classList.remove("selected");
        }
      }
      g.classList.add("selected");
      if (typeof onNodeClick === "function") {
        onNodeClick(node.id);
      }
    });

    parentG.appendChild(g);
  }

  // ---------------------------------------------------------------------------
  // Internal state for node positions (used by getNodePosition)
  // ---------------------------------------------------------------------------

  let _lastPositions = null;

  // ---------------------------------------------------------------------------
  // renderGraph
  // ---------------------------------------------------------------------------

  function renderGraph(container, graph, onNodeClick) {
    // Clear container
    while (container.firstChild) {
      container.removeChild(container.firstChild);
    }

    const nodes = graph.nodes || [];
    const edges = graph.edges || [];

    if (nodes.length === 0) {
      container.textContent = "Empty graph";
      return;
    }

    // 1. Compute levels
    const levels = computeLevels(nodes, edges);

    // 2. Layout
    const { positions, svgW, svgH } = layoutNodes(nodes, levels);
    _lastPositions = positions;

    // Tag positions with node ids for edge drawing
    const posById = new Map();
    for (const n of nodes) {
      const p = positions.get(n.id);
      posById.set(n.id, Object.assign({ _id: n.id }, p));
    }

    // 3. Create SVG
    const svg = svgEl("svg", {
      width: "100%",
      height: "100%",
      viewBox: "0 0 " + svgW + " " + svgH,
    });
    svg.style.display = "block";
    svg.style.userSelect = "none";

    // Inject embedded styles for selection / highlight
    const style = svgEl("style");
    style.textContent = [
      ".graph-node.selected > rect { stroke-width: 3; filter: drop-shadow(0 0 6px rgba(255,255,255,0.4)); }",
      ".graph-node.highlighted > rect { stroke-width: 3; filter: drop-shadow(0 0 8px rgba(88,166,255,0.7)); }",
      ".graph-edge.path-highlight { stroke-width: 3; filter: drop-shadow(0 0 4px rgba(88,166,255,0.6)); }",
    ].join("\n");
    svg.appendChild(style);

    // Defs (arrowheads)
    svg.appendChild(createDefs());

    // Root group for pan/zoom
    const rootG = svgEl("g", { class: "graph-root" });
    svg.appendChild(rootG);

    // 4. Draw edges (behind nodes)
    const edgeG = svgEl("g", { class: "graph-edges" });
    rootG.appendChild(edgeG);

    for (const e of edges) {
      const fromP = posById.get(e.from);
      const toP = posById.get(e.to);
      if (fromP && toP) {
        drawEdge(edgeG, fromP, toP, e.dependency);
      }
    }

    // 5. Draw nodes
    const nodeG = svgEl("g", { class: "graph-nodes" });
    rootG.appendChild(nodeG);

    for (const n of nodes) {
      const p = posById.get(n.id);
      if (p) {
        drawNode(nodeG, n, p, onNodeClick);
      }
    }

    // 6. Pan / Zoom
    const ts = createTransformState();

    // Zoom with mouse wheel
    svg.addEventListener("wheel", function (e) {
      e.preventDefault();
      const factor = e.deltaY < 0 ? 1.1 : 0.9;
      const newScale = ts.scale * factor;
      // Clamp scale
      if (newScale < 0.1 || newScale > 5) return;

      // Zoom towards cursor position
      const rect = svg.getBoundingClientRect();
      const mx = e.clientX - rect.left;
      const my = e.clientY - rect.top;

      ts.tx = mx - factor * (mx - ts.tx);
      ts.ty = my - factor * (my - ts.ty);
      ts.scale = newScale;
      applyTransform(rootG, ts);
    }, { passive: false });

    // Pan with mouse drag on background
    svg.addEventListener("mousedown", function (e) {
      // Only start drag if clicking on the SVG background (not on a node)
      if (e.target === svg || e.target.closest(".graph-edges") || e.target === rootG) {
        ts.dragging = true;
        ts.startX = e.clientX - ts.tx;
        ts.startY = e.clientY - ts.ty;
        svg.style.cursor = "grabbing";
      }
    });

    svg.addEventListener("mousemove", function (e) {
      if (!ts.dragging) return;
      ts.tx = e.clientX - ts.startX;
      ts.ty = e.clientY - ts.startY;
      applyTransform(rootG, ts);
    });

    function stopDrag() {
      if (ts.dragging) {
        ts.dragging = false;
        svg.style.cursor = "";
      }
    }

    svg.addEventListener("mouseup", stopDrag);
    svg.addEventListener("mouseleave", stopDrag);

    container.appendChild(svg);
  }

  // ---------------------------------------------------------------------------
  // highlightNodes
  // ---------------------------------------------------------------------------

  function highlightNodes(container, nodeIds, className) {
    const cls = className || "highlighted";
    const idSet = new Set(nodeIds.map(String));
    const svg = container.querySelector("svg");
    if (!svg) return;

    const nodeEls = svg.querySelectorAll(".graph-node");
    for (let i = 0; i < nodeEls.length; i++) {
      const el = nodeEls[i];
      const nid = el.getAttribute("data-node-id");
      if (idSet.has(nid)) {
        el.classList.add(cls);
      }
    }
  }

  // ---------------------------------------------------------------------------
  // highlightPath
  // ---------------------------------------------------------------------------

  function highlightPath(container, nodeIds) {
    const svg = container.querySelector("svg");
    if (!svg) return;

    const idSet = new Set(nodeIds.map(String));

    // Highlight nodes on the path
    highlightNodes(container, nodeIds, "highlighted");

    // Highlight edges between consecutive nodes in the path
    const pathSet = new Set();
    for (let i = 0; i < nodeIds.length - 1; i++) {
      pathSet.add(String(nodeIds[i]) + "->" + String(nodeIds[i + 1]));
    }

    const edgeEls = svg.querySelectorAll(".graph-edge");
    for (let i = 0; i < edgeEls.length; i++) {
      const el = edgeEls[i];
      const key = el.dataset.from + "->" + el.dataset.to;
      if (pathSet.has(key)) {
        el.classList.add("path-highlight");
      }
    }
  }

  // ---------------------------------------------------------------------------
  // getNodePosition
  // ---------------------------------------------------------------------------

  function getNodePosition(nodeId) {
    if (!_lastPositions) return null;
    const p = _lastPositions.get(nodeId);
    if (!p) return null;
    // Return center of the node
    return {
      x: p.x + NODE_WIDTH / 2,
      y: p.y + NODE_HEIGHT / 2,
    };
  }

  // ---------------------------------------------------------------------------
  // Export
  // ---------------------------------------------------------------------------

  window.GraphRenderer = {
    renderGraph: renderGraph,
    highlightNodes: highlightNodes,
    highlightPath: highlightPath,
    getNodePosition: getNodePosition,
    getOpCategory: getOpCategory,
  };
})();
