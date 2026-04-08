// ops.js -- AIS Operations Reference Viewer
// Renders a searchable, filterable catalog of all AIS operations.
// Export: window.OpsViewer

(function () {
  'use strict';

  // ---------------------------------------------------------------------------
  // Shared constants (from window.APXM)
  // ---------------------------------------------------------------------------

  var C = window.APXM || {};
  var COL = C.colors || {};

  // Module-level color shortcuts for inline styles and syntax highlighting
  var _cText         = COL.text          || '#e6edf3';
  var _cTextSec      = COL.textSecondary || '#8b949e';
  var _cSurface      = COL.surface       || '#161b22';
  var _cBlue         = COL.blue          || '#58a6ff';
  var _cBlueBright   = COL.blueBright    || '#79c0ff';
  var _cBlueLight    = COL.blueLight     || '#a5d6ff';
  var _cGreen        = COL.green         || '#3fb950';
  var _cRed          = COL.red           || '#f85149';
  var _cRedLight     = COL.redLight      || '#ff7b72';
  var _cYellow       = COL.yellow        || '#d29922';
  var _cWhite        = COL.white         || '#ffffff';

  /** Map API-level category names to APXM's normalised category keys. */
  var CATEGORY_KEY_MAP = {
    Reasoning:       'reasoning',
    Tools:           'tools',
    ControlFlow:     'control',
    Sync:            'sync',
    Synchronization: 'sync',
    Communication:   'communication',
    Memory:          'memory',
    Coordination:    'coordination',
    ErrorHandling:   'error',
    Identity:        'identity',
    Internal:        'internal',
    Metadata:        'metadata',
  };

  /** Latency level to APXM severity key. */
  var LATENCY_SEVERITY = {
    High:   'red',
    Medium: 'yellow',
    Low:    'green',
    None:   'gray',
  };

  var CATEGORIES = [
    'All',
    'Reasoning',
    'Tools',
    'ControlFlow',
    'Sync',
    'Communication',
    'Memory',
    'Coordination',
    'ErrorHandling',
    'Identity',
    'Internal',
  ];

  var DEBOUNCE_MS = 150;

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  function el(tag, attrs, children) {
    const node = document.createElement(tag);
    if (attrs) {
      for (const [k, v] of Object.entries(attrs)) {
        if (k === 'style' && typeof v === 'object') {
          Object.assign(node.style, v);
        } else if (k.startsWith('on') && typeof v === 'function') {
          node.addEventListener(k.slice(2).toLowerCase(), v);
        } else if (k === 'className') {
          node.className = v;
        } else if (k === 'innerHTML') {
          node.innerHTML = v;
        } else {
          node.setAttribute(k, v);
        }
      }
    }
    if (children != null) {
      if (typeof children === 'string') {
        node.textContent = children;
      } else if (Array.isArray(children)) {
        children.forEach(function (c) {
          if (c != null) {
            node.appendChild(typeof c === 'string' ? document.createTextNode(c) : c);
          }
        });
      } else {
        node.appendChild(children);
      }
    }
    return node;
  }

  function debounce(fn, ms) {
    var timer = null;
    return function () {
      var ctx = this;
      var args = arguments;
      if (timer) clearTimeout(timer);
      timer = setTimeout(function () {
        timer = null;
        fn.apply(ctx, args);
      }, ms);
    };
  }

  /** Normalise API category strings to pill labels (display). */
  function normaliseCategory(cat) {
    if (!cat) return 'Internal';
    // The API may send "Synchronization" but pills use "Sync"
    if (cat === 'Synchronization') return 'Sync';
    return cat;
  }

  /** Map any category form to an APXM category key (lowercase). */
  function categoryKey(cat) {
    if (!cat) return 'internal';
    return CATEGORY_KEY_MAP[cat] || CATEGORY_KEY_MAP[normaliseCategory(cat)] || cat.toLowerCase();
  }

  /** Match a category from an op against the filter pill value. */
  function categoryMatches(opCategory, filterCategory) {
    if (filterCategory === 'All') return true;
    var norm = normaliseCategory(opCategory);
    return norm === filterCategory;
  }

  /** Return the color for a category, delegating to APXM.getCategoryColor(). */
  function categoryColor(cat) {
    var key = categoryKey(cat);
    if (typeof C.getCategoryColor === 'function') {
      return C.getCategoryColor(key);
    }
    if (C.categoryInfo && C.categoryInfo[key]) {
      return C.categoryInfo[key].color;
    }
    // Fallback when APXM module is not loaded
    return _cTextSec;
  }

  // ---------------------------------------------------------------------------
  // Inject scoped styles (once)
  // ---------------------------------------------------------------------------

  var stylesInjected = false;

  function injectStyles() {
    if (stylesInjected) return;
    stylesInjected = true;

    // Color shortcuts from shared constants
    var COL = C.colors || {};
    var cText         = COL.text          || "#e6edf3";
    var cTextSec      = COL.textSecondary || "#8b949e";
    var cTextMuted    = COL.textMuted     || "#484f58";
    var cBg           = COL.bg            || "#0d1117";
    var cSurface      = COL.surface       || "#161b22";
    var cBorder       = COL.border        || "#30363d";
    var cBorderSubtle = COL.borderSubtle  || "#21262d";
    var cBlue         = COL.blue          || "#58a6ff";
    var cBlueBright   = COL.blueBright    || "#79c0ff";
    var cPurpleLight  = COL.purpleLight   || "#d2a8ff";
    var cWhite        = COL.white         || "#ffffff";

    var css = [
      '.ops-view { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; color: ' + cText + '; }',
      '.ops-search-wrap { position: relative; margin-bottom: 12px; }',
      '.ops-search-icon {',
      '  position: absolute; left: 10px; top: 50%; transform: translateY(-50%);',
      '  width: 16px; height: 16px; pointer-events: none; color: ' + cTextSec + ';',
      '}',
      '.ops-search {',
      '  width: 100%; box-sizing: border-box; padding: 8px 12px 8px 34px;',
      '  background: ' + cSurface + '; border: 1px solid ' + cBorder + '; border-radius: 6px;',
      '  color: ' + cText + '; font-size: 14px; outline: none;',
      '}',
      '.ops-search:focus { border-color: ' + cBlue + '; box-shadow: 0 0 0 2px rgba(88,166,255,.25); }',
      '.ops-search::placeholder { color: ' + cTextMuted + '; }',
      '',
      '.ops-pills { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 14px; }',
      '.ops-pill {',
      '  padding: 4px 12px; border-radius: 16px; font-size: 12px; font-weight: 500;',
      '  cursor: pointer; border: 1px solid ' + cBorder + '; background: transparent; color: ' + cTextSec + ';',
      '  transition: background .15s, color .15s, border-color .15s; user-select: none;',
      '}',
      '.ops-pill:hover { border-color: ' + cBlue + '; color: ' + cText + '; }',
      '.ops-pill--active { background: ' + cBlue + '; color: ' + cWhite + '; border-color: ' + cBlue + '; }',
      '',
      '.ops-table { width: 100%; border-collapse: collapse; font-size: 13px; }',
      '.ops-table th {',
      '  text-align: left; padding: 8px 10px; color: ' + cTextSec + '; font-weight: 600;',
      '  border-bottom: 1px solid ' + cBorder + '; font-size: 12px; text-transform: uppercase; letter-spacing: .5px;',
      '}',
      '.ops-table td { padding: 8px 10px; border-bottom: 1px solid ' + cBorderSubtle + '; }',
      '.ops-table tr.ops-row { cursor: pointer; transition: background .1s; }',
      '.ops-table tr.ops-row:hover { background: ' + cSurface + '; }',
      '.ops-table tr.ops-row--even { background: rgba(22,27,34,.45); }',
      '.ops-table tr.ops-row--even:hover { background: ' + cSurface + '; }',
      '',
      '.ops-badge {',
      '  display: inline-block; padding: 2px 8px; border-radius: 12px; font-size: 11px;',
      '  font-weight: 600; line-height: 1.4;',
      '}',
      '.ops-latency-dot {',
      '  display: inline-flex; align-items: center; gap: 5px; font-size: 12px; color: ' + cTextSec + ';',
      '}',
      '.ops-latency-dot span.dot {',
      '  display: inline-block; width: 8px; height: 8px; border-radius: 50%;',
      '}',
      '',
      '.ops-detail {',
      '  border: 1px solid ' + cBorder + '; border-radius: 8px; background: ' + cBg + ';',
      '  padding: 20px; margin-top: 2px; animation: ops-slide-in .15s ease;',
      '}',
      '@keyframes ops-slide-in { from { opacity: 0; transform: translateY(-6px); } to { opacity: 1; transform: none; } }',
      '.ops-detail h2 { margin: 0 0 4px; font-size: 22px; font-weight: 700; color: ' + cText + '; font-family: "SFMono-Regular", Consolas, monospace; }',
      '.ops-detail p.ops-desc { margin: 8px 0 16px; color: ' + cTextSec + '; font-size: 14px; line-height: 1.5; }',
      '.ops-detail h3 { font-size: 14px; font-weight: 600; margin: 16px 0 8px; color: ' + cText + '; }',
      '',
      '.ops-fields-table { width: 100%; border-collapse: collapse; font-size: 13px; margin-bottom: 12px; }',
      '.ops-fields-table th {',
      '  text-align: left; padding: 6px 8px; color: ' + cTextSec + '; font-weight: 600;',
      '  border-bottom: 1px solid ' + cBorder + '; font-size: 11px; text-transform: uppercase;',
      '}',
      '.ops-fields-table td { padding: 6px 8px; border-bottom: 1px solid ' + cBorderSubtle + '; }',
      '.ops-fields-table td.field-name { font-family: monospace; color: ' + cBlueBright + '; }',
      '.ops-fields-table td.field-type { font-family: monospace; color: ' + cPurpleLight + '; font-size: 12px; }',
      '.ops-fields-table td.field-req { text-align: center; }',
      '',
      '.ops-mlir-info { display: flex; gap: 16px; margin-bottom: 12px; }',
      '.ops-mlir-tag {',
      '  display: inline-block; padding: 3px 10px; border-radius: 4px;',
      '  background: ' + cSurface + '; border: 1px solid ' + cBorder + '; font-family: monospace; font-size: 12px; color: ' + cText + ';',
      '}',
      '.ops-mlir-label { font-size: 11px; color: ' + cTextSec + '; margin-bottom: 2px; }',
      '',
      '.ops-code-block {',
      '  background: ' + cSurface + '; border: 1px solid ' + cBorder + '; border-radius: 6px;',
      '  padding: 12px 14px; overflow-x: auto; font-family: "SFMono-Regular", Consolas, monospace;',
      '  font-size: 12px; line-height: 1.55; white-space: pre; margin-bottom: 12px; color: ' + cText + ';',
      '}',
      '',
      '.ops-mini-graph { margin-top: 8px; }',
      '',
      '.ops-empty { color: ' + cTextMuted + '; text-align: center; padding: 40px 20px; font-size: 14px; }',
      '.ops-count { color: ' + cTextMuted + '; font-size: 12px; margin-bottom: 8px; }',
    ].join('\n');

    var styleEl = document.createElement('style');
    styleEl.textContent = css;
    document.head.appendChild(styleEl);
  }

  // ---------------------------------------------------------------------------
  // filterOps
  // ---------------------------------------------------------------------------

  function filterOps(ops, searchText, category) {
    if (!ops) return [];
    var filtered = ops;

    if (category && category !== 'All') {
      filtered = filtered.filter(function (op) {
        return categoryMatches(op.category, category);
      });
    }

    if (searchText && searchText.trim()) {
      var q = searchText.trim().toLowerCase();
      filtered = filtered.filter(function (op) {
        if ((op.mnemonic || '').toLowerCase().indexOf(q) !== -1) return true;
        if ((op.description || '').toLowerCase().indexOf(q) !== -1) return true;
        if ((op.op_type || '').toLowerCase().indexOf(q) !== -1) return true;
        if (op.fields && op.fields.length) {
          for (var i = 0; i < op.fields.length; i++) {
            if ((op.fields[i].name || '').toLowerCase().indexOf(q) !== -1) return true;
          }
        }
        return false;
      });
    }

    return filtered;
  }

  // ---------------------------------------------------------------------------
  // renderCategoryPills
  // ---------------------------------------------------------------------------

  function renderCategoryPills(container, categories, activeCategory, onSelect) {
    container.innerHTML = '';
    var wrap = el('div', { className: 'ops-pills' });

    categories.forEach(function (cat) {
      var isActive = cat === activeCategory;
      var pill = el('button', {
        className: 'ops-pill' + (isActive ? ' ops-pill--active' : ''),
        style: isActive
          ? { background: cat === 'All' ? _cBlue : categoryColor(cat), borderColor: cat === 'All' ? _cBlue : categoryColor(cat), color: _cWhite }
          : {},
        onClick: function () { onSelect(cat); },
      }, cat === 'ControlFlow' ? 'Control Flow' : cat === 'ErrorHandling' ? 'Error Handling' : cat);

      wrap.appendChild(pill);
    });

    container.appendChild(wrap);
  }

  // ---------------------------------------------------------------------------
  // Syntax highlighting for JSON code blocks
  // ---------------------------------------------------------------------------

  function highlightJSON(jsonStr) {
    // Attempt pretty-print if it's parseable
    try {
      var obj = JSON.parse(jsonStr);
      jsonStr = JSON.stringify(obj, null, 2);
    } catch (e) { /* keep as-is */ }

    // Escape HTML entities first
    var escaped = jsonStr
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');

    // Colorise: strings (keys vs values), numbers, booleans, null
    // Process token by token to distinguish keys from string values
    var result = '';
    var i = 0;
    var len = escaped.length;

    while (i < len) {
      var ch = escaped[i];

      if (ch === '"') {
        // Read the whole string token
        var start = i;
        i++; // skip opening quote
        while (i < len) {
          if (escaped[i] === '\\') { i += 2; continue; }
          if (escaped[i] === '"') { i++; break; }
          i++;
        }
        var token = escaped.slice(start, i);

        // Look ahead to see if followed by ':'  (it's a key)
        var rest = escaped.slice(i).replace(/^\s*/, '');
        if (rest[0] === ':') {
          result += '<span style="color:' + _cBlueBright + '">' + token + '</span>';
        } else {
          result += '<span style="color:' + _cBlueLight + '">' + token + '</span>';
        }
        continue;
      }

      // Numbers
      if (/[0-9\-]/.test(ch)) {
        var numStart = i;
        while (i < len && /[0-9eE.\-+]/.test(escaped[i])) i++;
        result += '<span style="color:' + _cBlueBright + '">' + escaped.slice(numStart, i) + '</span>';
        continue;
      }

      // Booleans and null
      if (escaped.slice(i, i + 4) === 'true') {
        result += '<span style="color:' + _cRedLight + '">true</span>';
        i += 4; continue;
      }
      if (escaped.slice(i, i + 5) === 'false') {
        result += '<span style="color:' + _cRedLight + '">false</span>';
        i += 5; continue;
      }
      if (escaped.slice(i, i + 4) === 'null') {
        result += '<span style="color:' + _cTextSec + '">null</span>';
        i += 4; continue;
      }

      result += ch;
      i++;
    }

    return result;
  }

  // ---------------------------------------------------------------------------
  // renderOpDetail
  // ---------------------------------------------------------------------------

  function renderOpDetail(container, op) {
    container.innerHTML = '';
    if (!op) return;

    var detail = el('div', { className: 'ops-detail' });

    // Header row: mnemonic + badge + latency
    var header = el('div', { style: { display: 'flex', alignItems: 'center', gap: '12px', flexWrap: 'wrap' } });
    header.appendChild(el('h2', null, op.mnemonic || op.op_type));
    header.appendChild(makeBadge(op.category));
    header.appendChild(makeLatencyDot(op.latency));
    detail.appendChild(header);

    // Description
    detail.appendChild(el('p', { className: 'ops-desc' }, op.description || ''));

    // Fields table
    if (op.fields && op.fields.length > 0) {
      detail.appendChild(el('h3', null, 'Fields'));
      var ftable = el('table', { className: 'ops-fields-table' });
      var thead = el('tr', null, [
        el('th', null, 'Name'),
        el('th', null, 'Type'),
        el('th', { style: { textAlign: 'center' } }, 'Required'),
        el('th', null, 'Description'),
      ]);
      ftable.appendChild(el('thead', null, thead));
      var tbody = el('tbody');

      op.fields.forEach(function (f) {
        var row = el('tr', null, [
          el('td', { className: 'field-name' }, f.name),
          el('td', { className: 'field-type' }, f.field_type || ''),
          el('td', { className: 'field-req', innerHTML: f.required ? '<span style="color:' + _cGreen + '">&#10003;</span>' : '<span style="color:' + _cRed + '">&#10007;</span>' }),
          el('td', null, f.description || ''),
        ]);
        tbody.appendChild(row);
      });

      ftable.appendChild(tbody);
      detail.appendChild(ftable);
    }

    // MLIR info
    if (op.mlir) {
      detail.appendChild(el('h3', null, 'MLIR'));
      var mlirRow = el('div', { className: 'ops-mlir-info' });

      var mnemWrap = el('div');
      mnemWrap.appendChild(el('div', { className: 'ops-mlir-label' }, 'Mnemonic'));
      mnemWrap.appendChild(el('span', { className: 'ops-mlir-tag' }, op.mlir.mnemonic || ''));
      mlirRow.appendChild(mnemWrap);

      var resWrap = el('div');
      resWrap.appendChild(el('div', { className: 'ops-mlir-label' }, 'Result Type'));
      resWrap.appendChild(el('span', { className: 'ops-mlir-tag' }, op.mlir.result_type || ''));
      mlirRow.appendChild(resWrap);

      detail.appendChild(mlirRow);
    }

    // Example JSON
    if (op.examples && op.examples.length > 0) {
      detail.appendChild(el('h3', null, 'Examples'));
      op.examples.forEach(function (example) {
        if (example.description) {
          detail.appendChild(el('div', { style: { color: _cTextSec, fontSize: '12px', marginBottom: '4px' } }, example.description));
        }
        var codeBlock = el('div', { className: 'ops-code-block', innerHTML: highlightJSON(example.json || '') });
        detail.appendChild(codeBlock);
      });
    }

    // Mini graph snippet
    detail.appendChild(el('h3', null, 'Graph Snippet'));
    var graphWrap = el('div', { className: 'ops-mini-graph' });
    renderMiniGraph(graphWrap, op);
    detail.appendChild(graphWrap);

    container.appendChild(detail);
  }

  // ---------------------------------------------------------------------------
  // Mini graph renderer -- draws a single-node graph via GraphRenderer or SVG
  // ---------------------------------------------------------------------------

  function renderMiniGraph(container, op) {
    // Build a minimal graph with one node
    var miniGraph = {
      name: op.mnemonic + ' example',
      nodes: [{
        id: 1,
        name: (op.mnemonic || op.op_type || 'OP').toLowerCase(),
        op: op.mnemonic || op.op_type,
        attributes: {},
      }],
      edges: [],
      parameters: [],
      metadata: {},
    };

    // If GraphRenderer is available, use it
    if (window.GraphRenderer && typeof window.GraphRenderer.renderGraph === 'function') {
      var canvas = el('div', { style: { width: '200px', height: '100px' } });
      container.appendChild(canvas);
      try {
        window.GraphRenderer.renderGraph(canvas, miniGraph);
        return;
      } catch (e) {
        // Fall through to SVG fallback
        container.removeChild(canvas);
      }
    }

    // SVG fallback: draw a simple node box
    var color = categoryColor(op.category);
    var label = op.mnemonic || op.op_type || '?';
    var sublabel = (op.mnemonic || op.op_type || 'op').toLowerCase();
    var textWidth = Math.max(label.length, sublabel.length) * 8 + 32;
    var boxW = Math.max(textWidth, 100);
    var boxH = 48;
    var svgW = boxW + 40;
    var svgH = boxH + 24;
    var x = 20;
    var y = 12;

    var svg = [
      '<svg xmlns="http://www.w3.org/2000/svg" width="' + svgW + '" height="' + svgH + '" viewBox="0 0 ' + svgW + ' ' + svgH + '">',
      '  <rect x="' + x + '" y="' + y + '" width="' + boxW + '" height="' + boxH + '" rx="6" ry="6"',
      '    fill="' + _cSurface + '" stroke="' + color + '" stroke-width="2"/>',
      '  <text x="' + (x + boxW / 2) + '" y="' + (y + 19) + '" text-anchor="middle"',
      '    fill="' + color + '" font-family="SFMono-Regular,Consolas,monospace" font-size="13" font-weight="700">' + escSvg(label) + '</text>',
      '  <text x="' + (x + boxW / 2) + '" y="' + (y + 36) + '" text-anchor="middle"',
      '    fill="' + _cTextSec + '" font-family="SFMono-Regular,Consolas,monospace" font-size="10">' + escSvg(sublabel) + '</text>',
      '</svg>',
    ].join('\n');

    container.innerHTML = svg;
  }

  function escSvg(str) {
    return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }

  // ---------------------------------------------------------------------------
  // Small presentational helpers
  // ---------------------------------------------------------------------------

  function makeBadge(category) {
    var color = categoryColor(category);
    var label = normaliseCategory(category);
    if (label === 'ControlFlow') label = 'Control Flow';
    if (label === 'ErrorHandling') label = 'Error Handling';
    return el('span', {
      className: 'ops-badge',
      style: { background: color + '22', color: color, border: '1px solid ' + color + '44' },
    }, label);
  }

  function makeLatencyDot(latency) {
    var severity = LATENCY_SEVERITY[latency] || 'gray';
    var dotColor;
    if (typeof C.getSeverityColor === 'function') {
      dotColor = C.getSeverityColor(severity);
    } else if (C.severityColors && C.severityColors[severity]) {
      dotColor = C.severityColors[severity];
    } else {
      // Fallback when APXM module is not loaded
      var fallback = { red: _cRed, yellow: _cYellow, green: _cGreen, gray: _cTextSec };
      dotColor = fallback[severity] || _cTextSec;
    }
    var label = latency || 'None';
    var wrap = el('span', { className: 'ops-latency-dot' });
    var dot = el('span', { className: 'dot', style: { background: dotColor } });
    wrap.appendChild(dot);
    wrap.appendChild(document.createTextNode(label));
    return wrap;
  }

  // ---------------------------------------------------------------------------
  // renderOpsTable
  // ---------------------------------------------------------------------------

  function renderOpsTable(container, ops, filter) {
    container.innerHTML = '';

    var filtered = filter ? filterOps(ops, filter.search, filter.category) : ops;

    if (!filtered || filtered.length === 0) {
      container.appendChild(el('div', { className: 'ops-empty' }, 'No operations match your search.'));
      return;
    }

    // Count
    container.appendChild(el('div', { className: 'ops-count' }, filtered.length + ' operation' + (filtered.length !== 1 ? 's' : '')));

    var table = el('table', { className: 'ops-table' });
    var thead = el('thead');
    thead.appendChild(el('tr', null, [
      el('th', null, 'Op'),
      el('th', null, 'Category'),
      el('th', null, 'Latency'),
      el('th', null, 'Description'),
    ]));
    table.appendChild(thead);

    var tbody = el('tbody');
    var expandedId = null;
    var detailRow = null;

    filtered.forEach(function (op, idx) {
      var isEven = idx % 2 === 1;
      var row = el('tr', {
        className: 'ops-row' + (isEven ? ' ops-row--even' : ''),
      }, [
        el('td', { style: { fontFamily: 'monospace', fontWeight: '600', color: _cText, whiteSpace: 'nowrap' } }, op.mnemonic || op.op_type),
        el('td', null, makeBadge(op.category)),
        el('td', null, makeLatencyDot(op.latency)),
        el('td', { style: { color: _cTextSec, maxWidth: '400px' } }, op.description || ''),
      ]);

      row.addEventListener('click', function () {
        var opId = op.mnemonic || op.op_type;

        // Collapse if same row clicked again
        if (expandedId === opId && detailRow && detailRow.parentNode) {
          detailRow.parentNode.removeChild(detailRow);
          detailRow = null;
          expandedId = null;
          return;
        }

        // Remove previous detail row
        if (detailRow && detailRow.parentNode) {
          detailRow.parentNode.removeChild(detailRow);
        }

        expandedId = opId;
        detailRow = el('tr');
        var detailCell = el('td', { colSpan: '4', style: { padding: '0', border: 'none' } });
        renderOpDetail(detailCell, op);
        detailRow.appendChild(detailCell);

        // Insert detail row after clicked row
        if (row.nextSibling) {
          row.parentNode.insertBefore(detailRow, row.nextSibling);
        } else {
          row.parentNode.appendChild(detailRow);
        }
      });

      tbody.appendChild(row);
    });

    table.appendChild(tbody);
    container.appendChild(table);
  }

  // ---------------------------------------------------------------------------
  // renderOpsView
  // ---------------------------------------------------------------------------

  function renderOpsView(container, ops) {
    injectStyles();
    container.innerHTML = '';

    var view = el('div', { className: 'ops-view' });

    // -- State --
    var state = {
      search: '',
      category: 'All',
    };

    // -- Search bar --
    var searchWrap = el('div', { className: 'ops-search-wrap' });

    // Search icon (magnifying glass SVG)
    var searchIcon = el('div', {
      className: 'ops-search-icon',
      innerHTML: '<svg viewBox="0 0 16 16" width="16" height="16" fill="currentColor"><path d="M10.68 11.74a6 6 0 1 1 1.06-1.06l3.04 3.04a.75.75 0 1 1-1.06 1.06l-3.04-3.04ZM11.5 7a4.5 4.5 0 1 0-9 0 4.5 4.5 0 0 0 9 0Z"/></svg>',
    });
    searchWrap.appendChild(searchIcon);

    var searchInput = el('input', {
      className: 'ops-search',
      type: 'text',
      placeholder: 'Search operations by name, description, or field...',
    });
    searchWrap.appendChild(searchInput);
    view.appendChild(searchWrap);

    // -- Category pills --
    var pillsContainer = el('div');
    view.appendChild(pillsContainer);

    // -- Table container --
    var tableContainer = el('div');
    view.appendChild(tableContainer);

    // -- Render helpers --
    function refresh() {
      renderCategoryPills(pillsContainer, CATEGORIES, state.category, function (cat) {
        state.category = cat;
        refresh();
      });
      renderOpsTable(tableContainer, ops, { search: state.search, category: state.category });
    }

    // -- Wire search with debounce --
    var debouncedRefresh = debounce(function () {
      state.search = searchInput.value;
      renderOpsTable(tableContainer, ops, { search: state.search, category: state.category });
    }, DEBOUNCE_MS);

    searchInput.addEventListener('input', debouncedRefresh);

    // -- Initial render --
    refresh();

    container.appendChild(view);
  }

  // ---------------------------------------------------------------------------
  // Export
  // ---------------------------------------------------------------------------

  window.OpsViewer = {
    renderOpsView: renderOpsView,
    renderOpsTable: renderOpsTable,
    renderOpDetail: renderOpDetail,
    filterOps: filterOps,
    renderCategoryPills: renderCategoryPills,
  };

})();
