// config.js -- APXM GUI config viewer module
// Displays backend/agent/tool configuration parsed from ~/.apxm/config.toml

(function () {
  'use strict';

  // ---------------------------------------------------------------------------
  // TOML parser (minimal, covers APXM config format)
  // ---------------------------------------------------------------------------

  /**
   * Parse a minimal TOML string into a nested object.
   *
   * Handles:
   *   [section.subsection]
   *   key = "string"
   *   key = 42  / key = true / key = false
   *   key = ["a", "b"]
   *   # comments
   *
   * Does NOT handle: inline tables, multiline strings, dotted keys outside
   * section headers, nested arrays, datetime, etc.
   */
  function parseToml(text) {
    var result = {};
    var currentSection = null;

    var lines = text.split('\n');
    for (var i = 0; i < lines.length; i++) {
      var line = lines[i].trim();

      // skip blank lines and comments
      if (line === '' || line.charAt(0) === '#') continue;

      // strip inline comments (only outside quoted strings)
      line = stripInlineComment(line);

      // section header: [a.b.c]
      var sectionMatch = line.match(/^\[([^\]]+)\]$/);
      if (sectionMatch) {
        currentSection = sectionMatch[1].trim();
        ensurePath(result, currentSection);
        continue;
      }

      // key = value
      var eqIdx = line.indexOf('=');
      if (eqIdx === -1) continue;

      var key = line.substring(0, eqIdx).trim();
      var rawVal = line.substring(eqIdx + 1).trim();
      var value = parseValue(rawVal);

      if (currentSection) {
        var obj = getPath(result, currentSection);
        obj[key] = value;
      } else {
        result[key] = value;
      }
    }

    return result;
  }

  /** Strip an inline comment that is NOT inside a quoted string. */
  function stripInlineComment(line) {
    var inStr = false;
    var quote = '';
    for (var i = 0; i < line.length; i++) {
      var ch = line.charAt(i);
      if (inStr) {
        if (ch === quote) inStr = false;
      } else {
        if (ch === '"' || ch === "'") {
          inStr = true;
          quote = ch;
        } else if (ch === '#') {
          return line.substring(0, i).trimEnd();
        }
      }
    }
    return line;
  }

  /** Parse a TOML value (string, number, bool, array). */
  function parseValue(raw) {
    // string
    if (raw.charAt(0) === '"') {
      var end = raw.lastIndexOf('"');
      if (end > 0) return raw.substring(1, end);
      return raw.substring(1);
    }
    if (raw.charAt(0) === "'") {
      var end2 = raw.lastIndexOf("'");
      if (end2 > 0) return raw.substring(1, end2);
      return raw.substring(1);
    }
    // bool
    if (raw === 'true') return true;
    if (raw === 'false') return false;
    // number
    if (/^-?\d+(\.\d+)?$/.test(raw)) return Number(raw);
    // array
    if (raw.charAt(0) === '[') return parseArray(raw);
    // fallback: bare string
    return raw;
  }

  /** Parse a TOML inline array like ["a", "b", 3]. */
  function parseArray(raw) {
    // Remove surrounding brackets
    var inner = raw.substring(1, raw.length - 1).trim();
    if (inner === '') return [];
    var items = [];
    var buf = '';
    var inStr = false;
    var quote = '';
    for (var i = 0; i < inner.length; i++) {
      var ch = inner.charAt(i);
      if (inStr) {
        buf += ch;
        if (ch === quote) inStr = false;
      } else if (ch === '"' || ch === "'") {
        inStr = true;
        quote = ch;
        buf += ch;
      } else if (ch === ',') {
        items.push(parseValue(buf.trim()));
        buf = '';
      } else {
        buf += ch;
      }
    }
    buf = buf.trim();
    if (buf !== '') items.push(parseValue(buf));
    return items;
  }

  /** Ensure nested path exists in object (e.g. "backends.cloud"). */
  function ensurePath(obj, path) {
    var parts = path.split('.');
    var cur = obj;
    for (var i = 0; i < parts.length; i++) {
      if (!cur[parts[i]]) cur[parts[i]] = {};
      cur = cur[parts[i]];
    }
    return cur;
  }

  /** Retrieve nested path from object. */
  function getPath(obj, path) {
    var parts = path.split('.');
    var cur = obj;
    for (var i = 0; i < parts.length; i++) {
      if (!cur[parts[i]]) cur[parts[i]] = {};
      cur = cur[parts[i]];
    }
    return cur;
  }

  // ---------------------------------------------------------------------------
  // TOML syntax highlighting
  // ---------------------------------------------------------------------------

  function escapeHtml(text) {
    return text
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
  }

  /**
   * Return an HTML string with TOML syntax highlighted via <span> classes:
   *   .toml-section   -- [section.headers]
   *   .toml-key       -- keys before =
   *   .toml-string    -- "quoted strings"
   *   .toml-comment   -- # comments
   *   .toml-bracket   -- array brackets [ ]
   */
  function highlightToml(text) {
    var lines = text.split('\n');
    var out = [];

    for (var i = 0; i < lines.length; i++) {
      var line = lines[i];

      // full-line comment
      var trimmed = line.trim();
      if (trimmed.charAt(0) === '#') {
        out.push('<span class="toml-comment">' + escapeHtml(line) + '</span>');
        continue;
      }

      // section header
      if (/^\s*\[.+\]\s*$/.test(line)) {
        out.push('<span class="toml-section">' + escapeHtml(line) + '</span>');
        continue;
      }

      // key = value line
      var eqIdx = line.indexOf('=');
      if (eqIdx !== -1) {
        var keyPart = line.substring(0, eqIdx);
        var sep = '=';
        var valPart = line.substring(eqIdx + 1);
        var highlighted = '<span class="toml-key">' + escapeHtml(keyPart) + '</span>' + sep;

        // Check for inline comment in value part
        var valuePortion = valPart;
        var commentPortion = '';
        var inString = false;
        var quoteChar = '';
        for (var j = 0; j < valPart.length; j++) {
          var c = valPart.charAt(j);
          if (inString) {
            if (c === quoteChar) inString = false;
          } else {
            if (c === '"' || c === "'") {
              inString = true;
              quoteChar = c;
            } else if (c === '#') {
              valuePortion = valPart.substring(0, j);
              commentPortion = valPart.substring(j);
              break;
            }
          }
        }

        highlighted += highlightValue(valuePortion);
        if (commentPortion) {
          highlighted += '<span class="toml-comment">' + escapeHtml(commentPortion) + '</span>';
        }
        out.push(highlighted);
        continue;
      }

      // plain line (blank or other)
      out.push(escapeHtml(line));
    }

    return out.join('\n');
  }

  /** Highlight a TOML value portion (strings, arrays, bare values). */
  function highlightValue(val) {
    var result = '';
    var i = 0;
    while (i < val.length) {
      var ch = val.charAt(i);

      // string literals
      if (ch === '"' || ch === "'") {
        var quote = ch;
        var end = val.indexOf(quote, i + 1);
        if (end === -1) end = val.length - 1;
        result += '<span class="toml-string">' + escapeHtml(val.substring(i, end + 1)) + '</span>';
        i = end + 1;
        continue;
      }

      // array brackets
      if (ch === '[' || ch === ']') {
        result += '<span class="toml-bracket">' + ch + '</span>';
        i++;
        continue;
      }

      // everything else (commas, spaces, numbers, booleans)
      result += escapeHtml(ch);
      i++;
    }
    return result;
  }

  // ---------------------------------------------------------------------------
  // CSS injection (idempotent)
  // ---------------------------------------------------------------------------

  var CSS_INJECTED = false;

  function injectStyles() {
    if (CSS_INJECTED) return;
    CSS_INJECTED = true;

    // Color shortcuts from shared constants
    var C = window.APXM || {};
    var COL = C.colors || {};
    var cText         = COL.text          || '#e6edf3';
    var cTextSec      = COL.textSecondary || '#8b949e';
    var cTextMuted    = COL.textMuted     || '#484f58';
    var cTextCode     = COL.textCode      || '#c9d1d9';
    var cBg           = COL.bg            || '#0d1117';
    var cSurface      = COL.surface       || '#161b22';
    var cBorder       = COL.border        || '#30363d';
    var cBorderSubtle = COL.borderSubtle  || '#21262d';
    var cBlue         = COL.blue          || '#58a6ff';
    var cGreen        = COL.green         || '#3fb950';
    var cPurpleLight  = COL.purpleLight   || '#d2a8ff';
    // Semantic badge backgrounds (derived tints)
    var cBadgeBgCloud = '#1f3a5f';
    var cBadgeBgLocal = '#1a3d2e';

    var style = document.createElement('style');
    style.textContent = [
      /* Card grid */
      '.config-section { margin-bottom: 24px; }',
      '.config-section-title {',
      '  font-size: 14px; font-weight: 600; text-transform: uppercase;',
      '  letter-spacing: 0.05em; color: ' + cTextSec + '; margin-bottom: 12px;',
      '}',
      '.config-cards {',
      '  display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));',
      '  gap: 12px;',
      '}',
      '.config-card {',
      '  background: ' + cSurface + '; border: 1px solid ' + cBorder + '; border-radius: 8px;',
      '  padding: 16px; display: flex; flex-direction: column; gap: 10px;',
      '}',
      '.config-card-header {',
      '  display: flex; align-items: center; justify-content: space-between;',
      '}',
      '.config-card-name {',
      '  font-size: 15px; font-weight: 600; color: ' + cText + ';',
      '}',

      /* Badges */
      '.config-badge {',
      '  font-size: 11px; font-weight: 500; padding: 2px 8px;',
      '  border-radius: 12px; text-transform: uppercase; letter-spacing: 0.04em;',
      '}',
      '.config-badge--cloud { background: ' + cBadgeBgCloud + '; color: ' + cBlue + '; }',
      '.config-badge--local { background: ' + cBadgeBgLocal + '; color: ' + cGreen + '; }',
      '.config-badge--configured {',
      '  background: ' + cBadgeBgLocal + '; color: ' + cGreen + '; font-size: 10px;',
      '}',

      /* Property rows */
      '.config-prop {',
      '  display: flex; align-items: baseline; gap: 8px; font-size: 13px;',
      '}',
      '.config-prop-label {',
      '  color: ' + cTextSec + '; min-width: 70px; flex-shrink: 0;',
      '}',
      '.config-prop-value {',
      '  color: ' + cTextCode + '; word-break: break-all;',
      '}',

      /* Model pills */
      '.config-models { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 2px; }',
      '.config-model-pill {',
      '  font-size: 11px; padding: 2px 8px; border-radius: 4px;',
      '  background: ' + cBorderSubtle + '; border: 1px solid ' + cBorder + '; color: ' + cTextCode + ';',
      '}',

      /* Status indicator */
      '.config-status {',
      '  display: flex; align-items: center; gap: 6px; font-size: 12px; color: ' + cTextSec + ';',
      '}',
      '.config-status-dot {',
      '  width: 6px; height: 6px; border-radius: 50%; background: ' + cGreen + ';',
      '}',

      /* Command display */
      '.config-command {',
      '  font-family: "SF Mono", "Fira Code", monospace; font-size: 12px;',
      '  color: ' + cTextCode + '; background: ' + cBg + '; border: 1px solid ' + cBorderSubtle + ';',
      '  border-radius: 4px; padding: 6px 8px; overflow: hidden;',
      '  text-overflow: ellipsis; white-space: nowrap;',
      '}',

      /* Collapsible raw config */
      '.config-raw-toggle {',
      '  display: flex; align-items: center; gap: 8px; cursor: pointer;',
      '  padding: 10px 0; user-select: none; border: none; background: none;',
      '  color: ' + cTextSec + '; font-size: 14px; font-weight: 600;',
      '  text-transform: uppercase; letter-spacing: 0.05em; width: 100%;',
      '  text-align: left;',
      '}',
      '.config-raw-toggle:hover { color: ' + cTextCode + '; }',
      '.config-raw-arrow {',
      '  display: inline-block; transition: transform 0.15s ease;',
      '  font-size: 10px;',
      '}',
      '.config-raw-arrow--open { transform: rotate(90deg); }',
      '.config-raw-content {',
      '  display: none; background: ' + cBg + '; border: 1px solid ' + cBorderSubtle + ';',
      '  border-radius: 8px; padding: 16px; overflow-x: auto;',
      '}',
      '.config-raw-content--open { display: block; }',
      '.config-raw-pre {',
      '  margin: 0; font-family: "SF Mono", "Fira Code", monospace;',
      '  font-size: 13px; line-height: 1.5; white-space: pre;',
      '  color: ' + cTextCode + ';',
      '}',

      /* Syntax highlighting */
      '.toml-section  { color: ' + cBlue + '; }',
      '.toml-key      { color: ' + cTextCode + '; }',
      '.toml-string   { color: ' + cGreen + '; }',
      '.toml-comment  { color: ' + cTextMuted + '; font-style: italic; }',
      '.toml-bracket  { color: ' + cPurpleLight + '; }',

      /* Empty state */
      '.config-empty {',
      '  color: ' + cTextMuted + '; font-size: 13px; font-style: italic; padding: 8px 0;',
      '}',
    ].join('\n');

    document.head.appendChild(style);
  }

  // ---------------------------------------------------------------------------
  // Rendering helpers
  // ---------------------------------------------------------------------------

  function el(tag, attrs, children) {
    var node = document.createElement(tag);
    if (attrs) {
      for (var k in attrs) {
        if (k === 'className') node.className = attrs[k];
        else if (k === 'textContent') node.textContent = attrs[k];
        else if (k === 'innerHTML') node.innerHTML = attrs[k];
        else node.setAttribute(k, attrs[k]);
      }
    }
    if (children) {
      if (!Array.isArray(children)) children = [children];
      for (var i = 0; i < children.length; i++) {
        if (typeof children[i] === 'string') {
          node.appendChild(document.createTextNode(children[i]));
        } else if (children[i]) {
          node.appendChild(children[i]);
        }
      }
    }
    return node;
  }

  function truncate(str, max) {
    if (!str) return '';
    if (str.length <= max) return str;
    return str.substring(0, max) + '\u2026';
  }

  // ---------------------------------------------------------------------------
  // renderBackendCards
  // ---------------------------------------------------------------------------

  function renderBackendCards(container, backends) {
    var section = el('div', { className: 'config-section' });
    section.appendChild(el('div', { className: 'config-section-title', textContent: 'Backends' }));

    var keys = Object.keys(backends || {});
    if (keys.length === 0) {
      section.appendChild(el('div', { className: 'config-empty', textContent: 'No backends configured' }));
      container.appendChild(section);
      return;
    }

    var grid = el('div', { className: 'config-cards' });

    for (var i = 0; i < keys.length; i++) {
      var name = keys[i];
      var b = backends[name];
      var card = el('div', { className: 'config-card' });

      // Header: name + type badge
      var header = el('div', { className: 'config-card-header' });
      header.appendChild(el('span', { className: 'config-card-name', textContent: name }));

      var typeVal = (b.type || 'unknown').toLowerCase();
      var badgeClass = 'config-badge config-badge--' + (typeVal === 'local' ? 'local' : 'cloud');
      header.appendChild(el('span', { className: badgeClass, textContent: typeVal }));
      card.appendChild(header);

      // Protocol
      if (b.protocol) {
        var protoProp = el('div', { className: 'config-prop' });
        protoProp.appendChild(el('span', { className: 'config-prop-label', textContent: 'Protocol' }));
        protoProp.appendChild(el('span', { className: 'config-prop-value', textContent: b.protocol }));
        card.appendChild(protoProp);
      }

      // Endpoint
      if (b.endpoint) {
        var endpProp = el('div', { className: 'config-prop' });
        endpProp.appendChild(el('span', { className: 'config-prop-label', textContent: 'Endpoint' }));
        endpProp.appendChild(el('span', { className: 'config-prop-value', textContent: b.endpoint }));
        card.appendChild(endpProp);
      }

      // API key env
      if (b.api_key_env) {
        var keyProp = el('div', { className: 'config-prop' });
        keyProp.appendChild(el('span', { className: 'config-prop-label', textContent: 'API Key' }));
        keyProp.appendChild(el('span', { className: 'config-prop-value', textContent: '$' + b.api_key_env }));
        card.appendChild(keyProp);
      }

      // Models
      var models = b.models || [];
      if (models.length > 0) {
        var modelsProp = el('div', { className: 'config-prop' });
        modelsProp.appendChild(el('span', { className: 'config-prop-label', textContent: 'Models' }));
        var pillsContainer = el('div', { className: 'config-models' });
        for (var m = 0; m < models.length; m++) {
          pillsContainer.appendChild(el('span', { className: 'config-model-pill', textContent: models[m] }));
        }
        modelsProp.appendChild(pillsContainer);
        card.appendChild(modelsProp);
      }

      // Status
      var status = el('div', { className: 'config-status' });
      status.appendChild(el('span', { className: 'config-status-dot' }));
      status.appendChild(document.createTextNode('Configured'));
      card.appendChild(status);

      grid.appendChild(card);
    }

    section.appendChild(grid);
    container.appendChild(section);
  }

  // ---------------------------------------------------------------------------
  // renderAgentCards
  // ---------------------------------------------------------------------------

  function renderAgentCards(container, agents) {
    var section = el('div', { className: 'config-section' });
    section.appendChild(el('div', { className: 'config-section-title', textContent: 'Agents' }));

    var keys = Object.keys(agents || {});
    if (keys.length === 0) {
      section.appendChild(el('div', { className: 'config-empty', textContent: 'No agents configured' }));
      container.appendChild(section);
      return;
    }

    var grid = el('div', { className: 'config-cards' });

    for (var i = 0; i < keys.length; i++) {
      var name = keys[i];
      var a = agents[name];
      var card = el('div', { className: 'config-card' });

      // Header
      var header = el('div', { className: 'config-card-header' });
      header.appendChild(el('span', { className: 'config-card-name', textContent: name }));
      header.appendChild(el('span', { className: 'config-badge config-badge--configured', textContent: 'agent' }));
      card.appendChild(header);

      // Command
      if (a.command) {
        var cmdProp = el('div', { className: 'config-prop' });
        cmdProp.appendChild(el('span', { className: 'config-prop-label', textContent: 'Command' }));
        var cmdDisplay = el('div', { className: 'config-command', textContent: truncate(a.command, 60) });
        cmdDisplay.title = a.command;
        cmdProp.appendChild(cmdDisplay);
        card.appendChild(cmdProp);
      }

      // Test prompt
      if (a.test_prompt) {
        var testProp = el('div', { className: 'config-prop' });
        testProp.appendChild(el('span', { className: 'config-prop-label', textContent: 'Test' }));
        testProp.appendChild(el('span', { className: 'config-prop-value', textContent: truncate(a.test_prompt, 80) }));
        card.appendChild(testProp);
      }

      grid.appendChild(card);
    }

    section.appendChild(grid);
    container.appendChild(section);
  }

  // ---------------------------------------------------------------------------
  // renderToolCards
  // ---------------------------------------------------------------------------

  function renderToolCards(container, tools) {
    var section = el('div', { className: 'config-section' });
    section.appendChild(el('div', { className: 'config-section-title', textContent: 'Tools' }));

    var keys = Object.keys(tools || {});
    if (keys.length === 0) {
      section.appendChild(el('div', { className: 'config-empty', textContent: 'No tools configured' }));
      container.appendChild(section);
      return;
    }

    var grid = el('div', { className: 'config-cards' });

    for (var i = 0; i < keys.length; i++) {
      var name = keys[i];
      var t = tools[name];
      var card = el('div', { className: 'config-card' });

      // Header
      var header = el('div', { className: 'config-card-header' });
      header.appendChild(el('span', { className: 'config-card-name', textContent: name }));
      header.appendChild(el('span', { className: 'config-badge config-badge--configured', textContent: 'tool' }));
      card.appendChild(header);

      // Description
      if (t.description) {
        var descProp = el('div', { className: 'config-prop' });
        descProp.appendChild(el('span', { className: 'config-prop-label', textContent: 'Desc' }));
        descProp.appendChild(el('span', { className: 'config-prop-value', textContent: t.description }));
        card.appendChild(descProp);
      }

      // Command
      if (t.command) {
        var cmdProp = el('div', { className: 'config-prop' });
        cmdProp.appendChild(el('span', { className: 'config-prop-label', textContent: 'Command' }));
        var cmdDisplay = el('div', { className: 'config-command', textContent: truncate(t.command, 60) });
        cmdDisplay.title = t.command;
        cmdProp.appendChild(cmdDisplay);
        card.appendChild(cmdProp);
      }

      grid.appendChild(card);
    }

    section.appendChild(grid);
    container.appendChild(section);
  }

  // ---------------------------------------------------------------------------
  // renderRawConfig
  // ---------------------------------------------------------------------------

  function renderRawConfig(container, text) {
    var section = el('div', { className: 'config-section' });

    // Toggle button
    var toggle = el('button', { className: 'config-raw-toggle' });
    var arrow = el('span', { className: 'config-raw-arrow', textContent: '\u25B6' });
    toggle.appendChild(arrow);
    toggle.appendChild(document.createTextNode('Raw Configuration'));
    section.appendChild(toggle);

    // Collapsible content
    var content = el('div', { className: 'config-raw-content' });
    var pre = el('pre', { className: 'config-raw-pre', innerHTML: highlightToml(text) });
    content.appendChild(pre);
    section.appendChild(content);

    // Toggle behavior
    toggle.addEventListener('click', function () {
      var isOpen = content.classList.toggle('config-raw-content--open');
      if (isOpen) {
        arrow.classList.add('config-raw-arrow--open');
      } else {
        arrow.classList.remove('config-raw-arrow--open');
      }
    });

    container.appendChild(section);
  }

  // ---------------------------------------------------------------------------
  // renderConfigView (main entry point)
  // ---------------------------------------------------------------------------

  /**
   * Render the full config viewer into the given container element.
   *
   * @param {HTMLElement} container -- DOM element to render into (cleared first)
   * @param {string}      configText -- raw TOML content of ~/.apxm/config.toml
   */
  function renderConfigView(container, configText) {
    injectStyles();

    container.innerHTML = '';

    var text = configText || '';
    if (text.trim() === '') {
      container.appendChild(
        el('div', { className: 'config-empty', textContent: 'No configuration loaded.' })
      );
      return;
    }

    var parsed = parseToml(text);

    renderBackendCards(container, parsed.backends || {});
    renderAgentCards(container, parsed.agents || {});
    renderToolCards(container, parsed.tools || {});
    renderRawConfig(container, text);
  }

  // ---------------------------------------------------------------------------
  // Public API
  // ---------------------------------------------------------------------------

  window.ConfigViewer = {
    renderConfigView: renderConfigView,
    parseToml: parseToml,
    renderBackendCards: renderBackendCards,
    renderAgentCards: renderAgentCards,
    renderToolCards: renderToolCards,
    renderRawConfig: renderRawConfig,
    highlightToml: highlightToml,
  };
})();
