import { useEffect, useState, useMemo, useCallback } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchConfig } from "@/api/config";
import { fetchBackends } from "@/api/backends";
import { fetchHealth } from "@/api/health";
import { updateConfig } from "@/api/compile";
import { getErrorMessage, STATUS_COLORS, FALLBACK_STATUS_COLOR, statusBadgeStyle } from "@/lib/format";
import type { BackendDetail, ModelDetail } from "@/types/api";

type Tab = "backends" | "connections" | "config";

function formatContextWindow(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`;
  return String(n);
}

function ModelCard({ model }: { model: ModelDetail }) {
  return (
    <div className="model-card">
      <div className="model-card__header">
        <span className="model-card__id">{model.id}</span>
        {model.context_window > 0 && (
          <span className="model-card__ctx">{formatContextWindow(model.context_window)} ctx</span>
        )}
      </div>
      {model.aliases.length > 0 && (
        <div className="model-card__aliases">
          {model.aliases.map((a) => (
            <span key={a} className="model-card__alias">{a}</span>
          ))}
        </div>
      )}
      <div className="model-card__caps">
        {model.supports_vision && <span className="model-card__cap model-card__cap--vision">vision</span>}
        {model.supports_functions && <span className="model-card__cap model-card__cap--functions">functions</span>}
      </div>
      {model.tags.length > 0 && (
        <div className="model-card__tags">
          {model.tags.map((t) => (
            <span key={t} className="model-card__tag">{t}</span>
          ))}
        </div>
      )}
    </div>
  );
}

function BackendCard({
  backend,
  expanded,
  onToggle,
}: {
  backend: BackendDetail;
  expanded: boolean;
  onToggle: () => void;
}) {
  const statusColor = STATUS_COLORS[backend.status] ?? FALLBACK_STATUS_COLOR;

  return (
    <div className={`backend-card${expanded ? " backend-card--expanded" : ""}`}>
      <button type="button" className="backend-card__header" onClick={onToggle}>
        <div className="backend-card__title-row">
          <span className="backend-card__dot" style={{ backgroundColor: statusColor }} />
          <span className="backend-card__name">{backend.name}</span>
          <span className="backend-card__type-badge">{backend.backend_type}</span>
          <span className="backend-card__status" style={statusBadgeStyle(statusColor)}>
            {backend.status}
          </span>
        </div>
        <div className="backend-card__meta">
          <span className="backend-card__protocol">{backend.protocol}</span>
          <span className="backend-card__endpoint">{backend.endpoint || "—"}</span>
          <span className="backend-card__model-count">{backend.model_count} models</span>
        </div>
        <span className="backend-card__chevron">{expanded ? "▾" : "▸"}</span>
      </button>
      {expanded && (
        <div className="backend-card__models">
          {backend.models.length === 0 ? (
            <div className="backend-card__no-models">No models configured</div>
          ) : (
            <div className="backend-card__model-grid">
              {backend.models.map((m) => (
                <ModelCard key={m.id} model={m} />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function highlightToml(text: string): React.ReactNode[] {
  return text.split("\n").map((line, i) => {
    let content: React.ReactNode;
    const trimmed = line.trimStart();
    if (trimmed.startsWith("#")) {
      content = <span className="toml-comment">{line}</span>;
    } else if (/^\[.*\]/.test(trimmed)) {
      content = <span className="toml-section">{line}</span>;
    } else {
      const eqIdx = line.indexOf("=");
      if (eqIdx !== -1) {
        const key = line.slice(0, eqIdx);
        const rest = line.slice(eqIdx);
        content = (
          <>
            <span className="toml-key">{key}</span>
            <span className="toml-eq">{rest.charAt(0)}</span>
            <span className="toml-value">{rest.slice(1)}</span>
          </>
        );
      } else {
        content = line;
      }
    }
    return <span key={i}>{content}{"\n"}</span>;
  });
}

export function ConfigView() {
  const config = useAppStore((s) => s.config);
  const setConfig = useAppStore((s) => s.setConfig);
  const health = useAppStore((s) => s.health);
  const setHealth = useAppStore((s) => s.setHealth);

  const [tab, setTab] = useState<Tab>("backends");
  const [backends, setBackends] = useState<BackendDetail[]>([]);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [probing, setProbing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [editContent, setEditContent] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    fetchBackends().then(setBackends).catch((e) => setError(getErrorMessage(e)));
    fetchConfig().then(setConfig).catch(() => {});
    if (!health) fetchHealth().then(setHealth).catch(() => {});
  }, [setConfig, health, setHealth]);

  const totalModels = useMemo(
    () => backends.reduce((n, b) => n + b.models.length, 0),
    [backends],
  );

  const highlighted = useMemo(() => (config ? highlightToml(config) : null), [config]);

  function toggleExpand(name: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  }

  function expandAll() {
    setExpanded(new Set(backends.map((b) => b.name)));
  }

  function collapseAll() {
    setExpanded(new Set());
  }

  async function probeAll() {
    setProbing(true);
    try {
      const probed = await fetchBackends({ probe: true });
      setBackends(probed);
    } catch (e) {
      setError(getErrorMessage(e));
    }
    setProbing(false);
  }

  const startEdit = useCallback(() => {
    setEditContent(config);
    setEditing(true);
  }, [config]);

  const saveConfig = useCallback(async () => {
    setSaving(true);
    try {
      await updateConfig(editContent);
      setConfig(editContent);
      setEditing(false);
      // Refresh backends
      fetchBackends().then(setBackends).catch(() => {});
    } catch (e) {
      setError(getErrorMessage(e));
    }
    setSaving(false);
  }, [editContent, setConfig]);

  return (
    <div className="view-panel">
      <div className="view-panel__header">
        <h2>Infrastructure</h2>
        <div className="infra-tab-bar">
          <button
            type="button"
            className={`infra-tab${tab === "backends" ? " infra-tab--active" : ""}`}
            onClick={() => setTab("backends")}
          >
            Backends & Models
          </button>
          <button
            type="button"
            className={`infra-tab${tab === "connections" ? " infra-tab--active" : ""}`}
            onClick={() => setTab("connections")}
          >
            Connections
          </button>
          <button
            type="button"
            className={`infra-tab${tab === "config" ? " infra-tab--active" : ""}`}
            onClick={() => setTab("config")}
          >
            Config
          </button>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}

      {tab === "backends" && (
        <div className="infra-backends">
          <div className="infra-summary">
            <div className="infra-summary__stats">
              <span className="infra-summary__stat">
                <strong>{backends.length}</strong> backends
              </span>
              <span className="infra-summary__stat">
                <strong>{totalModels}</strong> models
              </span>
              {health && (
                <>
                  <span className="infra-summary__stat">
                    <strong>{health.total_agents}</strong> agents
                  </span>
                  <span className="infra-summary__stat">
                    <strong>{health.total_tools}</strong> tools
                  </span>
                  <span className="infra-summary__stat infra-summary__stat--faint">
                    config: {health.config_source}
                  </span>
                </>
              )}
            </div>
            <div className="infra-summary__actions">
              <button type="button" className="ghost-button ghost-button--sm" onClick={expandAll}>
                Expand All
              </button>
              <button type="button" className="ghost-button ghost-button--sm" onClick={collapseAll}>
                Collapse
              </button>
              <button
                type="button"
                className={`ghost-button ghost-button--sm${probing ? " ghost-button--loading" : ""}`}
                onClick={probeAll}
                disabled={probing}
              >
                {probing ? "Probing..." : "Probe All"}
              </button>
            </div>
          </div>

          <div className="infra-backend-list">
            {backends.length === 0 ? (
              <div className="infra-empty">
                No backends configured. Add backends to <code>~/.apxm/config.toml</code>.
              </div>
            ) : (
              backends.map((b) => (
                <BackendCard
                  key={b.name}
                  backend={b}
                  expanded={expanded.has(b.name)}
                  onToggle={() => toggleExpand(b.name)}
                />
              ))
            )}
          </div>
        </div>
      )}

      {tab === "connections" && (
        <ConnectionDashboard backends={backends} onProbe={probeAll} probing={probing} />
      )}

      {tab === "config" && (
        <div className="infra-config">
          <div className="infra-config__header">
            <span className="infra-config__path">~/.apxm/config.toml</span>
            <div className="infra-config__actions">
              {editing ? (
                <>
                  <button
                    type="button"
                    className={`ghost-button ghost-button--sm${saving ? " ghost-button--loading" : ""}`}
                    onClick={saveConfig}
                    disabled={saving}
                  >
                    {saving ? "Saving..." : "Save"}
                  </button>
                  <button
                    type="button"
                    className="ghost-button ghost-button--sm"
                    onClick={() => setEditing(false)}
                  >
                    Cancel
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  className="ghost-button ghost-button--sm"
                  onClick={startEdit}
                  disabled={!config}
                >
                  Edit
                </button>
              )}
            </div>
          </div>
          {editing ? (
            <textarea
              className="infra-config__editor"
              value={editContent}
              onChange={(e) => setEditContent(e.target.value)}
              spellCheck={false}
            />
          ) : highlighted ? (
            <pre className="code-block config-block">{highlighted}</pre>
          ) : (
            <div className="view-panel__empty">No configuration found</div>
          )}
        </div>
      )}
    </div>
  );
}

// ─── Connection Dashboard ──────────────────────────────────────────────────

function ConnectionDashboard({
  backends,
  onProbe,
  probing,
}: {
  backends: BackendDetail[];
  onProbe: () => void;
  probing: boolean;
}) {
  const healthy = backends.filter((b) => b.status === "healthy").length;
  const unreachable = backends.filter((b) => b.status === "unreachable").length;
  const unknown = backends.filter((b) => b.status === "unknown").length;

  return (
    <div className="connection-dashboard">
      <div className="connection-dashboard__header">
        <div className="connection-dashboard__summary">
          <span className="connection-dashboard__stat connection-dashboard__stat--healthy">
            <span className="connection-dashboard__dot" style={{ background: STATUS_COLORS.healthy }} />
            {healthy} healthy
          </span>
          <span className="connection-dashboard__stat connection-dashboard__stat--unreachable">
            <span className="connection-dashboard__dot" style={{ background: STATUS_COLORS.unreachable ?? "#e74c3c" }} />
            {unreachable} unreachable
          </span>
          <span className="connection-dashboard__stat connection-dashboard__stat--unknown">
            <span className="connection-dashboard__dot" style={{ background: FALLBACK_STATUS_COLOR }} />
            {unknown} unknown
          </span>
        </div>
        <button
          type="button"
          className={`ghost-button ghost-button--sm${probing ? " ghost-button--loading" : ""}`}
          onClick={onProbe}
          disabled={probing}
        >
          {probing ? "Probing..." : "Probe All"}
        </button>
      </div>
      <div className="connection-dashboard__list">
        {backends.map((b) => {
          const statusColor = STATUS_COLORS[b.status] ?? FALLBACK_STATUS_COLOR;
          return (
            <div key={b.name} className="connection-dashboard__row">
              <span className="connection-dashboard__dot" style={{ background: statusColor }} />
              <span className="connection-dashboard__name">{b.name}</span>
              <span className="connection-dashboard__endpoint">{b.endpoint || "—"}</span>
              <span className="connection-dashboard__protocol">{b.protocol}</span>
              <span className="connection-dashboard__models">{b.model_count} models</span>
              <span className="connection-dashboard__badge" style={statusBadgeStyle(statusColor)}>
                {b.status}
              </span>
            </div>
          );
        })}
        {backends.length === 0 && (
          <div className="infra-empty">No backends configured.</div>
        )}
      </div>
    </div>
  );
}
