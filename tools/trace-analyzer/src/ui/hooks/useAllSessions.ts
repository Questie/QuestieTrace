import { useEffect, useState } from "react";
import type { SessionRecord, SessionSummary } from "../../core/types.js";

/**
 * Fetches every full SessionRecord (not just summaries) for a trace file, so
 * extraction can replay all of them. Used by ExtractView, which - per project
 * decision - aggregates facts across ALL sessions in the selected file,
 * independent of the single session picked in the session dropdown.
 */
export function useAllSessions(fileName: string | null) {
  const [sessions, setSessions] = useState<SessionRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!fileName) {
      setSessions([]);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);

    fetch(`/api/file/${encodeURIComponent(fileName)}/sessions`)
      .then((r) => r.json())
      .then((summaries: SessionSummary[] & { error?: string }) => {
        if (summaries.error) throw new Error(summaries.error);
        return Promise.all(
          summaries.map((s) =>
            fetch(`/api/file/${encodeURIComponent(fileName)}/session/${s.index}`).then((r) =>
              r.json(),
            ),
          ),
        );
      })
      .then((full: SessionRecord[]) => {
        if (!cancelled) setSessions(full);
      })
      .catch((e) => {
        if (!cancelled) setError(e.message);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [fileName]);

  return { sessions, loading, error };
}
