// APXM GUI Dashboard -- Main Application
// Vanilla JS entry point: state management, tab switching, API calls,
// and coordination between component modules.
//
// All string constants (API endpoints, element IDs, CSS classes) come from
// window.APXM (loaded by constants.js before this file).

// ---------------------------------------------------------------------------
// Aliases for readability
// ---------------------------------------------------------------------------
var API  = APXM.api;
var EL   = APXM.el;
var CSS  = APXM.css;

// ---------------------------------------------------------------------------
// Global application state
// ---------------------------------------------------------------------------
var state = {
  currentTab: 'graph',
  graph: null,
  graphPath: '',
  analysis: null,
  optimizedGraph: null,
  session: null,
  sessionPath: '',
  ops: null,
  config: null,
  selectedNode: null,
  liveHandle: null,
};

// ---------------------------------------------------------------------------
// API helper
// ---------------------------------------------------------------------------
async function api(endpoint, params) {
  params = params || {};
  var url = new URL(endpoint, window.location.origin);
  Object.entries(params).forEach(function(entry) {
    url.searchParams.set(entry[0], entry[1]);
  });
  var res = await fetch(url);
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------
function esc(str) {
  var div = document.createElement('div');
  div.textContent = str;
  return div.innerHTML;
}

function $(id) {
  return document.getElementById(id);
}

function showError(containerId, message) {
  var el = $(containerId);
  if (el) {
    el.innerHTML = '<p class="' + CSS.error + '">' + esc(message) + '</p>';
    el.style.display = 'block';
  } else {
    console.error(message);
  }
}

function updateStatus(message) {
  var bar = $(EL.statusMessage);
  if (bar) bar.textContent = message;
}

// ---------------------------------------------------------------------------
// 1. Tab switching
// ---------------------------------------------------------------------------
function switchTab(tabName) {
  state.currentTab = tabName;

  document.querySelectorAll(CSS.tabPanel).forEach(function(el) {
    el.classList.remove(CSS.active);
    el.hidden = true;
  });

  document.querySelectorAll(CSS.tabBtn).forEach(function(el) {
    el.classList.remove(CSS.active);
    el.setAttribute('aria-selected', 'false');
  });

  var pane = $('tab-' + tabName);
  if (pane) {
    pane.classList.add(CSS.active);
    pane.hidden = false;
  }

  var btn = document.querySelector(CSS.tabBtn + '[data-tab="' + tabName + '"]');
  if (btn) {
    btn.classList.add(CSS.active);
    btn.setAttribute('aria-selected', 'true');
  }
}

// ---------------------------------------------------------------------------
// 2. Graph loading & rendering
// ---------------------------------------------------------------------------
async function loadGraph(path) {
  if (!path) return;
  try {
    updateStatus('Loading graph...');
    state.graphPath = path;
    state.graph = await api(API.graph, { path: path });
    state.selectedNode = null;
    state.optimizedGraph = null;
    renderGraph(state.graph);
    clearInspector();
    loadAnalysis(path);
    updateStatus('Graph loaded: ' + path);
  } catch (err) {
    showError('graph-error', 'Failed to load graph: ' + err.message);
    updateStatus('Graph load failed');
  }
}

function renderGraph(graph) {
  var canvas = $(EL.graphCanvas);
  if (!canvas) return;
  var empty = $(EL.graphCanvasEmpty);
  if (!graph || !graph.nodes || graph.nodes.length === 0) {
    if (empty) empty.style.display = '';
    return;
  }
  if (empty) empty.style.display = 'none';

  if (window.GraphRenderer) {
    GraphRenderer.renderGraph(canvas, graph, selectNode);
  }
}

function selectNode(nodeId) {
  state.selectedNode = nodeId;
  var node = (state.graph && state.graph.nodes || []).find(function(n) { return n.id === nodeId; });
  if (node) {
    updateInspector(node);
    if (state.session && state.session.trace) {
      renderThinkingForNode(nodeId);
    }
  }
}

// ---------------------------------------------------------------------------
// 3. Analysis (bottom strip)
// ---------------------------------------------------------------------------
async function loadAnalysis(path) {
  try {
    state.analysis = await api(API.graphAnalyze, { path: path });
    renderAnalysisStrip(state.analysis);
  } catch (err) {
    // Non-critical — just leave defaults
  }
}

function renderAnalysisStrip(a) {
  if (!a) return;
  var set = function(id, val) { var el = $(id); if (el) el.textContent = val; };
  set(EL.statNodes,       a.total_nodes != null ? a.total_nodes : '--');
  set(EL.statEdges,       a.total_edges != null ? a.total_edges : '--');
  set(EL.statParallelism, a.max_parallelism != null ? a.max_parallelism : '--');
  set(EL.statCriticalPath,a.critical_path_length != null ? a.critical_path_length : '--');
  set(EL.statSpeedup,     a.max_parallelism > 1
      ? (a.total_nodes / a.critical_path_length).toFixed(2) + 'x'
      : '1.00x');
}

// ---------------------------------------------------------------------------
// 4. Node inspector
// ---------------------------------------------------------------------------
function updateInspector(node) {
  var panel = $(EL.inspectorContent);
  if (!panel) return;

  var cat = APXM.getOpCategory(node.op);
  var color = APXM.getCategoryColor(cat);

  var attrs = node.attributes || {};
  var attrRows = Object.entries(attrs)
    .map(function(entry) {
      var k = entry[0], v = entry[1];
      var valStr = typeof v === 'object' ? JSON.stringify(v, null, 2) : String(v);
      return '<tr><td class="kv-key">' + esc(k) + '</td><td><pre class="attr-value">' + esc(valStr) + '</pre></td></tr>';
    })
    .join('');

  panel.innerHTML =
    '<table class="kv-table"><tbody>' +
    '<tr><td class="kv-key">ID</td><td>' + esc(String(node.id)) + '</td></tr>' +
    '<tr><td class="kv-key">Name</td><td>' + esc(node.name || '') + '</td></tr>' +
    '<tr><td class="kv-key">Op</td><td><span class="op-badge" style="background:' + color + '">' + esc(node.op || '?') + '</span></td></tr>' +
    attrRows +
    '</tbody></table>' +
    '<div id="thinking-section"></div>';
}

function clearInspector() {
  var panel = $(EL.inspectorContent);
  if (panel) panel.innerHTML = '<p class="' + CSS.emptyHint + '">Click a node in the graph to inspect it</p>';
}

// ---------------------------------------------------------------------------
// 5. Thinking integration (in node inspector)
// ---------------------------------------------------------------------------
function renderThinkingForNode(nodeId) {
  var section = $('thinking-section');
  if (!section) return;
  if (!state.session || !state.session.trace) return;

  var traceEvents = state.session.trace.filter(function(evt) {
    var payload = evt.payload || {};
    return payload.node_id === nodeId;
  });

  if (traceEvents.length === 0) return;

  if (window.ThinkingViewer) {
    ThinkingViewer.renderThinkingView(section, nodeId, traceEvents);
  }
}

// ---------------------------------------------------------------------------
// 6. Optimization (Compiler tab)
// ---------------------------------------------------------------------------
async function optimizeGraph(passes) {
  if (!state.graphPath) {
    showError('compiler-error', 'No graph loaded. Load a graph first.');
    return;
  }
  var panel = $('compiler-output');
  if (panel) panel.innerHTML = '<p>Optimizing...</p>';

  try {
    updateStatus('Running optimization passes...');
    var params = { path: state.graphPath };
    if (passes) params.passes = passes;
    var data = await api(API.graphOptimized, params);
    state.optimizedGraph = data;

    if (window.CompilerViewer && panel) {
      var compilerData = {
        original: state.graph,
        optimized: data.graph || data,
        changes: data.changes || [],
        passes_applied: data.passes_applied || [],
        stats: data.stats || { nodes_modified: 0, attributes_added: 0, estimated_token_savings_pct: 0 },
      };
      CompilerViewer.renderCompilerView(panel, compilerData);
    }
    updateStatus('Optimization complete');
  } catch (err) {
    if (panel) panel.innerHTML = '<p class="' + CSS.error + '">Optimization failed: ' + esc(err.message) + '</p>';
    updateStatus('Optimization failed');
  }
}

// ---------------------------------------------------------------------------
// 7. Session loading & rendering
// ---------------------------------------------------------------------------
async function loadSession(path) {
  if (!path) return;
  try {
    updateStatus('Loading session...');
    state.sessionPath = path;
    state.session = await api(API.session, { path: path });
    renderSessionView(state.session);
    updateStatus('Session loaded: ' + path);
  } catch (err) {
    showError('session-error', 'Failed to load session: ' + err.message);
    updateStatus('Session load failed');
  }
}

function renderSessionView(session) {
  var panel = $('session-panel');
  if (window.SessionViewer && panel) {
    SessionViewer.renderSession(panel, session);
  }
  renderSessionGantt(session);
}

function renderSessionGantt(session) {
  var ganttContainer = $('gantt-panel');
  if (!ganttContainer) return;
  if (!session || !session.trace || session.trace.length === 0) {
    ganttContainer.innerHTML = '';
    return;
  }

  var nodeNames = {};
  if (state.graph && state.graph.nodes) {
    state.graph.nodes.forEach(function(n) {
      nodeNames[n.id] = n.name || n.op || 'node-' + n.id;
    });
  }

  if (window.GanttChart) {
    GanttChart.renderGantt(ganttContainer, session.trace, nodeNames);
  }
}

// ---------------------------------------------------------------------------
// 8. Workspace integration (session node detail)
// ---------------------------------------------------------------------------
async function loadNodeWorkspace(nodeId) {
  var panel = $('workspace-panel');
  if (!panel) return;

  try {
    var nodeData = await api(API.sessionNode + '/' + nodeId, { path: state.sessionPath });
    if (window.WorkspaceViewer) {
      WorkspaceViewer.renderWorkspace(panel, nodeData);
    }
  } catch (err) {
    panel.innerHTML = '<p class="' + CSS.error + '">Failed to load workspace: ' + esc(err.message) + '</p>';
  }
}

// ---------------------------------------------------------------------------
// 9. Live session support
// ---------------------------------------------------------------------------
function startLive() {
  var input = $('input-live-session-path');
  var path = input ? input.value.trim() : state.sessionPath;
  if (!path) {
    showError('live-error', 'No session path specified.');
    return;
  }

  var container = $('live-panel');
  if (!container) return;

  if (window.LiveViewer) {
    state.liveHandle = LiveViewer.startLiveSession(container, path, state.graph);
    updateStatus('Live session started');
  }
}

function stopLive() {
  if (state.liveHandle && window.LiveViewer) {
    LiveViewer.stopLiveSession(state.liveHandle);
    state.liveHandle = null;
    updateStatus('Live session stopped');
  }
}

// ---------------------------------------------------------------------------
// 10. Ops catalog
// ---------------------------------------------------------------------------
async function loadOps() {
  try {
    state.ops = await api(API.ops);
    if (window.OpsViewer) {
      var panel = $('ops-panel');
      if (panel) OpsViewer.renderOpsView(panel, state.ops);
    }
  } catch (err) {
    // Non-critical
  }
}

// ---------------------------------------------------------------------------
// 11. Config
// ---------------------------------------------------------------------------
async function loadConfig() {
  try {
    state.config = await api(API.config);
    var configText = typeof state.config === 'string' ? state.config : JSON.stringify(state.config, null, 2);
    if (window.ConfigViewer) {
      var panel = $('config-panel');
      if (panel) ConfigViewer.renderConfigView(panel, configText);
    }
  } catch (err) {
    // Non-critical
  }
}

// ---------------------------------------------------------------------------
// 12. Examples browser
// ---------------------------------------------------------------------------
async function loadExamples() {
  var select = $(EL.examplesSelect);
  if (!select) return;
  try {
    var data = await api(API.examples);
    var examples = data.examples || [];
    if (examples.length === 0) {
      select.style.display = 'none';
      return;
    }

    // Group by category.
    var groups = {};
    examples.forEach(function(ex) {
      var cat = ex.category || 'other';
      if (!groups[cat]) groups[cat] = [];
      groups[cat].push(ex);
    });

    // Build optgroups.
    select.innerHTML = '<option value="">Examples...</option>';
    Object.entries(groups)
      .sort(function(a, b) { return a[0].localeCompare(b[0]); })
      .forEach(function(entry) {
        var cat = entry[0], items = entry[1];
        var optgroup = document.createElement('optgroup');
        optgroup.label = cat.charAt(0).toUpperCase() + cat.slice(1);
        items.forEach(function(ex) {
          var opt = document.createElement('option');
          opt.value = ex.path;
          var nodeInfo = ex.node_count ? ' (' + ex.node_count + ' nodes)' : '';
          opt.textContent = ex.name + nodeInfo;
          optgroup.appendChild(opt);
        });
        select.appendChild(optgroup);
      });
  } catch (err) {
    select.style.display = 'none';
  }
}

// ---------------------------------------------------------------------------
// 13. Startup file (--file arg)
// ---------------------------------------------------------------------------
async function checkStartupFile() {
  try {
    var data = await api(API.startup);
    if (data.initial_file) {
      var graphInput = $(EL.graphPathInput);
      if (graphInput) graphInput.value = data.initial_file;
      loadGraph(data.initial_file);
    }
  } catch (err) {
    // No startup file — that's fine.
  }
}

// ---------------------------------------------------------------------------
// Event binding -- DOMContentLoaded
// ---------------------------------------------------------------------------
document.addEventListener('DOMContentLoaded', function() {
  // Tab buttons
  document.querySelectorAll(CSS.tabBtn).forEach(function(btn) {
    btn.addEventListener('click', function() {
      var tab = btn.getAttribute('data-tab');
      if (tab) switchTab(tab);
    });
  });

  // Load Graph button
  var loadGraphBtn = $(EL.loadGraphBtn);
  var graphPathInput = $(EL.graphPathInput);
  if (loadGraphBtn && graphPathInput) {
    loadGraphBtn.addEventListener('click', function() { loadGraph(graphPathInput.value.trim()); });
    graphPathInput.addEventListener('keydown', function(e) {
      if (e.key === 'Enter') loadGraph(graphPathInput.value.trim());
    });
  }

  // Load Session button
  var loadSessionBtn = $(EL.loadSessionBtn);
  var sessionPathInput = $(EL.sessionPathInput);
  if (loadSessionBtn && sessionPathInput) {
    loadSessionBtn.addEventListener('click', function() { loadSession(sessionPathInput.value.trim()); });
    sessionPathInput.addEventListener('keydown', function(e) {
      if (e.key === 'Enter') loadSession(sessionPathInput.value.trim());
    });
  }

  // Optimize custom event from CompilerViewer
  document.addEventListener('apxm:optimize', function(e) {
    var passes = e.detail && e.detail.passes ? e.detail.passes.join(',') : '';
    optimizeGraph(passes);
  });

  // Examples dropdown
  var examplesSelect = $(EL.examplesSelect);
  if (examplesSelect) {
    examplesSelect.addEventListener('change', function() {
      var path = examplesSelect.value;
      if (path) {
        if (graphPathInput) graphPathInput.value = path;
        loadGraph(path);
      }
    });
  }

  // Set initial tab
  switchTab(state.currentTab);

  // Auto-load
  loadOps();
  loadConfig();
  loadExamples();
  checkStartupFile();

  updateStatus('Ready');
});
