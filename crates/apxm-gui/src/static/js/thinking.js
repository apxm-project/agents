// thinking.js -- APXM Thinking/Reasoning Chain Visualization
// Renders extended thinking, tool calls, and LLM output for APXM workflow nodes.
// Export: window.ThinkingViewer

(function () {
  "use strict";

  // ---------------------------------------------------------------------------
  // Shared constants (from constants.js)
  // ---------------------------------------------------------------------------

  var C = window.APXM || {};
  var EK = (C.eventKinds || {});

  // ---------------------------------------------------------------------------
  // Constants
  // ---------------------------------------------------------------------------

  var COLLAPSE_THRESHOLD = 500; // chars before auto-collapse
  var KEYWORD_PATTERNS = [
    { re: /\b(I think)\b/gi, cls: "thinking-keyword--reason" },
    { re: /\b(because)\b/gi, cls: "thinking-keyword--reason" },
    { re: /\b(therefore)\b/gi, cls: "thinking-keyword--conclusion" },
    { re: /\b(however)\b/gi, cls: "thinking-keyword--contrast" },
    { re: /\b(the answer is)\b/gi, cls: "thinking-keyword--conclusion" },
  ];

  var ICON_BRAIN =
    '<svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">' +
    '<path d="M8 1C5.8 1 4 2.8 4 5c0 1.1.4 2 1 2.7V14h6V7.7c.6-.7 1-1.6 1-2.7 0-2.2-1.8-4-4-4z"/>' +
    '<path d="M6 8.5c-.8-.3-1.5-1-1.5-2"/>' +
    '<path d="M10 8.5c.8-.3 1.5-1 1.5-2"/>' +
    '<line x1="8" y1="5" x2="8" y2="8"/>' +
    "</svg>";

  var ICON_WRENCH =
    '<svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">' +
    '<path d="M10.3 1.7a4 4 0 0 0-5 5L2 10l1 3 3-1 3.3-3.3a4 4 0 0 0 5-5l-2.3 2.3-1.4-1.4L12.8 2.3a4 4 0 0 0-2.5-.6z"/>' +
    "</svg>";

  var ICON_OUTPUT =
    '<svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">' +
    '<rect x="2" y="2" width="12" height="12" rx="2"/>' +
    '<line x1="5" y1="6" x2="11" y2="6"/>' +
    '<line x1="5" y1="8.5" x2="9" y2="8.5"/>' +
    '<line x1="5" y1="11" x2="11" y2="11"/>' +
    "</svg>";

  var ICON_CHEVRON =
    '<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">' +
    '<polyline points="4,2 8,6 4,10"/>' +
    "</svg>";

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  function el(tag, attrs, children) {
    var node = document.createElement(tag);
    if (attrs) {
      for (var k in attrs) {
        if (!attrs.hasOwnProperty(k)) continue;
        if (k === "className") {
          node.className = attrs[k];
        } else if (k === "style" && typeof attrs[k] === "object") {
          Object.assign(node.style, attrs[k]);
        } else if (k === "innerHTML") {
          node.innerHTML = attrs[k];
        } else if (k.indexOf("on") === 0 && typeof attrs[k] === "function") {
          node.addEventListener(k.slice(2).toLowerCase(), attrs[k]);
        } else {
          node.setAttribute(k, attrs[k]);
        }
      }
    }
    if (children != null) {
      if (typeof children === "string") {
        node.textContent = children;
      } else if (Array.isArray(children)) {
        for (var i = 0; i < children.length; i++) {
          if (children[i] == null) continue;
          if (typeof children[i] === "string") {
            node.appendChild(document.createTextNode(children[i]));
          } else {
            node.appendChild(children[i]);
          }
        }
      } else {
        node.appendChild(children);
      }
    }
    return node;
  }

  function escapeHtml(str) {
    if (!str) return "";
    return str
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function wordCount(text) {
    if (!text) return 0;
    return text.trim().split(/\s+/).filter(Boolean).length;
  }

  function formatNumber(n) {
    if (n == null) return "--";
    return Number(n).toLocaleString();
  }

  var formatDuration = (function () {
    var shared = C.formatDuration;
    return function (ms) {
      if (ms == null) return null;
      if (typeof shared === "function") return shared(ms);
      if (ms < 1000) return ms + "ms";
      if (ms < 60000) {
        var s = ms / 1000;
        return (s % 1 === 0 ? s.toFixed(0) : s.toFixed(1)) + "s";
      }
      var m = Math.floor(ms / 60000);
      var sec = Math.round((ms % 60000) / 1000);
      return m + "m " + sec + "s";
    };
  })();

  var truncate = (function () {
    var shared = C.truncate;
    return function (str, max) {
      if (!str) return "";
      if (typeof shared === "function") return shared(str, max);
      if (str.length <= max) return str;
      return str.slice(0, max) + "\u2026";
    };
  })();

  // ---------------------------------------------------------------------------
  // Code block detection and rendering
  // ---------------------------------------------------------------------------

  /**
   * Split text into segments of plain text and fenced code blocks.
   * Returns array of { type: "text"|"code", content, lang? }
   */
  function splitCodeBlocks(text) {
    var segments = [];
    var re = /```(\w*)\n?([\s\S]*?)```/g;
    var lastIndex = 0;
    var match;

    while ((match = re.exec(text)) !== null) {
      if (match.index > lastIndex) {
        segments.push({ type: "text", content: text.slice(lastIndex, match.index) });
      }
      segments.push({ type: "code", content: match[2], lang: match[1] || null });
      lastIndex = re.lastIndex;
    }

    if (lastIndex < text.length) {
      segments.push({ type: "text", content: text.slice(lastIndex) });
    }

    if (segments.length === 0 && text) {
      segments.push({ type: "text", content: text });
    }

    return segments;
  }

  /**
   * Render a code block element with optional language badge.
   */
  function renderCodeBlock(content, lang) {
    var wrapper = el("div", { className: "code-block" });

    if (lang) {
      wrapper.appendChild(
        el("span", { className: "code-block__lang" }, lang)
      );
    }

    var pre = el("pre");
    var code = el("code");
    code.textContent = content;
    pre.appendChild(code);
    wrapper.appendChild(pre);

    return wrapper;
  }

  // ---------------------------------------------------------------------------
  // Markdown-like rendering
  // ---------------------------------------------------------------------------

  /**
   * Render text with basic markdown:
   *   **bold** -> <strong>
   *   `code`  -> <code>
   *   ```blocks``` -> <pre><code>
   *   - item  -> <ul><li>
   */
  function renderMarkdown(text) {
    var container = el("div", { className: "thinking-markdown" });
    var segments = splitCodeBlocks(text);

    for (var i = 0; i < segments.length; i++) {
      var seg = segments[i];

      if (seg.type === "code") {
        container.appendChild(renderCodeBlock(seg.content, seg.lang));
        continue;
      }

      // Process inline markdown for text segments
      var lines = seg.content.split("\n");
      var inList = false;
      var listEl = null;
      var html = "";

      for (var j = 0; j < lines.length; j++) {
        var line = lines[j];
        var trimmed = line.trim();

        // List items
        if (/^[-*]\s+/.test(trimmed)) {
          if (!inList) {
            if (html) {
              container.appendChild(el("div", { innerHTML: processInline(html) }));
              html = "";
            }
            inList = true;
            listEl = el("ul", { className: "thinking-list" });
          }
          var itemText = trimmed.replace(/^[-*]\s+/, "");
          listEl.appendChild(el("li", { innerHTML: processInline(escapeHtml(itemText)) }));
          continue;
        }

        // Numbered list items
        if (/^\d+[.)]\s+/.test(trimmed)) {
          if (!inList) {
            if (html) {
              container.appendChild(el("div", { innerHTML: processInline(html) }));
              html = "";
            }
            inList = true;
            listEl = el("ol", { className: "thinking-list" });
          }
          var olText = trimmed.replace(/^\d+[.)]\s+/, "");
          listEl.appendChild(el("li", { innerHTML: processInline(escapeHtml(olText)) }));
          continue;
        }

        // End of list
        if (inList && listEl) {
          container.appendChild(listEl);
          inList = false;
          listEl = null;
        }

        // Empty line -> paragraph break
        if (trimmed === "") {
          if (html) {
            container.appendChild(el("div", { innerHTML: processInline(html) }));
            html = "";
          }
          continue;
        }

        html += (html ? "\n" : "") + escapeHtml(line);
      }

      // Flush remaining content
      if (inList && listEl) {
        container.appendChild(listEl);
      }
      if (html) {
        container.appendChild(el("div", { innerHTML: processInline(html) }));
      }
    }

    return container;
  }

  /**
   * Process inline markdown: bold, inline code.
   * Expects already-escaped HTML input.
   */
  function processInline(html) {
    // Bold: **text** or __text__
    html = html.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
    html = html.replace(/__(.+?)__/g, "<strong>$1</strong>");

    // Inline code: `code`
    html = html.replace(/`([^`]+)`/g, '<code class="thinking-inline-code">$1</code>');

    return html;
  }

  // ---------------------------------------------------------------------------
  // Keyword highlighting for thinking text
  // ---------------------------------------------------------------------------

  function highlightKeywords(html) {
    for (var i = 0; i < KEYWORD_PATTERNS.length; i++) {
      var pattern = KEYWORD_PATTERNS[i];
      html = html.replace(pattern.re, '<span class="' + pattern.cls + '">$1</span>');
    }
    return html;
  }

  // ---------------------------------------------------------------------------
  // Thinking text rendering with structure detection
  // ---------------------------------------------------------------------------

  /**
   * Render thinking text with structure detection (numbered steps, bullet points)
   * and keyword highlighting.
   */
  function renderThinkingText(text) {
    var container = el("div", { className: "thinking-body__content" });
    var segments = splitCodeBlocks(text);

    for (var i = 0; i < segments.length; i++) {
      var seg = segments[i];

      if (seg.type === "code") {
        container.appendChild(renderCodeBlock(seg.content, seg.lang));
        continue;
      }

      var lines = seg.content.split("\n");
      var inList = false;
      var listEl = null;
      var paraLines = [];

      for (var j = 0; j < lines.length; j++) {
        var line = lines[j];
        var trimmed = line.trim();

        // Numbered steps
        if (/^\d+[.)]\s+/.test(trimmed)) {
          if (paraLines.length > 0) {
            flushParagraph(container, paraLines);
            paraLines = [];
          }
          if (!inList) {
            inList = true;
            listEl = el("ol", { className: "thinking-list thinking-list--structured" });
          }
          var stepText = trimmed.replace(/^\d+[.)]\s+/, "");
          listEl.appendChild(
            el("li", { innerHTML: highlightKeywords(processInline(escapeHtml(stepText))) })
          );
          continue;
        }

        // Bullet points
        if (/^[-*]\s+/.test(trimmed)) {
          if (paraLines.length > 0) {
            flushParagraph(container, paraLines);
            paraLines = [];
          }
          if (!inList) {
            inList = true;
            listEl = el("ul", { className: "thinking-list thinking-list--structured" });
          }
          var bulletText = trimmed.replace(/^[-*]\s+/, "");
          listEl.appendChild(
            el("li", { innerHTML: highlightKeywords(processInline(escapeHtml(bulletText))) })
          );
          continue;
        }

        // End of list
        if (inList && listEl) {
          container.appendChild(listEl);
          inList = false;
          listEl = null;
        }

        if (trimmed === "") {
          if (paraLines.length > 0) {
            flushParagraph(container, paraLines);
            paraLines = [];
          }
        } else {
          paraLines.push(line);
        }
      }

      // Flush remaining
      if (inList && listEl) {
        container.appendChild(listEl);
      }
      if (paraLines.length > 0) {
        flushParagraph(container, paraLines);
      }
    }

    return container;
  }

  function flushParagraph(container, lines) {
    var text = lines.join("\n");
    var html = highlightKeywords(processInline(escapeHtml(text)));
    container.appendChild(el("p", { className: "thinking-paragraph", innerHTML: html }));
  }

  // ---------------------------------------------------------------------------
  // JSON syntax highlighting
  // ---------------------------------------------------------------------------

  function highlightJSON(str) {
    try {
      var obj = JSON.parse(str);
      str = JSON.stringify(obj, null, 2);
    } catch (e) {
      // keep as-is
    }

    var escaped = escapeHtml(str);
    var result = "";
    var i = 0;
    var len = escaped.length;

    while (i < len) {
      var ch = escaped[i];

      if (ch === "&" && escaped.slice(i, i + 6) === "&quot;") {
        // Start of a quoted string in escaped form
        var start = i;
        i += 6; // skip &quot;
        while (i < len) {
          if (escaped[i] === "\\" && i + 1 < len) {
            i += 2;
            continue;
          }
          if (escaped.slice(i, i + 6) === "&quot;") {
            i += 6;
            break;
          }
          i++;
        }
        var token = escaped.slice(start, i);
        var rest = escaped.slice(i).replace(/^\s*/, "");
        if (rest[0] === ":") {
          result += '<span class="json-key">' + token + "</span>";
        } else {
          result += '<span class="json-string">' + token + "</span>";
        }
        continue;
      }

      // Plain quotes (if JSON wasn't fully escaped)
      if (ch === '"') {
        var qStart = i;
        i++;
        while (i < len) {
          if (escaped[i] === "\\" && i + 1 < len) {
            i += 2;
            continue;
          }
          if (escaped[i] === '"') {
            i++;
            break;
          }
          i++;
        }
        var qToken = escaped.slice(qStart, i);
        var qRest = escaped.slice(i).replace(/^\s*/, "");
        if (qRest[0] === ":") {
          result += '<span class="json-key">' + qToken + "</span>";
        } else {
          result += '<span class="json-string">' + qToken + "</span>";
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

      // Booleans & null
      if (escaped.slice(i, i + 4) === "true") {
        result += '<span class="json-bool">true</span>';
        i += 4;
        continue;
      }
      if (escaped.slice(i, i + 5) === "false") {
        result += '<span class="json-bool">false</span>';
        i += 5;
        continue;
      }
      if (escaped.slice(i, i + 4) === "null") {
        result += '<span class="json-null">null</span>';
        i += 4;
        continue;
      }

      result += ch;
      i++;
    }

    return result;
  }

  // ---------------------------------------------------------------------------
  // Event classification helpers
  // ---------------------------------------------------------------------------

  function classifyEvents(events) {
    var thoughts = [];
    var toolCalls = [];
    var toolResults = {};
    var tokens = [];
    var llmDone = null;
    var prompt = null;

    for (var i = 0; i < events.length; i++) {
      var evt = events[i];
      var payload = evt.payload || evt;
      var kind = payload.kind;

      switch (kind) {
        case EK.THOUGHT || "thought":
          thoughts.push(payload);
          break;
        case EK.TOKEN || "token":
          tokens.push(payload);
          break;
        case EK.TOOL_CALL || "tool_call":
          toolCalls.push({ call: payload, index: i });
          break;
        case EK.TOOL_END || "tool_end":
        case "tool_result":
          var callId = payload.id || payload.call_id;
          if (callId) toolResults[callId] = payload;
          break;
        case EK.LLM_DONE || "llm_done":
          llmDone = payload;
          break;
        case "prompt":
          prompt = payload;
          break;
        default:
          break;
      }
    }

    return {
      thoughts: thoughts,
      toolCalls: toolCalls,
      toolResults: toolResults,
      tokens: tokens,
      llmDone: llmDone,
      prompt: prompt,
    };
  }

  // ---------------------------------------------------------------------------
  // renderThinkingCard
  // ---------------------------------------------------------------------------

  function renderThinkingCard(container, thought, index) {
    var card = el("div", { className: "thinking-card" });

    // Header
    var header = el("div", { className: "thinking-header" });

    var iconWrap = el("span", { className: "thinking-header__icon", innerHTML: ICON_BRAIN });
    header.appendChild(iconWrap);

    var label = el("span", { className: "thinking-header__label" }, "Thinking\u2026");
    header.appendChild(label);

    if (index != null) {
      header.appendChild(
        el("span", { className: "thinking-header__index" }, "#" + (index + 1))
      );
    }

    var wc = wordCount(thought.text);
    header.appendChild(
      el("span", { className: "thinking-header__wordcount" }, wc + " words")
    );

    if (thought.duration_ms != null) {
      var dur = formatDuration(thought.duration_ms);
      if (dur) {
        header.appendChild(
          el("span", { className: "thinking-header__duration" }, dur)
        );
      }
    }

    // Chevron for collapse
    var chevron = el("span", { className: "thinking-header__chevron", innerHTML: ICON_CHEVRON });
    header.appendChild(chevron);

    card.appendChild(header);

    // Summary
    if (thought.summary) {
      card.appendChild(
        el("div", { className: "thinking-summary" }, thought.summary)
      );
    }

    // Body
    var body = el("div", { className: "thinking-body" });
    var textContent = renderThinkingText(thought.text || "");
    body.appendChild(textContent);
    card.appendChild(body);

    // Collapse logic
    var isLong = (thought.text || "").length > COLLAPSE_THRESHOLD;
    var collapsed = isLong;

    function setCollapsed(val) {
      collapsed = val;
      body.style.maxHeight = collapsed ? "0" : body.scrollHeight + "px";
      body.style.opacity = collapsed ? "0" : "1";
      card.classList.toggle("thinking-card--collapsed", collapsed);
      chevron.classList.toggle("thinking-header__chevron--open", !collapsed);
    }

    header.style.cursor = "pointer";
    header.addEventListener("click", function () {
      setCollapsed(!collapsed);
    });

    card.appendChild(body);

    // Initial state
    if (collapsed) {
      // Use requestAnimationFrame to set initial collapsed state after DOM insertion
      requestAnimationFrame(function () {
        setCollapsed(true);
      });
    } else {
      chevron.classList.add("thinking-header__chevron--open");
    }

    container.appendChild(card);
    return card;
  }

  // ---------------------------------------------------------------------------
  // renderToolCallCard
  // ---------------------------------------------------------------------------

  function renderToolCallCard(container, toolCall, toolResult) {
    var card = el("div", { className: "tool-card" });

    // Header
    var header = el("div", { className: "tool-card__header" });

    header.appendChild(
      el("span", { className: "tool-card__icon", innerHTML: ICON_WRENCH })
    );

    header.appendChild(
      el("span", { className: "tool-card__name" }, toolCall.name || "unknown")
    );

    if (toolCall.id) {
      header.appendChild(
        el("span", { className: "tool-card__id" }, toolCall.id)
      );
    }

    // Status badge
    var hasResult = !!toolResult;
    var isError = hasResult && (toolResult.error || toolResult.success === false);
    var statusText = !hasResult ? "pending" : isError ? "error" : "success";
    var statusClass = "tool-card__status tool-card__status--" + statusText;
    header.appendChild(el("span", { className: statusClass }, statusText));

    if (toolResult && toolResult.duration_ms != null) {
      var dur = formatDuration(toolResult.duration_ms);
      if (dur) {
        header.appendChild(
          el("span", { className: "tool-card__duration" }, dur)
        );
      }
    }

    card.appendChild(header);

    // Arguments
    var args = toolCall.arguments || toolCall.args;
    if (args) {
      var argsSection = el("div", { className: "tool-card__section" });
      argsSection.appendChild(
        el("div", { className: "tool-card__section-label" }, "Arguments")
      );

      var argsStr = typeof args === "string" ? args : JSON.stringify(args, null, 2);
      var argsCode = el("div", {
        className: "code-block code-block--json",
        innerHTML: "<pre><code>" + highlightJSON(argsStr) + "</code></pre>",
      });
      argsSection.appendChild(argsCode);
      card.appendChild(argsSection);
    }

    // Result
    if (toolResult) {
      var resultSection = el("div", { className: "tool-card__section" });
      resultSection.appendChild(
        el("div", { className: "tool-card__section-label" }, "Result")
      );

      var resultContent = toolResult.result || toolResult.output || toolResult.content || toolResult.error;
      if (resultContent != null) {
        var resultStr =
          typeof resultContent === "string"
            ? resultContent
            : JSON.stringify(resultContent, null, 2);

        if (resultStr.length > 500) {
          // Collapsible result
          var resultWrapper = el("div", { className: "tool-card__result-wrap" });
          var resultPre = el("pre", {
            className: "tool-card__result" + (isError ? " tool-card__result--error" : ""),
          });
          var resultCode = el("code");
          resultCode.textContent = resultStr;
          resultPre.appendChild(resultCode);
          resultWrapper.appendChild(resultPre);

          var toggleBtn = el(
            "button",
            {
              className: "tool-card__toggle",
              onClick: function () {
                var isOpen = resultWrapper.classList.toggle("tool-card__result-wrap--open");
                toggleBtn.textContent = isOpen ? "Collapse" : "Show full result (" + resultStr.length + " chars)";
              },
            },
            "Show full result (" + resultStr.length + " chars)"
          );
          resultSection.appendChild(toggleBtn);
          resultSection.appendChild(resultWrapper);
        } else {
          var simpleResult = el("pre", {
            className: "tool-card__result" + (isError ? " tool-card__result--error" : ""),
          });
          var simpleCode = el("code");
          simpleCode.textContent = resultStr;
          simpleResult.appendChild(simpleCode);
          resultSection.appendChild(simpleResult);
        }
      }

      card.appendChild(resultSection);
    }

    container.appendChild(card);
    return card;
  }

  // ---------------------------------------------------------------------------
  // renderOutputCard
  // ---------------------------------------------------------------------------

  function renderOutputCard(container, llmDone) {
    var card = el("div", { className: "output-card" });

    // Header
    var header = el("div", { className: "output-card__header" });
    header.appendChild(
      el("span", { className: "output-card__icon", innerHTML: ICON_OUTPUT })
    );
    header.appendChild(
      el("span", { className: "output-card__label" }, "Output")
    );

    // Model badge
    if (llmDone.model) {
      header.appendChild(
        el("span", { className: "output-card__model" }, llmDone.model)
      );
    }

    // Finish reason
    var reason = llmDone.finish_reason || llmDone.stop_reason;
    if (reason) {
      header.appendChild(
        el("span", { className: "output-card__reason" }, reason)
      );
    }

    card.appendChild(header);

    // Content
    if (llmDone.content) {
      var contentBody = renderMarkdown(llmDone.content);
      contentBody.className = "output-card__content";
      card.appendChild(contentBody);
    }

    // Usage stats
    var usage = llmDone.usage;
    if (usage) {
      var statsRow = el("div", { className: "output-card__stats" });

      var inputTok = usage.input_tokens || usage.prompt_tokens;
      var outputTok = usage.output_tokens || usage.completion_tokens;
      var totalTok =
        usage.total_tokens || (inputTok != null && outputTok != null ? inputTok + outputTok : null);

      if (inputTok != null) {
        statsRow.appendChild(
          el("span", { className: "output-card__stat" }, [
            el("span", { className: "output-card__stat-label" }, "Input: "),
            el("span", { className: "output-card__stat-value" }, formatNumber(inputTok)),
          ])
        );
      }
      if (outputTok != null) {
        statsRow.appendChild(
          el("span", { className: "output-card__stat" }, [
            el("span", { className: "output-card__stat-label" }, "Output: "),
            el("span", { className: "output-card__stat-value" }, formatNumber(outputTok)),
          ])
        );
      }
      if (totalTok != null) {
        statsRow.appendChild(
          el("span", { className: "output-card__stat" }, [
            el("span", { className: "output-card__stat-label" }, "Total: "),
            el("span", { className: "output-card__stat-value" }, formatNumber(totalTok)),
          ])
        );
      }

      card.appendChild(statsRow);
    }

    container.appendChild(card);
    return card;
  }

  // ---------------------------------------------------------------------------
  // renderThinkingView
  // ---------------------------------------------------------------------------

  function renderThinkingView(container, nodeId, traceEvents) {
    container.innerHTML = "";

    var root = el("div", { className: "thinking-view" });

    // Title
    var titleRow = el("div", { className: "thinking-view__title-row" });
    titleRow.appendChild(
      el("h3", { className: "thinking-view__title" }, "Reasoning Chain")
    );
    if (nodeId != null) {
      titleRow.appendChild(
        el("span", { className: "thinking-view__node-id" }, "Node " + nodeId)
      );
    }
    root.appendChild(titleRow);

    if (!traceEvents || traceEvents.length === 0) {
      root.appendChild(
        el("div", { className: "thinking-view__empty" }, "No thinking trace events available.")
      );
      container.appendChild(root);
      return;
    }

    var classified = classifyEvents(traceEvents);

    // Timeline bar (compact overview)
    var timelineContainer = el("div", { className: "thinking-timeline-container" });
    renderThinkingTimeline(timelineContainer, traceEvents);
    root.appendChild(timelineContainer);

    // Prompt section
    if (classified.prompt && classified.prompt.text) {
      var promptCard = el("div", { className: "thinking-prompt" });
      promptCard.appendChild(
        el("div", { className: "thinking-prompt__label" }, "Prompt")
      );
      var promptBody = el("div", { className: "thinking-prompt__body" });
      promptBody.appendChild(renderMarkdown(classified.prompt.text));
      promptCard.appendChild(promptBody);
      root.appendChild(promptCard);
    }

    // Build the interleaved sequence from the original event order
    // to preserve the chronological chain: thought -> tool_call -> thought -> output
    var sequence = buildSequence(traceEvents);

    for (var i = 0; i < sequence.length; i++) {
      var item = sequence[i];

      if (item.type === "thought") {
        renderThinkingCard(root, item.data, item.index);
      } else if (item.type === "tool_call") {
        var callId = item.data.id || item.data.call_id;
        var result = callId ? classified.toolResults[callId] : null;
        renderToolCallCard(root, item.data, result);
      } else if (item.type === "llm_done") {
        renderOutputCard(root, item.data);
      }
    }

    // If no llm_done event but we have tokens, show assembled output
    if (!classified.llmDone && classified.tokens.length > 0) {
      var assembledContent = classified.tokens
        .map(function (t) { return t.text || ""; })
        .join("");
      if (assembledContent.trim()) {
        renderOutputCard(root, { content: assembledContent });
      }
    }

    container.appendChild(root);
  }

  /**
   * Build a chronological sequence from raw events, preserving order.
   */
  function buildSequence(events) {
    var sequence = [];
    var thoughtIndex = 0;

    for (var i = 0; i < events.length; i++) {
      var payload = events[i].payload || events[i];
      var kind = payload.kind;

      if (kind === (EK.THOUGHT || "thought")) {
        sequence.push({ type: "thought", data: payload, index: thoughtIndex++ });
      } else if (kind === (EK.TOOL_CALL || "tool_call")) {
        sequence.push({ type: "tool_call", data: payload });
      } else if (kind === (EK.LLM_DONE || "llm_done")) {
        sequence.push({ type: "llm_done", data: payload });
      }
    }

    return sequence;
  }

  // ---------------------------------------------------------------------------
  // renderThinkingTimeline
  // ---------------------------------------------------------------------------

  function renderThinkingTimeline(container, events) {
    container.innerHTML = "";

    var bar = el("div", { className: "thinking-timeline" });

    if (!events || events.length === 0) {
      container.appendChild(bar);
      return;
    }

    // Compute time spans for each phase
    var phases = [];
    var firstTs = null;
    var lastTs = null;

    for (var i = 0; i < events.length; i++) {
      var evt = events[i];
      var payload = evt.payload || evt;
      var meta = evt.meta || {};
      var kind = payload.kind;
      var ts = meta.timestamp ? new Date(meta.timestamp).getTime() : null;

      if (ts && !firstTs) firstTs = ts;
      if (ts) lastTs = ts;

      if (kind === (EK.THOUGHT || "thought") || kind === (EK.TOOL_CALL || "tool_call") || kind === (EK.LLM_DONE || "llm_done")) {
        phases.push({
          kind: kind,
          ts: ts,
          payload: payload,
        });
      }
    }

    // If no timestamps, render equal-width segments
    var totalSpan = firstTs && lastTs ? lastTs - firstTs : 0;
    if (totalSpan <= 0) totalSpan = phases.length; // fallback: 1 unit per phase

    for (var j = 0; j < phases.length; j++) {
      var phase = phases[j];
      var cls = "thinking-timeline__segment thinking-timeline__segment--" + phase.kind;

      // Compute proportional width
      var width;
      if (firstTs && lastTs && totalSpan > 0 && phases.length > 1) {
        var segStart = phase.ts ? phase.ts - firstTs : 0;
        var segEnd =
          j + 1 < phases.length && phases[j + 1].ts
            ? phases[j + 1].ts - firstTs
            : totalSpan;
        var segDuration = segEnd - segStart;
        width = Math.max((segDuration / totalSpan) * 100, 3); // minimum 3%
      } else {
        width = Math.max(100 / phases.length, 3);
      }

      var segment = el("div", {
        className: cls,
        style: { width: width + "%" },
        title: phase.kind +
          (phase.kind === (EK.TOOL_CALL || "tool_call") && phase.payload.name ? ": " + phase.payload.name : ""),
      });

      // Make clickable to scroll to matching card
      (function (phaseKind, phaseIndex) {
        segment.style.cursor = "pointer";
        segment.addEventListener("click", function () {
          var root = container.closest(".thinking-view");
          if (!root) return;

          var selector;
          if (phaseKind === (EK.THOUGHT || "thought")) {
            selector = ".thinking-card";
          } else if (phaseKind === (EK.TOOL_CALL || "tool_call")) {
            selector = ".tool-card";
          } else if (phaseKind === (EK.LLM_DONE || "llm_done")) {
            selector = ".output-card";
          }

          if (selector) {
            var cards = root.querySelectorAll(selector);
            // Count how many of this kind we've seen so far
            var kindCount = 0;
            for (var k = 0; k < phaseIndex; k++) {
              if (phases[k].kind === phaseKind) kindCount++;
            }
            var target = cards[kindCount];
            if (target) {
              target.scrollIntoView({ behavior: "smooth", block: "center" });
              target.classList.add("thinking-flash");
              setTimeout(function () {
                target.classList.remove("thinking-flash");
              }, 1200);
            }
          }
        });
      })(phase.kind, j);

      bar.appendChild(segment);
    }

    container.appendChild(bar);
  }

  // ---------------------------------------------------------------------------
  // streamThinking
  // ---------------------------------------------------------------------------

  function streamThinking(container, options) {
    options = options || {};
    container.innerHTML = "";

    var root = el("div", { className: "thinking-view thinking-view--streaming" });
    container.appendChild(root);

    // Current blocks
    var currentThinkingCard = null;
    var currentThinkingBody = null;
    var outputBlock = null;
    var outputBody = null;
    var userHasScrolled = false;
    var lastScrollTop = 0;

    // Detect manual scrolling
    var scrollParent = container;
    if (options.scrollParent) scrollParent = options.scrollParent;

    scrollParent.addEventListener("scroll", function () {
      var atBottom =
        scrollParent.scrollTop + scrollParent.clientHeight >=
        scrollParent.scrollHeight - 40;
      if (!atBottom && scrollParent.scrollTop < lastScrollTop) {
        userHasScrolled = true;
      }
      if (atBottom) {
        userHasScrolled = false;
      }
      lastScrollTop = scrollParent.scrollTop;
    });

    function autoScroll() {
      if (!userHasScrolled) {
        scrollParent.scrollTop = scrollParent.scrollHeight;
      }
    }

    function ensureThinkingCard() {
      if (!currentThinkingCard) {
        currentThinkingCard = el("div", { className: "thinking-card thinking-card--streaming" });

        var header = el("div", { className: "thinking-header" });
        header.appendChild(
          el("span", { className: "thinking-header__icon", innerHTML: ICON_BRAIN })
        );
        header.appendChild(
          el("span", { className: "thinking-header__label" }, "Thinking\u2026")
        );

        var pulse = el("span", { className: "thinking-header__pulse" });
        header.appendChild(pulse);

        currentThinkingCard.appendChild(header);

        currentThinkingBody = el("div", { className: "thinking-body thinking-body--streaming" });
        currentThinkingCard.appendChild(currentThinkingBody);

        root.appendChild(currentThinkingCard);
      }
    }

    function ensureOutputBlock() {
      if (!outputBlock) {
        outputBlock = el("div", { className: "output-card output-card--streaming" });

        var header = el("div", { className: "output-card__header" });
        header.appendChild(
          el("span", { className: "output-card__icon", innerHTML: ICON_OUTPUT })
        );
        header.appendChild(
          el("span", { className: "output-card__label" }, "Output")
        );
        outputBlock.appendChild(header);

        outputBody = el("div", { className: "output-card__content output-card__content--streaming" });
        outputBlock.appendChild(outputBody);

        root.appendChild(outputBlock);
      }
    }

    return {
      appendThought: function (text) {
        ensureThinkingCard();
        // Append text with typing feel
        var span = el("span", { className: "thinking-stream-chunk" });
        span.textContent = text;
        currentThinkingBody.appendChild(span);
        autoScroll();
      },

      appendToken: function (text) {
        // Finalize current thinking card if any
        if (currentThinkingCard) {
          currentThinkingCard.classList.remove("thinking-card--streaming");
          var pulse = currentThinkingCard.querySelector(".thinking-header__pulse");
          if (pulse) pulse.remove();
          currentThinkingCard = null;
          currentThinkingBody = null;
        }

        ensureOutputBlock();
        var tokenSpan = document.createTextNode(text);
        outputBody.appendChild(tokenSpan);
        autoScroll();
      },

      addToolCall: function (toolCall) {
        // Finalize current thinking card
        if (currentThinkingCard) {
          currentThinkingCard.classList.remove("thinking-card--streaming");
          var pulse = currentThinkingCard.querySelector(".thinking-header__pulse");
          if (pulse) pulse.remove();
          currentThinkingCard = null;
          currentThinkingBody = null;
        }

        renderToolCallCard(root, toolCall, null);
        autoScroll();
      },

      finalize: function (llmDone) {
        // Finalize any streaming blocks
        if (currentThinkingCard) {
          currentThinkingCard.classList.remove("thinking-card--streaming");
          var pulse = currentThinkingCard.querySelector(".thinking-header__pulse");
          if (pulse) pulse.remove();
          currentThinkingCard = null;
          currentThinkingBody = null;
        }

        // If we have an output block streaming, finalize it
        if (outputBlock) {
          outputBlock.classList.remove("output-card--streaming");
        }

        // Add stats if llmDone provided
        if (llmDone) {
          // Model badge
          if (llmDone.model && outputBlock) {
            var header = outputBlock.querySelector(".output-card__header");
            if (header) {
              header.appendChild(
                el("span", { className: "output-card__model" }, llmDone.model)
              );
            }
          }

          // Usage stats
          var usage = llmDone.usage;
          if (usage) {
            var target = outputBlock || root;
            var statsRow = el("div", { className: "output-card__stats" });

            var inputTok = usage.input_tokens || usage.prompt_tokens;
            var outputTok = usage.output_tokens || usage.completion_tokens;
            var totalTok =
              usage.total_tokens ||
              (inputTok != null && outputTok != null ? inputTok + outputTok : null);

            if (inputTok != null) {
              statsRow.appendChild(
                el("span", { className: "output-card__stat" }, [
                  el("span", { className: "output-card__stat-label" }, "Input: "),
                  el("span", { className: "output-card__stat-value" }, formatNumber(inputTok)),
                ])
              );
            }
            if (outputTok != null) {
              statsRow.appendChild(
                el("span", { className: "output-card__stat" }, [
                  el("span", { className: "output-card__stat-label" }, "Output: "),
                  el("span", { className: "output-card__stat-value" }, formatNumber(outputTok)),
                ])
              );
            }
            if (totalTok != null) {
              statsRow.appendChild(
                el("span", { className: "output-card__stat" }, [
                  el("span", { className: "output-card__stat-label" }, "Total: "),
                  el("span", { className: "output-card__stat-value" }, formatNumber(totalTok)),
                ])
              );
            }

            target.appendChild(statsRow);
          }
        }

        autoScroll();
      },
    };
  }

  // ---------------------------------------------------------------------------
  // Inject scoped styles (once)
  // ---------------------------------------------------------------------------

  function injectStyles() {
    if (document.getElementById("thinking-viewer-styles")) return;

    // Color shortcuts from shared constants
    var COL = C.colors || {};
    var cText         = COL.text          || "#e6edf3";
    var cTextSec      = COL.textSecondary || "#8b949e";
    var cTextMuted    = COL.textMuted     || "#484f58";
    var cTextCode     = COL.textCode      || "#c9d1d9";
    var cBorder       = COL.border        || "#30363d";
    var cBorderSubtle = COL.borderSubtle  || "#21262d";
    var cBg           = COL.bg            || "#0d1117";
    var cSurface      = COL.surface       || "#161b22";
    var cBlue         = COL.blue          || "#58a6ff";
    var cBlueBright   = COL.blueBright    || "#79c0ff";
    var cBlueLight    = COL.blueLight     || "#a5d6ff";
    var cGreen        = COL.green         || "#3fb950";
    var cRed          = COL.red           || "#f85149";
    var cRedLight     = COL.redLight      || "#ff7b72";
    var cYellow       = COL.yellow        || "#d29922";
    var cPurple       = COL.purple        || "#bc8cff";
    var cPurpleLight  = COL.purpleLight   || "#d2a8ff";

    var css = [
      // ── View container ──
      ".thinking-view {",
      "  display: flex;",
      "  flex-direction: column;",
      "  gap: 12px;",
      "  padding: 12px;",
      "  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif;",
      "  color: " + cText + ";",
      "}",

      ".thinking-view__title-row {",
      "  display: flex;",
      "  align-items: center;",
      "  gap: 12px;",
      "}",

      ".thinking-view__title {",
      "  font-size: 16px;",
      "  font-weight: 700;",
      "  color: " + cText + ";",
      "  margin: 0;",
      "}",

      ".thinking-view__node-id {",
      "  font-size: 12px;",
      "  font-family: 'SF Mono', 'Fira Code', monospace;",
      "  color: " + cTextSec + ";",
      "  padding: 2px 8px;",
      "  border-radius: 4px;",
      "  background: rgba(255, 255, 255, 0.06);",
      "}",

      ".thinking-view__empty {",
      "  color: " + cTextMuted + ";",
      "  font-size: 13px;",
      "  font-style: italic;",
      "  padding: 20px;",
      "  text-align: center;",
      "}",

      // ── Prompt card ──
      ".thinking-prompt {",
      "  border: 1px solid " + cBorder + ";",
      "  border-left: 3px solid " + cPurpleLight + ";",
      "  border-radius: 8px;",
      "  background: rgba(210, 168, 255, 0.04);",
      "  padding: 12px 16px;",
      "}",

      ".thinking-prompt__label {",
      "  font-size: 11px;",
      "  font-weight: 700;",
      "  text-transform: uppercase;",
      "  letter-spacing: 0.08em;",
      "  color: " + cPurpleLight + ";",
      "  margin-bottom: 8px;",
      "}",

      ".thinking-prompt__body {",
      "  font-size: 13px;",
      "  line-height: 1.6;",
      "  color: " + cTextCode + ";",
      "}",

      // ── Thinking card ──
      ".thinking-card {",
      "  border: 1px solid " + cBorder + ";",
      "  border-left: 3px solid " + cTextSec + ";",
      "  border-radius: 8px;",
      "  background: rgba(139, 148, 158, 0.04);",
      "  overflow: hidden;",
      "  transition: border-color 0.2s ease;",
      "}",

      ".thinking-card:hover {",
      "  border-color: " + cTextMuted + ";",
      "}",

      ".thinking-header {",
      "  display: flex;",
      "  align-items: center;",
      "  gap: 8px;",
      "  padding: 10px 14px;",
      "  user-select: none;",
      "}",

      ".thinking-header__icon {",
      "  display: flex;",
      "  color: " + cTextSec + ";",
      "  flex-shrink: 0;",
      "}",

      ".thinking-header__label {",
      "  font-size: 13px;",
      "  font-weight: 600;",
      "  color: " + cTextSec + ";",
      "}",

      ".thinking-header__index {",
      "  font-size: 11px;",
      "  font-family: 'SF Mono', 'Fira Code', monospace;",
      "  color: " + cTextMuted + ";",
      "}",

      ".thinking-header__wordcount {",
      "  font-size: 11px;",
      "  color: " + cTextMuted + ";",
      "  margin-left: auto;",
      "}",

      ".thinking-header__duration {",
      "  font-size: 11px;",
      "  color: " + cTextMuted + ";",
      "  padding: 1px 6px;",
      "  border-radius: 4px;",
      "  background: rgba(255, 255, 255, 0.04);",
      "}",

      ".thinking-header__chevron {",
      "  display: flex;",
      "  color: " + cTextMuted + ";",
      "  transition: transform 0.2s ease;",
      "  flex-shrink: 0;",
      "}",

      ".thinking-header__chevron--open {",
      "  transform: rotate(90deg);",
      "}",

      // ── Thinking summary ──
      ".thinking-summary {",
      "  padding: 0 14px 8px;",
      "  font-size: 13px;",
      "  font-weight: 600;",
      "  color: " + cTextCode + ";",
      "  line-height: 1.4;",
      "}",

      // ── Thinking body ──
      ".thinking-body {",
      "  padding: 0 14px 14px;",
      "  overflow: hidden;",
      "  transition: max-height 0.3s ease, opacity 0.2s ease;",
      "}",

      ".thinking-card--collapsed .thinking-body {",
      "  padding-bottom: 0;",
      "}",

      ".thinking-body__content {",
      "  font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;",
      "  font-size: 12px;",
      "  line-height: 1.6;",
      "  color: " + cTextSec + ";",
      "}",

      ".thinking-paragraph {",
      "  margin: 0 0 8px;",
      "}",

      ".thinking-paragraph:last-child {",
      "  margin-bottom: 0;",
      "}",

      // ── Thinking lists (structured) ──
      ".thinking-list {",
      "  margin: 6px 0;",
      "  padding-left: 24px;",
      "}",

      ".thinking-list--structured {",
      "  border-left: 2px solid rgba(139, 148, 158, 0.15);",
      "  padding-left: 16px;",
      "  margin-left: 4px;",
      "}",

      ".thinking-list li {",
      "  margin: 4px 0;",
      "  line-height: 1.5;",
      "}",

      // ── Keyword highlighting ──
      ".thinking-keyword--reason {",
      "  color: " + cBlue + ";",
      "  font-weight: 600;",
      "}",

      ".thinking-keyword--conclusion {",
      "  color: " + cGreen + ";",
      "  font-weight: 600;",
      "}",

      ".thinking-keyword--contrast {",
      "  color: " + cYellow + ";",
      "  font-weight: 600;",
      "}",

      // ── Inline code ──
      ".thinking-inline-code {",
      "  font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;",
      "  font-size: 0.9em;",
      "  padding: 1px 5px;",
      "  border-radius: 4px;",
      "  background: rgba(255, 255, 255, 0.08);",
      "  color: " + cTextCode + ";",
      "}",

      // ── Code block ──
      ".code-block {",
      "  position: relative;",
      "  background: " + cBg + ";",
      "  border: 1px solid " + cBorderSubtle + ";",
      "  border-radius: 6px;",
      "  margin: 8px 0;",
      "  overflow-x: auto;",
      "}",

      ".code-block pre {",
      "  margin: 0;",
      "  padding: 12px 14px;",
      "  font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;",
      "  font-size: 12px;",
      "  line-height: 1.55;",
      "  white-space: pre;",
      "  color: " + cText + ";",
      "}",

      ".code-block code {",
      "  font-family: inherit;",
      "}",

      ".code-block__lang {",
      "  position: absolute;",
      "  top: 4px;",
      "  right: 8px;",
      "  font-size: 10px;",
      "  font-weight: 600;",
      "  text-transform: uppercase;",
      "  letter-spacing: 0.06em;",
      "  color: " + cTextMuted + ";",
      "  pointer-events: none;",
      "}",

      // ── JSON highlighting in code blocks ──
      ".json-key { color: " + cBlueBright + "; }",
      ".json-string { color: " + cBlueLight + "; }",
      ".json-number { color: " + cBlueBright + "; }",
      ".json-bool { color: " + cRedLight + "; }",
      ".json-null { color: " + cTextSec + "; }",

      // ── Tool call card ──
      ".tool-card {",
      "  border: 1px solid " + cBorder + ";",
      "  border-left: 3px solid " + cGreen + ";",
      "  border-radius: 8px;",
      "  background: rgba(63, 185, 80, 0.04);",
      "  overflow: hidden;",
      "}",

      ".tool-card__header {",
      "  display: flex;",
      "  align-items: center;",
      "  gap: 8px;",
      "  padding: 10px 14px;",
      "  flex-wrap: wrap;",
      "}",

      ".tool-card__icon {",
      "  display: flex;",
      "  color: " + cGreen + ";",
      "  flex-shrink: 0;",
      "}",

      ".tool-card__name {",
      "  font-size: 13px;",
      "  font-weight: 700;",
      "  font-family: 'SF Mono', 'Fira Code', monospace;",
      "  color: " + cText + ";",
      "}",

      ".tool-card__id {",
      "  font-size: 10px;",
      "  font-family: 'SF Mono', 'Fira Code', monospace;",
      "  color: " + cTextMuted + ";",
      "  padding: 1px 6px;",
      "  border-radius: 4px;",
      "  background: rgba(255, 255, 255, 0.04);",
      "}",

      ".tool-card__status {",
      "  font-size: 11px;",
      "  font-weight: 600;",
      "  padding: 2px 8px;",
      "  border-radius: 12px;",
      "  text-transform: uppercase;",
      "  letter-spacing: 0.04em;",
      "}",

      ".tool-card__status--success {",
      "  background: rgba(63, 185, 80, 0.15);",
      "  color: " + cGreen + ";",
      "}",

      ".tool-card__status--error {",
      "  background: rgba(248, 81, 73, 0.15);",
      "  color: " + cRed + ";",
      "}",

      ".tool-card__status--pending {",
      "  background: rgba(210, 153, 34, 0.15);",
      "  color: " + cYellow + ";",
      "}",

      ".tool-card__duration {",
      "  font-size: 11px;",
      "  color: " + cTextMuted + ";",
      "  margin-left: auto;",
      "}",

      ".tool-card__section {",
      "  padding: 0 14px 12px;",
      "}",

      ".tool-card__section-label {",
      "  font-size: 10px;",
      "  font-weight: 700;",
      "  text-transform: uppercase;",
      "  letter-spacing: 0.08em;",
      "  color: " + cTextSec + ";",
      "  margin-bottom: 6px;",
      "}",

      ".tool-card__result {",
      "  margin: 0;",
      "  padding: 10px 12px;",
      "  background: " + cBg + ";",
      "  border: 1px solid " + cBorderSubtle + ";",
      "  border-radius: 6px;",
      "  font-family: 'SF Mono', 'Fira Code', monospace;",
      "  font-size: 12px;",
      "  line-height: 1.5;",
      "  white-space: pre-wrap;",
      "  word-break: break-word;",
      "  color: " + cTextCode + ";",
      "  overflow: hidden;",
      "}",

      ".tool-card__result--error {",
      "  border-color: rgba(248, 81, 73, 0.3);",
      "  color: " + cRed + ";",
      "}",

      ".tool-card__result-wrap {",
      "  max-height: 120px;",
      "  overflow: hidden;",
      "  position: relative;",
      "  transition: max-height 0.3s ease;",
      "}",

      ".tool-card__result-wrap::after {",
      "  content: '';",
      "  position: absolute;",
      "  bottom: 0;",
      "  left: 0;",
      "  right: 0;",
      "  height: 40px;",
      "  background: linear-gradient(transparent, rgba(13, 17, 23, 0.95));",
      "  pointer-events: none;",
      "}",

      ".tool-card__result-wrap--open {",
      "  max-height: none;",
      "}",

      ".tool-card__result-wrap--open::after {",
      "  display: none;",
      "}",

      ".tool-card__toggle {",
      "  display: inline-block;",
      "  background: none;",
      "  border: none;",
      "  color: " + cBlue + ";",
      "  font-size: 12px;",
      "  cursor: pointer;",
      "  padding: 4px 0;",
      "  margin-bottom: 6px;",
      "}",

      ".tool-card__toggle:hover {",
      "  text-decoration: underline;",
      "}",

      // ── Output card ──
      ".output-card {",
      "  border: 1px solid " + cBorder + ";",
      "  border-left: 3px solid " + cBlue + ";",
      "  border-radius: 8px;",
      "  background: rgba(88, 166, 255, 0.04);",
      "  overflow: hidden;",
      "}",

      ".output-card__header {",
      "  display: flex;",
      "  align-items: center;",
      "  gap: 8px;",
      "  padding: 10px 14px;",
      "  flex-wrap: wrap;",
      "}",

      ".output-card__icon {",
      "  display: flex;",
      "  color: " + cBlue + ";",
      "  flex-shrink: 0;",
      "}",

      ".output-card__label {",
      "  font-size: 13px;",
      "  font-weight: 600;",
      "  color: " + cBlue + ";",
      "}",

      ".output-card__model {",
      "  font-size: 11px;",
      "  font-weight: 600;",
      "  padding: 2px 8px;",
      "  border-radius: 4px;",
      "  background: rgba(88, 166, 255, 0.12);",
      "  color: " + cBlue + ";",
      "  font-family: 'SF Mono', 'Fira Code', monospace;",
      "}",

      ".output-card__reason {",
      "  font-size: 11px;",
      "  padding: 2px 8px;",
      "  border-radius: 12px;",
      "  background: rgba(139, 148, 158, 0.12);",
      "  color: " + cTextSec + ";",
      "}",

      ".output-card__content {",
      "  padding: 0 14px 14px;",
      "  font-size: 13px;",
      "  line-height: 1.6;",
      "  color: " + cText + ";",
      "}",

      ".output-card__stats {",
      "  display: flex;",
      "  gap: 16px;",
      "  padding: 8px 14px;",
      "  border-top: 1px solid " + cBorderSubtle + ";",
      "  background: rgba(0, 0, 0, 0.15);",
      "  flex-wrap: wrap;",
      "}",

      ".output-card__stat {",
      "  display: inline-flex;",
      "  align-items: center;",
      "  gap: 4px;",
      "  font-size: 12px;",
      "}",

      ".output-card__stat-label {",
      "  color: " + cTextSec + ";",
      "}",

      ".output-card__stat-value {",
      "  color: " + cText + ";",
      "  font-weight: 600;",
      "  font-family: 'SF Mono', 'Fira Code', monospace;",
      "}",

      // ── Thinking timeline ──
      ".thinking-timeline-container {",
      "  margin-bottom: 4px;",
      "}",

      ".thinking-timeline {",
      "  display: flex;",
      "  height: 8px;",
      "  border-radius: 4px;",
      "  overflow: hidden;",
      "  background: " + cBorderSubtle + ";",
      "  gap: 1px;",
      "}",

      ".thinking-timeline__segment {",
      "  height: 100%;",
      "  min-width: 4px;",
      "  border-radius: 2px;",
      "  transition: opacity 0.15s ease;",
      "}",

      ".thinking-timeline__segment:hover {",
      "  opacity: 0.8;",
      "}",

      ".thinking-timeline__segment--thought {",
      "  background: " + cTextSec + ";",
      "}",

      ".thinking-timeline__segment--tool_call {",
      "  background: " + cGreen + ";",
      "}",

      ".thinking-timeline__segment--llm_done {",
      "  background: " + cBlue + ";",
      "}",

      // ── Flash animation for timeline click ──
      "@keyframes thinking-flash-anim {",
      "  0%   { box-shadow: 0 0 0 0 rgba(88, 166, 255, 0.5); }",
      "  50%  { box-shadow: 0 0 0 4px rgba(88, 166, 255, 0.3); }",
      "  100% { box-shadow: 0 0 0 0 rgba(88, 166, 255, 0); }",
      "}",

      ".thinking-flash {",
      "  animation: thinking-flash-anim 0.6s ease 2;",
      "}",

      // ── Streaming mode ──
      ".thinking-card--streaming .thinking-header__label {",
      "  color: " + cTextCode + ";",
      "}",

      ".thinking-body--streaming {",
      "  padding: 0 14px 14px;",
      "  font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;",
      "  font-size: 12px;",
      "  line-height: 1.6;",
      "  color: " + cTextSec + ";",
      "  white-space: pre-wrap;",
      "  word-break: break-word;",
      "}",

      ".thinking-stream-chunk {",
      "  animation: thinking-fade-in 0.15s ease;",
      "}",

      "@keyframes thinking-fade-in {",
      "  from { opacity: 0; }",
      "  to   { opacity: 1; }",
      "}",

      ".output-card__content--streaming {",
      "  padding: 0 14px 14px;",
      "  font-size: 13px;",
      "  line-height: 1.6;",
      "  color: " + cText + ";",
      "  white-space: pre-wrap;",
      "  word-break: break-word;",
      "}",

      // ── Pulse indicator ──
      ".thinking-header__pulse {",
      "  display: inline-block;",
      "  width: 6px;",
      "  height: 6px;",
      "  border-radius: 50%;",
      "  background: " + cTextSec + ";",
      "  animation: thinking-pulse-anim 1.5s ease infinite;",
      "}",

      "@keyframes thinking-pulse-anim {",
      "  0%, 100% { opacity: 0.3; }",
      "  50%      { opacity: 1; }",
      "}",

      // ── Markdown rendering inside output ──
      ".thinking-markdown {",
      "  font-size: inherit;",
      "  line-height: inherit;",
      "  color: inherit;",
      "}",

      ".thinking-markdown strong {",
      "  font-weight: 700;",
      "  color: " + cText + ";",
      "}",

      ".thinking-markdown div {",
      "  margin-bottom: 6px;",
      "}",

      ".thinking-markdown div:last-child {",
      "  margin-bottom: 0;",
      "}",
    ].join("\n");

    var style = document.createElement("style");
    style.id = "thinking-viewer-styles";
    style.textContent = css;
    document.head.appendChild(style);
  }

  // ---------------------------------------------------------------------------
  // Public API
  // ---------------------------------------------------------------------------

  injectStyles();

  window.ThinkingViewer = {
    renderThinkingView: renderThinkingView,
    renderThinkingCard: renderThinkingCard,
    renderToolCallCard: renderToolCallCard,
    renderOutputCard: renderOutputCard,
    renderThinkingTimeline: renderThinkingTimeline,
    streamThinking: streamThinking,
  };
})();
