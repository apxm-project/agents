// workspace.js -- APXM Agent Workspace Viewer
// Renders per-node workspace files from session execution.
// Browse agent instructions, prompts, responses, outputs, and traces.
// Export: window.WorkspaceViewer

(function () {
  "use strict";

  // ---------------------------------------------------------------------------
  // Constants
  // ---------------------------------------------------------------------------

  var FILE_ICONS = {
    ".md": "\u{1F4C4}",    // document
    ".json": "\u{007B}\u{007D}",  // braces
    ".txt": "\u{1F4DD}",   // memo
    ".ndjson": "\u{2261}", // list
  };

  var FOLDER_ICON_OPEN = "\u25BE";   // down triangle
  var FOLDER_ICON_CLOSED = "\u25B8"; // right triangle

  // File sort priority: .md first, .json, .txt, then others
  var FILE_SORT_ORDER = { ".md": 0, ".json": 1, ".txt": 2, ".ndjson": 3 };

  var C = window.APXM || {};
  var COL = C.colors || {};
  var NS = C.nodeStatus || {};

  var STATUS_COLORS = {
    completed: (NS.completed && NS.completed.color) || COL.green  || "#3fb950",
    failed:    (NS.failed    && NS.failed.color)    || COL.red    || "#f85149",
    running:   (NS.running   && NS.running.color)   || COL.yellow || "#d29922",
    pending:   (NS.pending   && NS.pending.color)   || COL.textSecondary || "#8b949e",
    skipped:   COL.textSecondary || "#6e7681",
  };

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  function el(tag, attrs, children) {
    var elem = document.createElement(tag);
    if (attrs) {
      Object.keys(attrs).forEach(function (key) {
        if (key === "className") {
          elem.className = attrs[key];
        } else if (key === "style" && typeof attrs[key] === "object") {
          Object.assign(elem.style, attrs[key]);
        } else if (key === "innerHTML") {
          elem.innerHTML = attrs[key];
        } else if (key.indexOf("on") === 0 && typeof attrs[key] === "function") {
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

  function escHtml(str) {
    var div = document.createElement("div");
    div.textContent = str;
    return div.innerHTML;
  }

  function fileExt(name) {
    var dot = name.lastIndexOf(".");
    if (dot === -1) return "";
    return name.slice(dot).toLowerCase();
  }

  function fileIcon(name) {
    var ext = fileExt(name);
    return FILE_ICONS[ext] || "\u{1F4CE}"; // paperclip fallback
  }

  function fileSortKey(name) {
    var ext = fileExt(name);
    var order = FILE_SORT_ORDER[ext];
    return order !== undefined ? order : 99;
  }

  function formatSize(bytes) {
    if (bytes == null) return "";
    if (bytes < 1024) return bytes + " B";
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
    return (bytes / (1024 * 1024)).toFixed(1) + " MB";
  }

  function estimateTokens(text) {
    if (!text) return 0;
    return Math.ceil(text.length / 4);
  }

  function copyToClipboard(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text);
    } else {
      var ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      try { document.execCommand("copy"); } catch (e) { /* ignore */ }
      document.body.removeChild(ta);
    }
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
    var cGreen        = COL.green         || "#3fb950";
    var cRed          = COL.red           || "#f85149";
    var cYellow       = COL.yellow        || "#d29922";
    var cPurple       = COL.purple        || "#bc8cff";
    var cBlueBright   = COL.blueBright    || "#79c0ff";
    var cBlueLight    = COL.blueLight     || "#a5d6ff";
    var cRedLight     = COL.redLight      || "#ff7b72";
    var cPurpleLight  = COL.purpleLight   || "#d2a8ff";
    var cTextCode     = COL.textCode      || "#c9d1d9";
    var cBlueFocus    = COL.blueFocus     || "#1f6feb";
    var cWhite        = COL.white         || "#ffffff";

    var css = [
      // Workspace layout
      ".workspace-container {",
      "  display: flex;",
      "  flex-direction: column;",
      "  height: 100%;",
      "  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif;",
      "  color: " + cText + ";",
      "  background: " + cBg + ";",
      "  border-radius: 8px;",
      "  overflow: hidden;",
      "}",

      // Header
      ".workspace-header {",
      "  display: flex;",
      "  align-items: center;",
      "  gap: 12px;",
      "  padding: 12px 16px;",
      "  border-bottom: 1px solid " + cBorderSubtle + ";",
      "  background: " + cSurface + ";",
      "  flex-shrink: 0;",
      "  flex-wrap: wrap;",
      "}",
      ".workspace-header__name {",
      "  font-size: 16px;",
      "  font-weight: 700;",
      "  color: " + cText + ";",
      "}",
      ".workspace-header__id {",
      "  font-size: 12px;",
      "  font-family: 'SFMono-Regular', Consolas, monospace;",
      "  color: " + cTextSec + ";",
      "}",
      ".workspace-header__op-badge {",
      "  display: inline-block;",
      "  padding: 2px 10px;",
      "  border-radius: 12px;",
      "  font-size: 11px;",
      "  font-weight: 700;",
      "  font-family: 'SFMono-Regular', Consolas, monospace;",
      "  background: rgba(88, 166, 255, 0.15);",
      "  color: " + cBlue + ";",
      "  border: 1px solid rgba(88, 166, 255, 0.3);",
      "}",
      ".workspace-header__status-badge {",
      "  display: inline-block;",
      "  padding: 2px 10px;",
      "  border-radius: 12px;",
      "  font-size: 11px;",
      "  font-weight: 600;",
      "}",
      ".workspace-header__path {",
      "  flex-basis: 100%;",
      "  font-size: 11px;",
      "  font-family: 'SFMono-Regular', Consolas, monospace;",
      "  color: " + cTextMuted + ";",
      "  overflow: hidden;",
      "  text-overflow: ellipsis;",
      "  white-space: nowrap;",
      "}",

      // Quick panels
      ".workspace-quick-panels {",
      "  display: flex;",
      "  gap: 10px;",
      "  padding: 10px 16px;",
      "  border-bottom: 1px solid " + cBorderSubtle + ";",
      "  flex-shrink: 0;",
      "  overflow-x: auto;",
      "}",
      ".workspace-quick-panels--stacked {",
      "  flex-direction: column;",
      "}",

      // Prompt/Response blocks
      ".prompt-block, .response-block {",
      "  flex: 1;",
      "  min-width: 0;",
      "  padding: 10px 14px;",
      "  border-radius: 8px;",
      "  background: " + cSurface + ";",
      "  border: 1px solid " + cBorder + ";",
      "  max-height: 200px;",
      "  overflow-y: auto;",
      "}",
      ".prompt-block {",
      "  border-left: 3px solid " + cBlue + ";",
      "}",
      ".response-block {",
      "  border-left: 3px solid " + cGreen + ";",
      "}",
      ".prompt-block__label, .response-block__label {",
      "  font-size: 11px;",
      "  font-weight: 700;",
      "  text-transform: uppercase;",
      "  letter-spacing: 0.08em;",
      "  margin-bottom: 6px;",
      "}",
      ".prompt-block__label {",
      "  color: " + cBlue + ";",
      "}",
      ".response-block__label {",
      "  color: " + cGreen + ";",
      "}",
      ".prompt-block__text, .response-block__text {",
      "  font-size: 13px;",
      "  line-height: 1.5;",
      "  white-space: pre-wrap;",
      "  word-break: break-word;",
      "  color: " + cTextCode + ";",
      "}",
      ".prompt-block__tokens, .response-block__tokens {",
      "  font-size: 10px;",
      "  color: " + cTextMuted + ";",
      "  margin-top: 6px;",
      "  text-align: right;",
      "}",

      // Output panel
      ".output-block {",
      "  flex: 1;",
      "  min-width: 0;",
      "  padding: 10px 14px;",
      "  border-radius: 8px;",
      "  background: " + cSurface + ";",
      "  border: 1px solid " + cBorder + ";",
      "  border-left: 3px solid " + cPurple + ";",
      "  max-height: 200px;",
      "  overflow-y: auto;",
      "}",
      ".output-block__label {",
      "  font-size: 11px;",
      "  font-weight: 700;",
      "  text-transform: uppercase;",
      "  letter-spacing: 0.08em;",
      "  margin-bottom: 6px;",
      "  color: " + cPurple + ";",
      "}",
      ".output-block__content {",
      "  font-size: 12px;",
      "  font-family: 'SFMono-Regular', Consolas, monospace;",
      "  line-height: 1.5;",
      "  white-space: pre-wrap;",
      "  word-break: break-word;",
      "  color: " + cTextCode + ";",
      "}",

      // Body: file tree + viewer split
      ".workspace-body {",
      "  display: flex;",
      "  flex: 1;",
      "  min-height: 0;",
      "  overflow: hidden;",
      "}",

      // File tree
      ".workspace-tree {",
      "  width: 250px;",
      "  min-width: 250px;",
      "  border-right: 1px solid " + cBorderSubtle + ";",
      "  background: " + cBg + ";",
      "  overflow-y: auto;",
      "  padding: 8px 0;",
      "  flex-shrink: 0;",
      "}",
      ".workspace-tree__title {",
      "  font-size: 11px;",
      "  font-weight: 700;",
      "  text-transform: uppercase;",
      "  letter-spacing: 0.08em;",
      "  color: " + cTextSec + ";",
      "  padding: 4px 14px 8px;",
      "}",

      // File items
      ".file-item {",
      "  display: flex;",
      "  align-items: center;",
      "  gap: 6px;",
      "  padding: 5px 14px;",
      "  cursor: pointer;",
      "  font-size: 13px;",
      "  color: " + cTextCode + ";",
      "  transition: background 100ms ease;",
      "  user-select: none;",
      "}",
      ".file-item:hover {",
      "  background: " + cSurface + ";",
      "}",
      ".file-item.active {",
      "  background: " + cBlueFocus + "22;",
      "  color: " + cBlue + ";",
      "}",
      ".file-item__icon {",
      "  flex-shrink: 0;",
      "  width: 18px;",
      "  text-align: center;",
      "  font-size: 12px;",
      "}",
      ".file-item__name {",
      "  flex: 1;",
      "  overflow: hidden;",
      "  text-overflow: ellipsis;",
      "  white-space: nowrap;",
      "  font-family: 'SFMono-Regular', Consolas, monospace;",
      "  font-size: 12px;",
      "}",
      ".file-item__size {",
      "  font-size: 10px;",
      "  color: " + cTextMuted + ";",
      "  flex-shrink: 0;",
      "}",

      // Directory items
      ".dir-item {",
      "  padding: 5px 14px;",
      "  cursor: pointer;",
      "  user-select: none;",
      "}",
      ".dir-item__header {",
      "  display: flex;",
      "  align-items: center;",
      "  gap: 6px;",
      "  font-size: 13px;",
      "  font-weight: 600;",
      "  color: " + cText + ";",
      "}",
      ".dir-item__toggle {",
      "  flex-shrink: 0;",
      "  width: 12px;",
      "  font-size: 10px;",
      "  color: " + cTextSec + ";",
      "}",
      ".dir-item__children {",
      "  padding-left: 18px;",
      "}",
      ".dir-item__children--collapsed {",
      "  display: none;",
      "}",

      // Workspace viewer (right pane)
      ".workspace-viewer {",
      "  flex: 1;",
      "  display: flex;",
      "  flex-direction: column;",
      "  min-width: 0;",
      "  overflow: hidden;",
      "}",
      ".workspace-viewer__empty {",
      "  display: flex;",
      "  align-items: center;",
      "  justify-content: center;",
      "  height: 100%;",
      "  color: " + cTextMuted + ";",
      "  font-size: 14px;",
      "}",

      // File content header
      ".file-content__header {",
      "  display: flex;",
      "  align-items: center;",
      "  gap: 10px;",
      "  padding: 8px 14px;",
      "  border-bottom: 1px solid " + cBorderSubtle + ";",
      "  background: " + cSurface + ";",
      "  flex-shrink: 0;",
      "}",
      ".file-content__name {",
      "  font-family: 'SFMono-Regular', Consolas, monospace;",
      "  font-size: 13px;",
      "  font-weight: 600;",
      "  color: " + cText + ";",
      "}",
      ".file-content__size {",
      "  font-size: 11px;",
      "  color: " + cTextMuted + ";",
      "}",
      ".file-content__spacer {",
      "  flex: 1;",
      "}",
      ".file-content__copy-btn {",
      "  padding: 3px 10px;",
      "  font-size: 11px;",
      "  font-weight: 600;",
      "  border: 1px solid " + cBorder + ";",
      "  border-radius: 6px;",
      "  background: " + cBorderSubtle + ";",
      "  color: " + cTextCode + ";",
      "  cursor: pointer;",
      "  transition: background 100ms ease;",
      "}",
      ".file-content__copy-btn:hover {",
      "  background: " + cBorder + ";",
      "}",
      ".file-content__wrap-btn {",
      "  padding: 3px 10px;",
      "  font-size: 11px;",
      "  font-weight: 600;",
      "  border: 1px solid " + cBorder + ";",
      "  border-radius: 6px;",
      "  background: " + cBorderSubtle + ";",
      "  color: " + cTextCode + ";",
      "  cursor: pointer;",
      "  transition: background 100ms ease;",
      "}",
      ".file-content__wrap-btn:hover {",
      "  background: " + cBorder + ";",
      "}",
      ".file-content__wrap-btn--active {",
      "  background: " + cBlueFocus + ";",
      "  border-color: " + cBlueFocus + ";",
      "  color: " + cWhite + ";",
      "}",

      // File content body
      ".file-content {",
      "  display: flex;",
      "  flex: 1;",
      "  min-height: 0;",
      "  overflow: auto;",
      "  background: " + cBg + ";",
      "}",
      ".file-content--wrap .file-content__code {",
      "  white-space: pre-wrap;",
      "  word-break: break-word;",
      "}",

      // Line numbers gutter
      ".line-numbers {",
      "  flex-shrink: 0;",
      "  padding: 12px 8px;",
      "  text-align: right;",
      "  font-family: 'SFMono-Regular', Consolas, monospace;",
      "  font-size: 12px;",
      "  line-height: 1.6;",
      "  color: " + cTextMuted + ";",
      "  user-select: none;",
      "  border-right: 1px solid " + cBorderSubtle + ";",
      "  background: " + cBg + ";",
      "}",

      // Code area
      ".file-content__code {",
      "  flex: 1;",
      "  padding: 12px 14px;",
      "  font-family: 'SFMono-Regular', Consolas, monospace;",
      "  font-size: 12px;",
      "  line-height: 1.6;",
      "  white-space: pre;",
      "  color: " + cText + ";",
      "  min-width: 0;",
      "  overflow-x: auto;",
      "}",

      // Markdown rendered content
      ".file-content__markdown {",
      "  flex: 1;",
      "  padding: 16px 20px;",
      "  font-size: 14px;",
      "  line-height: 1.6;",
      "  color: " + cTextCode + ";",
      "  overflow-y: auto;",
      "}",
      ".file-content__markdown h1 {",
      "  font-size: 24px;",
      "  font-weight: 700;",
      "  color: " + cText + ";",
      "  margin: 0 0 12px;",
      "  padding-bottom: 8px;",
      "  border-bottom: 1px solid " + cBorderSubtle + ";",
      "}",
      ".file-content__markdown h2 {",
      "  font-size: 20px;",
      "  font-weight: 700;",
      "  color: " + cText + ";",
      "  margin: 20px 0 8px;",
      "  padding-bottom: 6px;",
      "  border-bottom: 1px solid " + cBorderSubtle + ";",
      "}",
      ".file-content__markdown h3 {",
      "  font-size: 16px;",
      "  font-weight: 700;",
      "  color: " + cText + ";",
      "  margin: 16px 0 6px;",
      "}",
      ".file-content__markdown h4 {",
      "  font-size: 14px;",
      "  font-weight: 700;",
      "  color: " + cText + ";",
      "  margin: 12px 0 4px;",
      "}",
      ".file-content__markdown strong {",
      "  font-weight: 700;",
      "  color: " + cText + ";",
      "}",
      ".file-content__markdown em {",
      "  font-style: italic;",
      "  color: " + cPurpleLight + ";",
      "}",
      ".file-content__markdown code {",
      "  font-family: 'SFMono-Regular', Consolas, monospace;",
      "  font-size: 12px;",
      "  padding: 2px 6px;",
      "  border-radius: 4px;",
      "  background: " + cSurface + ";",
      "  border: 1px solid " + cBorder + ";",
      "  color: " + cBlueBright + ";",
      "}",
      ".file-content__markdown pre {",
      "  padding: 12px 14px;",
      "  border-radius: 6px;",
      "  background: " + cSurface + ";",
      "  border: 1px solid " + cBorder + ";",
      "  overflow-x: auto;",
      "  margin: 8px 0;",
      "}",
      ".file-content__markdown pre code {",
      "  padding: 0;",
      "  border: none;",
      "  background: none;",
      "  color: " + cText + ";",
      "  font-size: 12px;",
      "  line-height: 1.55;",
      "}",
      ".file-content__markdown ul, .file-content__markdown ol {",
      "  padding-left: 24px;",
      "  margin: 6px 0;",
      "}",
      ".file-content__markdown li {",
      "  margin: 3px 0;",
      "}",
      ".file-content__markdown blockquote {",
      "  border-left: 3px solid " + cBorder + ";",
      "  padding: 4px 14px;",
      "  margin: 8px 0;",
      "  color: " + cTextSec + ";",
      "}",
      ".file-content__markdown hr {",
      "  border: none;",
      "  border-top: 1px solid " + cBorderSubtle + ";",
      "  margin: 16px 0;",
      "}",

      // NDJSON line styling
      ".ndjson-line {",
      "  padding: 6px 10px;",
      "  font-family: 'SFMono-Regular', Consolas, monospace;",
      "  font-size: 12px;",
      "  line-height: 1.5;",
      "  border-bottom: 1px solid " + cBorderSubtle + ";",
      "}",
      ".ndjson-line--even {",
      "  background: rgba(22, 27, 34, 0.6);",
      "}",
      ".ndjson-line--odd {",
      "  background: " + cBg + ";",
      "}",

      // JSON syntax colors
      ".json-key { color: " + cBlueBright + "; }",
      ".json-string { color: " + cBlueLight + "; }",
      ".json-number { color: " + cYellow + "; }",
      ".json-boolean { color: " + cPurpleLight + "; }",
      ".json-null { color: " + cTextSec + "; }",
      ".json-bracket { color: " + cTextSec + "; }",

      // Responsive
      "@media (max-width: 700px) {",
      "  .workspace-body {",
      "    flex-direction: column;",
      "  }",
      "  .workspace-tree {",
      "    width: 100%;",
      "    min-width: 100%;",
      "    max-height: 200px;",
      "    border-right: none;",
      "    border-bottom: 1px solid " + cBorderSubtle + ";",
      "  }",
      "}",
    ].join("\n");

    var styleEl = document.createElement("style");
    styleEl.id = "workspace-viewer-styles";
    styleEl.textContent = css;
    document.head.appendChild(styleEl);
  }

  // ---------------------------------------------------------------------------
  // highlightJson -- JSON syntax highlighting returning HTML
  // ---------------------------------------------------------------------------

  function highlightJson(text) {
    // Try to pretty-print if parseable
    try {
      var obj = JSON.parse(text);
      text = JSON.stringify(obj, null, 2);
    } catch (e) { /* keep as-is */ }

    // Escape HTML entities
    var escaped = text
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");

    var result = "";
    var i = 0;
    var len = escaped.length;

    while (i < len) {
      var ch = escaped[i];

      // String tokens
      if (ch === '"') {
        var start = i;
        i++; // skip opening quote
        while (i < len) {
          if (escaped[i] === "\\" && i + 1 < len) { i += 2; continue; }
          if (escaped[i] === '"') { i++; break; }
          i++;
        }
        var token = escaped.slice(start, i);

        // Look ahead: is this a key (followed by ':') or a value?
        var rest = escaped.slice(i).replace(/^\s*/, "");
        if (rest[0] === ":") {
          result += '<span class="json-key">' + token + "</span>";
        } else {
          result += '<span class="json-string">' + token + "</span>";
        }
        continue;
      }

      // Numbers
      if (/[0-9\-]/.test(ch)) {
        var numStart = i;
        while (i < len && /[0-9eE.\-+]/.test(escaped[i])) i++;
        result += '<span class="json-number">' + escaped.slice(numStart, i) + "</span>";
        continue;
      }

      // Booleans
      if (escaped.slice(i, i + 4) === "true") {
        result += '<span class="json-boolean">true</span>';
        i += 4;
        continue;
      }
      if (escaped.slice(i, i + 5) === "false") {
        result += '<span class="json-boolean">false</span>';
        i += 5;
        continue;
      }

      // Null
      if (escaped.slice(i, i + 4) === "null") {
        result += '<span class="json-null">null</span>';
        i += 4;
        continue;
      }

      // Brackets and braces
      if (ch === "{" || ch === "}" || ch === "[" || ch === "]") {
        result += '<span class="json-bracket">' + ch + "</span>";
        i++;
        continue;
      }

      result += ch;
      i++;
    }

    return result;
  }

  // ---------------------------------------------------------------------------
  // highlightMarkdown -- basic markdown rendering returning HTML
  // ---------------------------------------------------------------------------

  function highlightMarkdown(text) {
    var html = escHtml(text);
    var lines = html.split("\n");
    var result = [];
    var inCodeBlock = false;
    var inList = false;
    var listType = null;

    for (var i = 0; i < lines.length; i++) {
      var line = lines[i];

      // Fenced code blocks
      if (line.match(/^```/)) {
        if (inCodeBlock) {
          result.push("</code></pre>");
          inCodeBlock = false;
        } else {
          closeList();
          inCodeBlock = true;
          result.push("<pre><code>");
        }
        continue;
      }

      if (inCodeBlock) {
        result.push(line);
        if (i < lines.length - 1) result.push("\n");
        continue;
      }

      // Horizontal rule
      if (line.match(/^---+\s*$/) || line.match(/^\*\*\*+\s*$/) || line.match(/^___+\s*$/)) {
        closeList();
        result.push("<hr>");
        continue;
      }

      // Headings
      var headingMatch = line.match(/^(#{1,6})\s+(.*)/);
      if (headingMatch) {
        closeList();
        var level = headingMatch[1].length;
        result.push("<h" + level + ">" + inlineFormat(headingMatch[2]) + "</h" + level + ">");
        continue;
      }

      // Blockquotes
      if (line.match(/^&gt;\s?/)) {
        closeList();
        var quoteContent = line.replace(/^&gt;\s?/, "");
        result.push("<blockquote>" + inlineFormat(quoteContent) + "</blockquote>");
        continue;
      }

      // Unordered list items
      var ulMatch = line.match(/^(\s*)[-*+]\s+(.*)/);
      if (ulMatch) {
        if (!inList || listType !== "ul") {
          closeList();
          result.push("<ul>");
          inList = true;
          listType = "ul";
        }
        result.push("<li>" + inlineFormat(ulMatch[2]) + "</li>");
        continue;
      }

      // Ordered list items
      var olMatch = line.match(/^(\s*)\d+\.\s+(.*)/);
      if (olMatch) {
        if (!inList || listType !== "ol") {
          closeList();
          result.push("<ol>");
          inList = true;
          listType = "ol";
        }
        result.push("<li>" + inlineFormat(olMatch[2]) + "</li>");
        continue;
      }

      // Close list if we hit a non-list line
      closeList();

      // Empty lines
      if (line.trim() === "") {
        result.push("");
        continue;
      }

      // Paragraph text
      result.push("<p>" + inlineFormat(line) + "</p>");
    }

    if (inCodeBlock) {
      result.push("</code></pre>");
    }
    closeList();

    return result.join("\n");

    function closeList() {
      if (inList) {
        result.push(listType === "ol" ? "</ol>" : "</ul>");
        inList = false;
        listType = null;
      }
    }

    function inlineFormat(str) {
      // Code spans (backtick) -- process first to avoid interfering with other patterns
      str = str.replace(/`([^`]+)`/g, "<code>$1</code>");
      // Bold
      str = str.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
      str = str.replace(/__([^_]+)__/g, "<strong>$1</strong>");
      // Italic
      str = str.replace(/\*([^*]+)\*/g, "<em>$1</em>");
      str = str.replace(/_([^_]+)_/g, "<em>$1</em>");
      return str;
    }
  }

  // ---------------------------------------------------------------------------
  // renderLineNumbers -- line numbers gutter
  // ---------------------------------------------------------------------------

  function renderLineNumbers(container, lineCount) {
    container.innerHTML = "";
    var gutter = el("div", { className: "line-numbers" });
    var lines = [];
    for (var i = 1; i <= lineCount; i++) {
      lines.push(String(i));
    }
    gutter.textContent = lines.join("\n");
    container.appendChild(gutter);
    return gutter;
  }

  // ---------------------------------------------------------------------------
  // renderFileContent -- display file content with formatting
  // ---------------------------------------------------------------------------

  function renderFileContent(container, fileName, content) {
    container.innerHTML = "";

    var ext = fileExt(fileName);
    var size = content ? content.length : 0;
    var wordWrap = false;

    // Header bar
    var header = el("div", { className: "file-content__header" });
    header.appendChild(el("span", { className: "file-content__name" }, fileName));
    header.appendChild(el("span", { className: "file-content__size" }, formatSize(size)));
    header.appendChild(el("div", { className: "file-content__spacer" }));

    // Word wrap toggle
    var wrapBtn = el("button", { className: "file-content__wrap-btn" }, "Wrap");
    header.appendChild(wrapBtn);

    // Copy button
    var copyBtn = el("button", { className: "file-content__copy-btn" }, "Copy");
    copyBtn.addEventListener("click", function () {
      copyToClipboard(content || "");
      copyBtn.textContent = "Copied!";
      setTimeout(function () { copyBtn.textContent = "Copy"; }, 1500);
    });
    header.appendChild(copyBtn);

    container.appendChild(header);

    // Content area
    var contentArea = el("div", { className: "file-content" });

    if (ext === ".md") {
      // Markdown: rendered view, no line numbers
      var mdDiv = el("div", { className: "file-content__markdown", innerHTML: highlightMarkdown(content || "") });
      contentArea.appendChild(mdDiv);
    } else if (ext === ".ndjson") {
      // NDJSON: each line as a separate JSON block
      var ndjsonDiv = el("div", { style: { flex: "1", overflowY: "auto" } });
      var lines = (content || "").split("\n").filter(function (l) { return l.trim() !== ""; });
      lines.forEach(function (line, idx) {
        var lineDiv = el("div", {
          className: "ndjson-line " + (idx % 2 === 0 ? "ndjson-line--even" : "ndjson-line--odd"),
          innerHTML: highlightJson(line),
        });
        ndjsonDiv.appendChild(lineDiv);
      });
      contentArea.appendChild(ndjsonDiv);
    } else if (ext === ".json") {
      // JSON: syntax highlighted with line numbers
      var lineCount = (content || "").split("\n").length;
      // Pretty-print if possible
      var displayText = content || "";
      try {
        var parsed = JSON.parse(displayText);
        displayText = JSON.stringify(parsed, null, 2);
        lineCount = displayText.split("\n").length;
      } catch (e) { /* keep as-is */ }

      var gutterEl = el("div", { className: "line-numbers" });
      renderLineNumbers(gutterEl, lineCount);
      contentArea.appendChild(gutterEl.firstChild || gutterEl);

      var codeDiv = el("div", {
        className: "file-content__code",
        innerHTML: highlightJson(displayText),
      });
      contentArea.appendChild(codeDiv);
    } else {
      // Plain text with line numbers
      var textLines = (content || "").split("\n");
      var textGutter = el("div", { className: "line-numbers" });
      renderLineNumbers(textGutter, textLines.length);
      contentArea.appendChild(textGutter.firstChild || textGutter);

      var textDiv = el("div", { className: "file-content__code" }, content || "");
      contentArea.appendChild(textDiv);
    }

    container.appendChild(contentArea);

    // Wire word wrap toggle
    wrapBtn.addEventListener("click", function () {
      wordWrap = !wordWrap;
      if (wordWrap) {
        contentArea.classList.add("file-content--wrap");
        wrapBtn.classList.add("file-content__wrap-btn--active");
      } else {
        contentArea.classList.remove("file-content--wrap");
        wrapBtn.classList.remove("file-content__wrap-btn--active");
      }
    });
  }

  // ---------------------------------------------------------------------------
  // renderFileTree -- collapsible file tree
  // ---------------------------------------------------------------------------

  function renderFileTree(container, files, subdirs, onFileSelect) {
    container.innerHTML = "";

    var treeEl = el("div", { className: "workspace-tree" });
    treeEl.appendChild(el("div", { className: "workspace-tree__title" }, "Files"));

    var activeEl = null;

    // Sort files: .md first, .json, .txt, .ndjson, then rest
    var sortedFiles = (files || []).slice().sort(function (a, b) {
      var orderA = fileSortKey(a.name);
      var orderB = fileSortKey(b.name);
      if (orderA !== orderB) return orderA - orderB;
      return a.name.localeCompare(b.name);
    });

    // Render top-level files
    sortedFiles.forEach(function (file) {
      var item = createFileItem(file.name, file.size, file.content);
      treeEl.appendChild(item);
    });

    // Render subdirectories
    (subdirs || []).forEach(function (subdir) {
      var dirEl = createDirItem(subdir.name, subdir.files || []);
      treeEl.appendChild(dirEl);
    });

    container.appendChild(treeEl);

    function createFileItem(name, size, content) {
      var item = el("div", { className: "file-item" });
      item.appendChild(el("span", { className: "file-item__icon" }, fileIcon(name)));
      item.appendChild(el("span", { className: "file-item__name" }, name));
      if (size != null) {
        item.appendChild(el("span", { className: "file-item__size" }, formatSize(size)));
      }

      item.addEventListener("click", function () {
        if (activeEl) activeEl.classList.remove("active");
        item.classList.add("active");
        activeEl = item;
        if (onFileSelect) onFileSelect(name, content);
      });

      return item;
    }

    function createDirItem(name, dirFiles) {
      var dirEl = el("div", { className: "dir-item" });
      var expanded = true;

      var headerEl = el("div", { className: "dir-item__header" });
      var toggleEl = el("span", { className: "dir-item__toggle" }, FOLDER_ICON_OPEN);
      headerEl.appendChild(toggleEl);
      headerEl.appendChild(document.createTextNode(name));
      dirEl.appendChild(headerEl);

      var childrenEl = el("div", { className: "dir-item__children" });

      // Sort subdir files
      var sortedDirFiles = (dirFiles || []).slice().sort(function (a, b) {
        var orderA = fileSortKey(a.name);
        var orderB = fileSortKey(b.name);
        if (orderA !== orderB) return orderA - orderB;
        return a.name.localeCompare(b.name);
      });

      sortedDirFiles.forEach(function (file) {
        var item = createFileItem(file.name, file.size, file.content);
        childrenEl.appendChild(item);
      });

      dirEl.appendChild(childrenEl);

      headerEl.addEventListener("click", function () {
        expanded = !expanded;
        toggleEl.textContent = expanded ? FOLDER_ICON_OPEN : FOLDER_ICON_CLOSED;
        if (expanded) {
          childrenEl.classList.remove("dir-item__children--collapsed");
        } else {
          childrenEl.classList.add("dir-item__children--collapsed");
        }
      });

      return dirEl;
    }
  }

  // ---------------------------------------------------------------------------
  // renderPromptResponse -- side-by-side or stacked prompt/response view
  // ---------------------------------------------------------------------------

  function renderPromptResponse(container, prompt, response) {
    container.innerHTML = "";

    var wrapper = el("div", { className: "workspace-quick-panels" });

    if (prompt != null) {
      var promptBlock = el("div", { className: "prompt-block" });
      promptBlock.appendChild(el("div", { className: "prompt-block__label" }, "Prompt"));
      promptBlock.appendChild(el("div", { className: "prompt-block__text" }, prompt));
      var promptTokens = estimateTokens(prompt);
      promptBlock.appendChild(el("div", { className: "prompt-block__tokens" }, "~" + promptTokens.toLocaleString() + " tokens"));
      wrapper.appendChild(promptBlock);
    }

    if (response != null) {
      var responseBlock = el("div", { className: "response-block" });
      responseBlock.appendChild(el("div", { className: "response-block__label" }, "Response"));
      responseBlock.appendChild(el("div", { className: "response-block__text" }, response));
      var responseTokens = estimateTokens(response);
      responseBlock.appendChild(el("div", { className: "response-block__tokens" }, "~" + responseTokens.toLocaleString() + " tokens"));
      wrapper.appendChild(responseBlock);
    }

    container.appendChild(wrapper);
  }

  // ---------------------------------------------------------------------------
  // renderWorkspace -- full workspace view
  // ---------------------------------------------------------------------------

  function renderWorkspace(container, nodeData) {
    injectStyles();
    container.innerHTML = "";

    if (!nodeData) {
      container.appendChild(el("div", { className: "workspace-viewer__empty" }, "No workspace data."));
      return;
    }

    var root = el("div", { className: "workspace-container" });

    // --- Header ---
    var header = el("div", { className: "workspace-header" });

    // Node name
    var nodeName = nodeData.workspace_path
      ? nodeData.workspace_path.split("/").pop()
      : "Node " + (nodeData.node_id || "?");
    header.appendChild(el("span", { className: "workspace-header__name" }, nodeName));

    // Node ID
    header.appendChild(el("span", { className: "workspace-header__id" }, "#" + (nodeData.node_id || "?")));

    // Op type badge -- extract from node.json if available
    var opType = extractField(nodeData, "node.json", "op");
    if (opType) {
      header.appendChild(el("span", { className: "workspace-header__op-badge" }, opType));
    }

    // Status badge -- extract from status.json
    var status = extractField(nodeData, "status.json", "status");
    if (status) {
      var statusColor = STATUS_COLORS[status] || STATUS_COLORS.pending;
      var statusBadge = el("span", {
        className: "workspace-header__status-badge",
        style: {
          background: statusColor + "22",
          color: statusColor,
          border: "1px solid " + statusColor + "44",
        },
      }, status);
      header.appendChild(statusBadge);
    }

    // Workspace path
    if (nodeData.workspace_path) {
      header.appendChild(el("div", { className: "workspace-header__path" }, nodeData.workspace_path));
    }

    root.appendChild(header);

    // --- Quick panels (prompt, response, output) ---
    var promptContent = findFileContent(nodeData, "prompt.txt");
    var responseContent = findFileContent(nodeData, "response.txt");
    var outputContent = findFileContent(nodeData, "output.json");

    if (promptContent != null || responseContent != null || outputContent != null) {
      var quickPanels = el("div", { className: "workspace-quick-panels" });

      if (promptContent != null) {
        var promptBlock = el("div", { className: "prompt-block" });
        promptBlock.appendChild(el("div", { className: "prompt-block__label" }, "Prompt"));
        promptBlock.appendChild(el("div", { className: "prompt-block__text" }, promptContent));
        var pTokens = estimateTokens(promptContent);
        promptBlock.appendChild(el("div", { className: "prompt-block__tokens" }, "~" + pTokens.toLocaleString() + " tokens"));
        quickPanels.appendChild(promptBlock);
      }

      if (responseContent != null) {
        var responseBlock = el("div", { className: "response-block" });
        responseBlock.appendChild(el("div", { className: "response-block__label" }, "Response"));
        responseBlock.appendChild(el("div", { className: "response-block__text" }, responseContent));
        var rTokens = estimateTokens(responseContent);
        responseBlock.appendChild(el("div", { className: "response-block__tokens" }, "~" + rTokens.toLocaleString() + " tokens"));
        quickPanels.appendChild(responseBlock);
      }

      if (outputContent != null) {
        var outputBlock = el("div", { className: "output-block" });
        outputBlock.appendChild(el("div", { className: "output-block__label" }, "Output"));
        var outputDisplay = outputContent;
        try {
          var parsedOutput = JSON.parse(outputContent);
          outputDisplay = JSON.stringify(parsedOutput, null, 2);
        } catch (e) { /* keep as-is */ }
        outputBlock.appendChild(el("div", {
          className: "output-block__content",
          innerHTML: highlightJson(outputDisplay),
        }));
        quickPanels.appendChild(outputBlock);
      }

      root.appendChild(quickPanels);
    }

    // --- Body: file tree + file viewer ---
    var body = el("div", { className: "workspace-body" });

    var treeContainer = el("div");
    var viewerContainer = el("div", { className: "workspace-viewer" });

    // Initial placeholder
    viewerContainer.appendChild(el("div", { className: "workspace-viewer__empty" }, "Select a file to view its contents."));

    // File select handler
    function onFileSelect(fileName, content) {
      renderFileContent(viewerContainer, fileName, content);
    }

    // Render file tree
    renderFileTree(treeContainer, nodeData.files, nodeData.subdirs, onFileSelect);

    // The tree container already has the workspace-tree class child inside it; move it out
    if (treeContainer.firstChild) {
      body.appendChild(treeContainer.firstChild);
    }
    body.appendChild(viewerContainer);

    root.appendChild(body);
    container.appendChild(root);
  }

  // ---------------------------------------------------------------------------
  // Internal helpers for extracting data from nodeData
  // ---------------------------------------------------------------------------

  function findFileContent(nodeData, fileName) {
    if (!nodeData || !nodeData.files) return null;
    for (var i = 0; i < nodeData.files.length; i++) {
      if (nodeData.files[i].name === fileName) {
        return nodeData.files[i].content || null;
      }
    }
    return null;
  }

  function extractField(nodeData, fileName, field) {
    var content = findFileContent(nodeData, fileName);
    if (!content) return null;
    try {
      var obj = JSON.parse(content);
      return obj[field] || null;
    } catch (e) {
      return null;
    }
  }

  // ---------------------------------------------------------------------------
  // Public API
  // ---------------------------------------------------------------------------

  window.WorkspaceViewer = {
    renderWorkspace: renderWorkspace,
    renderFileTree: renderFileTree,
    renderFileContent: renderFileContent,
    renderPromptResponse: renderPromptResponse,
    highlightJson: highlightJson,
    highlightMarkdown: highlightMarkdown,
    renderLineNumbers: renderLineNumbers,
  };
})();
