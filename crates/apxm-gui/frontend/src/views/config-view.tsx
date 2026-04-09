import { useEffect, useState } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchConfig } from "@/api/config";
import { getErrorMessage } from "@/lib/format";

export function ConfigView() {
  const config = useAppStore((s) => s.config);
  const setConfig = useAppStore((s) => s.setConfig);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetchConfig().then(setConfig).catch((e) => setError(getErrorMessage(e)));
  }, [setConfig]);

  return (
    <div className="view-panel">
      <div className="view-panel__header">
        <h2>Configuration</h2>
      </div>
      {error ? <div className="error-banner">{error}</div> : null}
      {config ? (
        <pre className="code-block config-block">{config}</pre>
      ) : (
        <div className="view-panel__empty">No configuration found at ~/.apxm/config.toml</div>
      )}
    </div>
  );
}
