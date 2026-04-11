import { useCallback, useEffect, useRef, useState, useMemo } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchChatModels, streamChat } from "@/api/chat";
import { executeWorkflow, compileWorkflow, explainWorkflow } from "@/api/compile";
import { fetchGraph } from "@/api/graph";
import { formatDuration, sessionPath } from "@/lib/format";
import { Overlay, CtxTab } from "@/lib/constants";
import type { ChatMessage, ModelInfo } from "@/api/chat";
import type { WorkflowInfo } from "@/types/api";

type RichCard = {
  type: "compile" | "execute" | "explain" | "error";
  data: Record<string, unknown>;
};

type EnrichedMessage = ChatMessage & {
  card?: RichCard;
};

export function ChatView() {
  const selectedWorkflow = useAppStore((s) => s.selectedWorkflow);
  const workflows = useAppStore((s) => s.workflows);
  const sessions = useAppStore((s) => s.sessions);
  const ops = useAppStore((s) => s.ops);
  const passes = useAppStore((s) => s.passes);
  const health = useAppStore((s) => s.health);
  const setOverlayView = useAppStore((s) => s.setOverlayView);
  const selectWorkflowAndShow = useAppStore((s) => s.selectWorkflowAndShow);
  const setGraphData = useAppStore((s) => s.setGraphData);
  const navigateToSession = useAppStore((s) => s.navigateToSession);
  const startInlineRun = useAppStore((s) => s.startInlineRun);
  const setContextPanelTab = useAppStore((s) => s.setContextPanelTab);

  const [messages, setMessages] = useState<EnrichedMessage[]>([]);
  const [input, setInput] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [selectedModel, setSelectedModel] = useState("claude-sonnet-4-5@20250929");
  const [error, setError] = useState<string | null>(null);
  const [showCommands, setShowCommands] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    fetchChatModels()
      .then((m) => {
        setModels(m);
        if (m.length > 0 && !m.find((x) => x.id === selectedModel)) {
          setSelectedModel(m[0].id);
        }
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  useEffect(() => {
    if (!streaming) inputRef.current?.focus();
  }, [streaming]);

  // Command palette
  const COMMANDS = useMemo(() => [
    { cmd: "/run", label: "Execute workflow", available: !!selectedWorkflow },
    { cmd: "/compile", label: "Compile workflow", available: !!selectedWorkflow },
    { cmd: "/explain", label: "Explain workflow", available: !!selectedWorkflow },
    { cmd: "/sessions", label: "Show sessions", available: true },
    { cmd: "/studio", label: "Open Studio", available: true },
    { cmd: "/clear", label: "Clear chat", available: messages.length > 0 },
  ], [selectedWorkflow, messages.length]);

  const filteredCommands = useMemo(() => {
    if (!showCommands) return [];
    const q = input.slice(1).toLowerCase();
    return COMMANDS.filter((c) => c.available && c.cmd.toLowerCase().includes("/" + q));
  }, [showCommands, input, COMMANDS]);

  async function handleCommand(cmd: string) {
    setShowCommands(false);
    setInput("");

    if (cmd === "/clear") {
      setMessages([]);
      setError(null);
      return;
    }

    if (cmd === "/studio") {
      setOverlayView(Overlay.STUDIO);
      return;
    }

    if (cmd === "/sessions") {
      setContextPanelTab(CtxTab.SESSIONS);
      return;
    }

    if (cmd === "/run" && selectedWorkflow) {
      const sysMsg: EnrichedMessage = {
        role: "assistant",
        content: `Executing **${selectedWorkflow.name}**...`,
      };
      setMessages((prev) => [...prev, sysMsg]);

      try {
        const result = await executeWorkflow(selectedWorkflow.path);
        if (result.session_path) {
          startInlineRun(result.session_path);
          const cardMsg: EnrichedMessage = {
            role: "assistant",
            content: "",
            card: {
              type: "execute",
              data: {
                workflow: selectedWorkflow.name,
                session_path: result.session_path,
                execution_id: result.execution_id,
              },
            },
          };
          setMessages((prev) => [...prev, cardMsg]);
        }
      } catch (e) {
        const errMsg: EnrichedMessage = {
          role: "assistant",
          content: "",
          card: { type: "error", data: { message: e instanceof Error ? e.message : String(e) } },
        };
        setMessages((prev) => [...prev, errMsg]);
      }
      return;
    }

    if (cmd === "/compile" && selectedWorkflow) {
      const sysMsg: EnrichedMessage = {
        role: "assistant",
        content: `Compiling **${selectedWorkflow.name}**...`,
      };
      setMessages((prev) => [...prev, sysMsg]);

      try {
        const result = await compileWorkflow(selectedWorkflow.path);
        const cardMsg: EnrichedMessage = {
          role: "assistant",
          content: "",
          card: {
            type: "compile",
            data: {
              success: result.success,
              artifact_path: result.artifact_path,
              duration_ms: result.duration_ms,
              passes: result.passes,
              node_count_before: result.node_count_before,
              node_count_after: result.node_count_after,
              stderr: result.stderr,
            },
          },
        };
        setMessages((prev) => [...prev, cardMsg]);
      } catch (e) {
        const errMsg: EnrichedMessage = {
          role: "assistant",
          content: "",
          card: { type: "error", data: { message: e instanceof Error ? e.message : String(e) } },
        };
        setMessages((prev) => [...prev, errMsg]);
      }
      return;
    }

    if (cmd === "/explain" && selectedWorkflow) {
      const sysMsg: EnrichedMessage = {
        role: "assistant",
        content: `Explaining **${selectedWorkflow.name}**...`,
      };
      setMessages((prev) => [...prev, sysMsg]);

      try {
        const result = await explainWorkflow(selectedWorkflow.path);
        const cardMsg: EnrichedMessage = {
          role: "assistant",
          content: "",
          card: {
            type: "explain",
            data: {
              success: result.success,
              explanation: result.explanation,
            },
          },
        };
        setMessages((prev) => [...prev, cardMsg]);
      } catch (e) {
        const errMsg: EnrichedMessage = {
          role: "assistant",
          content: "",
          card: { type: "error", data: { message: e instanceof Error ? e.message : String(e) } },
        };
        setMessages((prev) => [...prev, errMsg]);
      }
      return;
    }
  }

  const handleSend = useCallback(async () => {
    const text = input.trim();
    if (!text || streaming) return;

    // Handle commands
    if (text.startsWith("/")) {
      const cmd = text.split(" ")[0].toLowerCase();
      const match = COMMANDS.find((c) => c.cmd === cmd && c.available);
      if (match) {
        await handleCommand(match.cmd);
        return;
      }
    }

    setError(null);
    setShowCommands(false);
    const userMsg: EnrichedMessage = { role: "user", content: text };
    const newMessages = [...messages, userMsg];
    setMessages(newMessages);
    setInput("");
    setStreaming(true);

    const assistantMsg: EnrichedMessage = { role: "assistant", content: "" };
    setMessages([...newMessages, assistantMsg]);

    const controller = new AbortController();
    abortRef.current = controller;

    try {
      let accumulated = "";
      await streamChat(
        newMessages,
        selectedModel,
        (token) => {
          accumulated += token;
          setMessages((prev) => {
            const updated = [...prev];
            updated[updated.length - 1] = { role: "assistant", content: accumulated };
            return updated;
          });
        },
        () => {},
        (err) => setError(err),
        controller.signal,
      );
    } catch (e: unknown) {
      if ((e as Error).name !== "AbortError") {
        setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      setStreaming(false);
      abortRef.current = null;
    }
  }, [input, messages, selectedModel, streaming, COMMANDS]);

  const handleStop = useCallback(() => {
    abortRef.current?.abort();
    setStreaming(false);
  }, []);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  const handleInputChange = useCallback((e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const val = e.target.value;
    setInput(val);
    setShowCommands(val.startsWith("/") && val.length > 0 && !val.includes(" "));
  }, []);

  const modelLabel = (m: ModelInfo) => {
    const alias = m.aliases[0];
    return alias ? `${alias} (${m.id})` : m.id;
  };

  const totalNodes = workflows.reduce((sum, w) => sum + (w.node_count ?? 0), 0);
  const runningSessions = sessions.filter((s) => s.status === "running");

  return (
    <div className="chat-view">
      {/* Workflow context header */}
      {selectedWorkflow && (
        <div className="chat-ctx-bar">
          <div className="chat-ctx-bar__info">
            <span className="chat-ctx-bar__label">Workflow:</span>
            <span className="chat-ctx-bar__name">{selectedWorkflow.name}</span>
            {selectedWorkflow.node_count != null && (
              <span className="chat-ctx-bar__meta">{selectedWorkflow.node_count} nodes</span>
            )}
            <span className="chat-ctx-bar__type">{selectedWorkflow.source_type}</span>
          </div>
          <div className="chat-ctx-bar__actions">
            <button
              type="button"
              className="chat-ctx-btn"
              onClick={() => handleCommand("/compile")}
              disabled={streaming}
            >
              Compile
            </button>
            <button
              type="button"
              className="chat-ctx-btn chat-ctx-btn--primary"
              onClick={() => handleCommand("/run")}
              disabled={streaming}
            >
              Execute
            </button>
          </div>
        </div>
      )}

      {/* Messages */}
      <div className="chat-view__messages">
        {messages.length === 0 && <ChatEmptyState
          workflows={workflows}
          sessions={sessions}
          ops={ops}
          passes={passes}
          health={health}
          totalNodes={totalNodes}
          runningSessions={runningSessions}
          onSelectWorkflow={(w) => {
            selectWorkflowAndShow(w);
            fetchGraph(w.path).then((g) => setGraphData(g, w.path)).catch(() => {});
          }}
          onOpenStudio={() => setOverlayView(Overlay.STUDIO)}
          onNavigateToSession={navigateToSession}
        />}
        {messages.map((msg, i) => (
          <MessageBubble
            key={i}
            message={msg}
            isStreaming={streaming && i === messages.length - 1 && msg.role === "assistant"}
          />
        ))}
        {error && (
          <div className="chat-view__error">
            <strong>Error:</strong> {error}
          </div>
        )}
        <div ref={messagesEndRef} />
      </div>

      {/* Command palette popup */}
      {showCommands && filteredCommands.length > 0 && (
        <div className="chat-cmd-palette">
          {filteredCommands.map((c) => (
            <button
              key={c.cmd}
              type="button"
              className="chat-cmd-palette__item"
              onClick={() => {
                setInput("");
                handleCommand(c.cmd);
              }}
            >
              <span className="chat-cmd-palette__cmd">{c.cmd}</span>
              <span className="chat-cmd-palette__label">{c.label}</span>
            </button>
          ))}
        </div>
      )}

      {/* Input area */}
      <div className="chat-view__input-area">
        <div className="chat-view__input-row">
          <select
            className="chat-view__model-select"
            value={selectedModel}
            onChange={(e) => setSelectedModel(e.target.value)}
            disabled={streaming}
          >
            {models.map((m) => (
              <option key={m.id} value={m.id}>{modelLabel(m)}</option>
            ))}
            {models.length === 0 && (
              <option value={selectedModel}>{selectedModel}</option>
            )}
          </select>
          <textarea
            ref={inputRef}
            className="chat-view__input"
            placeholder={selectedWorkflow ? `Ask about ${selectedWorkflow.name}, or type / for commands...` : "Type a message or / for commands..."}
            value={input}
            onChange={handleInputChange}
            onKeyDown={handleKeyDown}
            disabled={streaming}
            rows={1}
            onInput={(e) => {
              const target = e.target as HTMLTextAreaElement;
              target.style.height = "auto";
              target.style.height = Math.min(target.scrollHeight, 160) + "px";
            }}
          />
          {streaming ? (
            <button type="button" className="chat-view__send-btn chat-view__send-btn--stop" onClick={handleStop}>
              <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
                <rect x="3" y="3" width="10" height="10" rx="1" />
              </svg>
            </button>
          ) : (
            <button
              type="button"
              className="chat-view__send-btn"
              onClick={handleSend}
              disabled={!input.trim()}
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M22 2L11 13M22 2l-7 20-4-9-9-4z" />
              </svg>
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

// ─── Empty State (replaces Dashboard) ──────────────────────────────────────

function ChatEmptyState({
  workflows,
  sessions,
  ops,
  passes,
  health,
  totalNodes,
  runningSessions,
  onSelectWorkflow,
  onOpenStudio,
  onNavigateToSession,
}: {
  workflows: WorkflowInfo[];
  sessions: any[];
  ops: any[];
  passes: any[];
  health: any;
  totalNodes: number;
  runningSessions: any[];
  onSelectWorkflow: (w: WorkflowInfo) => void;
  onOpenStudio: () => void;
  onNavigateToSession: (path: string) => void;
}) {
  return (
    <div className="chat-empty">
      <div className="chat-empty__hero">
        <h2 className="chat-empty__title">APXM Agent Studio</h2>
        <p className="chat-empty__subtitle">
          Design, compile, execute &amp; observe agent workflows
        </p>
      </div>

      {/* Compact stats row */}
      <div className="chat-empty__stats">
        <StatPill value={workflows.length} label="workflows" />
        <StatPill value={totalNodes} label="nodes" />
        <StatPill value={ops.length} label="ops" />
        <StatPill value={passes.length} label="passes" />
        <StatPill value={sessions.length} label="sessions" />
        <StatPill value={health?.total_models ?? 0} label="models" />
      </div>

      {/* Live sessions alert */}
      {runningSessions.length > 0 && (
        <div className="chat-empty__live">
          <span className="chat-empty__live-dot" />
          <span>{runningSessions.length} running session{runningSessions.length !== 1 ? "s" : ""}</span>
          <button
            type="button"
            className="ghost-button ghost-button--sm"
            onClick={() => onNavigateToSession(sessionPath(runningSessions[0].id))}
          >
            Watch
          </button>
        </div>
      )}

      {/* Getting started hints */}
      <div className="chat-empty__hints">
        <div className="chat-empty__hint" onClick={onOpenStudio}>
          <span className="chat-empty__hint-icon">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
              <path d="M7 21l3-3m0 0l3 3m-3-3v8M4 4h16M4 4v16h16V4M9 9h6M9 9v6m6-6v6M9 15h6" />
            </svg>
          </span>
          <div>
            <strong>Open Studio</strong>
            <p>Design and edit workflow graphs</p>
          </div>
        </div>
        <div className="chat-empty__hint">
          <span className="chat-empty__hint-icon">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
              <path d="M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2z" />
            </svg>
          </span>
          <div>
            <strong>Chat with any model</strong>
            <p>Ask about workflows, debug agents, or get help</p>
          </div>
        </div>
        <div className="chat-empty__hint">
          <span className="chat-empty__hint-icon">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
              <path d="M13 10V3L4 14h7v7l9-11h-7z" />
            </svg>
          </span>
          <div>
            <strong>Use commands</strong>
            <p>Type <code>/run</code>, <code>/compile</code>, or <code>/explain</code></p>
          </div>
        </div>
      </div>

      {/* Quick workflow picks */}
      {workflows.length > 0 && (
        <div className="chat-empty__workflows">
          <span className="chat-empty__workflows-label">Quick start &mdash; select a workflow:</span>
          <div className="chat-empty__workflow-list">
            {workflows.slice(0, 6).map((w) => (
              <button
                key={w.path}
                type="button"
                className="chat-empty__workflow-btn"
                onClick={() => onSelectWorkflow(w)}
              >
                <span className="chat-empty__workflow-name">{w.name}</span>
                {w.node_count != null && (
                  <span className="chat-empty__workflow-nodes">{w.node_count}n</span>
                )}
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function StatPill({ value, label }: { value: number; label: string }) {
  return (
    <div className="stat-pill">
      <span className="stat-pill__value">{value}</span>
      <span className="stat-pill__label">{label}</span>
    </div>
  );
}

// ─── Message Bubble ────────────────────────────────────────────────────────

function MessageBubble({ message, isStreaming }: { message: EnrichedMessage; isStreaming: boolean }) {
  const isUser = message.role === "user";

  return (
    <div className={`chat-msg ${isUser ? "chat-msg--user" : "chat-msg--assistant"}`}>
      <div className="chat-msg__avatar">
        {isUser ? (
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
            <path d="M20 21v-2a4 4 0 00-4-4H8a4 4 0 00-4 4v2M12 11a4 4 0 100-8 4 4 0 000 8z" />
          </svg>
        ) : (
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
            <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" />
          </svg>
        )}
      </div>
      <div className="chat-msg__body">
        <div className="chat-msg__role">{isUser ? "You" : "APXM"}</div>
        <div className="chat-msg__content">
          {message.content && <MessageContent text={message.content} />}
          {message.card && <RichCardRenderer card={message.card} />}
          {isStreaming && <span className="chat-msg__cursor" />}
        </div>
      </div>
    </div>
  );
}

// ─── Rich Card Renderer ────────────────────────────────────────────────────

function RichCardRenderer({ card }: { card: RichCard }) {
  if (card.type === "execute") {
    return (
      <div className="rich-card rich-card--execute">
        <div className="rich-card__header">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M5 3l14 9-14 9V3z" />
          </svg>
          <span>Execution Started</span>
        </div>
        <div className="rich-card__body">
          <div className="rich-card__field">
            <span className="rich-card__field-label">Workflow</span>
            <span>{card.data.workflow as string}</span>
          </div>
          <div className="rich-card__field">
            <span className="rich-card__field-label">Session</span>
            <code className="rich-card__field-code">
              {(card.data.session_path as string)?.split("/").pop()}
            </code>
          </div>
        </div>
      </div>
    );
  }

  if (card.type === "compile") {
    const success = card.data.success as boolean;
    return (
      <div className={`rich-card ${success ? "rich-card--success" : "rich-card--error"}`}>
        <div className="rich-card__header">
          <span>{success ? "Compilation Successful" : "Compilation Failed"}</span>
        </div>
        <div className="rich-card__body">
          {card.data.duration_ms != null && (
            <div className="rich-card__field">
              <span className="rich-card__field-label">Duration</span>
              <span>{formatDuration(card.data.duration_ms as number)}</span>
            </div>
          )}
          {typeof card.data.artifact_path === "string" && (
            <div className="rich-card__field">
              <span className="rich-card__field-label">Artifact</span>
              <code className="rich-card__field-code">{card.data.artifact_path}</code>
            </div>
          )}
          {card.data.node_count_before != null && card.data.node_count_after != null && (
            <div className="rich-card__field">
              <span className="rich-card__field-label">Nodes</span>
              <span>{String(card.data.node_count_before)} &rarr; {String(card.data.node_count_after)}</span>
            </div>
          )}
          {typeof card.data.stderr === "string" && card.data.stderr && (
            <pre className="rich-card__stderr">{card.data.stderr}</pre>
          )}
        </div>
      </div>
    );
  }

  if (card.type === "explain") {
    return (
      <div className="rich-card rich-card--explain">
        <div className="rich-card__header">
          <span>Workflow Explanation</span>
        </div>
        <div className="rich-card__body">
          <div className="rich-card__explanation">
            <MessageContent text={card.data.explanation as string} />
          </div>
        </div>
      </div>
    );
  }

  if (card.type === "error") {
    return (
      <div className="rich-card rich-card--error">
        <div className="rich-card__header">
          <span>Error</span>
        </div>
        <div className="rich-card__body">
          <pre className="rich-card__stderr">{card.data.message as string}</pre>
        </div>
      </div>
    );
  }

  return null;
}

// ─── Message Content ───────────────────────────────────────────────────────

function MessageContent({ text }: { text: string }) {
  if (!text) return null;

  const parts = text.split(/(```[\s\S]*?```)/g);
  return (
    <>
      {parts.map((part, i) => {
        if (part.startsWith("```") && part.endsWith("```")) {
          const inner = part.slice(3, -3);
          const newlineIdx = inner.indexOf("\n");
          const code = newlineIdx >= 0 ? inner.slice(newlineIdx + 1) : inner;
          return (
            <pre key={i} className="chat-msg__code">
              <code>{code}</code>
            </pre>
          );
        }
        const lines = part.split("\n");
        return lines.map((line, j) => (
          <span key={`${i}-${j}`}>
            {j > 0 && <br />}
            {formatInline(line)}
          </span>
        ));
      })}
    </>
  );
}

function formatInline(text: string) {
  const parts = text.split(/(\*\*[^*]+\*\*|`[^`]+`)/g);
  return parts.map((p, i) => {
    if (p.startsWith("`") && p.endsWith("`")) {
      return <code key={i} className="chat-msg__inline-code">{p.slice(1, -1)}</code>;
    }
    if (p.startsWith("**") && p.endsWith("**")) {
      return <strong key={i}>{p.slice(2, -2)}</strong>;
    }
    return <span key={i}>{p}</span>;
  });
}
