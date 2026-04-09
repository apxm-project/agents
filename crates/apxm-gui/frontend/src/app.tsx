import { useEffect } from "react";
import { AppShell } from "./layouts/app-shell";
import { useAppStore } from "@/store/app-store";
import { useLiveSession } from "@/hooks/use-live-session";
import { useKeyboard } from "@/hooks/use-keyboard";
import { fetchStartup, fetchExamples } from "@/api/startup";
import { fetchOps } from "@/api/ops";
import { fetchGraph } from "@/api/graph";
import { getErrorMessage } from "@/lib/format";

export function App() {
  const setGraphData = useAppStore((s) => s.setGraphData);
  const setOps = useAppStore((s) => s.setOps);
  const setExamples = useAppStore((s) => s.setExamples);
  const setLoading = useAppStore((s) => s.setLoading);
  const setError = useAppStore((s) => s.setError);

  useKeyboard();
  useLiveSession();

  useEffect(() => {
    let cancelled = false;

    async function bootstrap() {
      setLoading("startup", true);

      // Fetch ops metadata (non-critical)
      fetchOps()
        .then((ops) => { if (!cancelled) setOps(ops); })
        .catch(() => {});

      // Fetch examples (non-critical)
      fetchExamples()
        .then((examples) => { if (!cancelled) setExamples(examples); })
        .catch(() => {});

      // Check for initial file
      try {
        const startup = await fetchStartup();
        if (startup.initial_file && !cancelled) {
          const graph = await fetchGraph(startup.initial_file);
          if (!cancelled) setGraphData(graph, startup.initial_file);
        }
      } catch (e) {
        if (!cancelled) setError("startup", getErrorMessage(e));
      }

      if (!cancelled) setLoading("startup", false);
    }

    void bootstrap();
    return () => { cancelled = true; };
  }, [setGraphData, setOps, setExamples, setLoading, setError]);

  return <AppShell />;
}
