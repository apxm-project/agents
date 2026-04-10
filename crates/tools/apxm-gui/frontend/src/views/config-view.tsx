import { useEffect, useState, useMemo } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchConfig } from "@/api/config";
import { getErrorMessage } from "@/lib/format";

function highlightToml(text: string): React.ReactNode[] {
  return text.split("\n").map((line, i) => {
    let content: React.ReactNode;
    const trimmed = line.trimStart();

    if (trimmed.startsWith("#")) {
      content = <span key={i} className="toml-comment">{line}</span>;
    } else if (/^\[.*\]/.test(trimmed)) {
      content = <span key={i} className="toml-section">{line}</span>;
    } else {
      const eqIdx = line.indexOf("=");
      if (eqIdx !== -1) {
        const key = line.slice(0, eqIdx);
        const rest = line.slice(eqIdx);
        content = (
          <span key={i}>
            <span className="toml-key">{key}</span>
            <span className="toml-eq">{rest.charAt(0)}</span>
            <span className="toml-value">{rest.slice(1)}</span>
          </span>
        );
      } else {
        content = <span key={i}>{line}</span>;
      }
    }
    return <span key={i}>{content}{"\n"}</span>;
  });
}

export function ConfigView() {
  const config = useAppStore((s) => s.config);
  const setConfig = useAppStore((s) => s.setConfig);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetchConfig().then(setConfig).catch((e) => setError(getErrorMessage(e)));
  }, [setConfig]);

  const highlighted = useMemo(() => (config ? highlightToml(config) : null), [config]);

  return (
    <div className="view-panel">
      <div className="view-panel__header">
        <h2>Configuration</h2>
        <span className="config-path">~/.apxm/config.toml</span>
      </div>
      {error ? <div className="error-banner">{error}</div> : null}
      {highlighted ? (
        <pre className="code-block config-block">{highlighted}</pre>
      ) : (
        <div className="view-panel__empty">No configuration found at ~/.apxm/config.toml</div>
      )}
    </div>
  );
}
