import { useState } from "react";

interface ExtractResult {
  npcDB: string;
  sessionCount: number;
  fileCount: number;
  skippedFiles: { name: string; error: string }[];
}

type Status =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "done"; result: ExtractResult }
  | { kind: "error"; message: string };

/**
 * "Extract" tab content: on demand, runs the full observe -> aggregate ->
 * emit -> write chain over EVERY trace file in Traces/ (server-side, via
 * /api/extract/npc) and renders the resulting Questie-format Lua for
 * copy/download.
 *
 * Combining across all trace files - not just the one selected in the file
 * dropdown - is the whole point of this tool: more traces means more
 * confidence in the aggregated facts. Extraction happens server-side because
 * transferring full session JSON for dozens of multi-MB trace files to the
 * browser just to extract a handful of facts doesn't scale.
 *
 * Currently only the npc entity kind has observers wired up (see
 * src/extract/index.ts); quest/item/object will get their own
 * endpoints/output once their observers land.
 */
export function ExtractView() {
  const [status, setStatus] = useState<Status>({ kind: "idle" });

  const handleGenerate = () => {
    setStatus({ kind: "loading" });
    fetch("/api/extract/npc")
      .then((r) => r.json())
      .then((data: ExtractResult & { error?: string }) => {
        if (data.error) throw new Error(data.error);
        setStatus({ kind: "done", result: data });
      })
      .catch((e) => setStatus({ kind: "error", message: e.message }));
  };

  const handleCopy = (npcDB: string) => {
    navigator.clipboard.writeText(npcDB);
  };

  const handleDownload = (npcDB: string) => {
    const blob = new Blob([npcDB], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "npcDB.lua";
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div className="extract-view">
      <div className="extract-toolbar">
        <button onClick={handleGenerate} disabled={status.kind === "loading"}>
          {status.kind === "loading" ? "Generating..." : "Generate"}
        </button>
        {status.kind === "done" && (
          <>
            <button onClick={() => handleCopy(status.result.npcDB)}>Copy to clipboard</button>
            <button onClick={() => handleDownload(status.result.npcDB)}>Download npcDB.lua</button>
          </>
        )}
      </div>

      {status.kind === "error" && <div className="error-msg">Error: {status.message}</div>}

      {status.kind === "done" && (
        <>
          <div className="extract-summary">
            {status.result.sessionCount} session(s) from {status.result.fileCount} trace file(s)
          </div>
          {status.result.skippedFiles.length > 0 && (
            <div className="extract-warning">
              Skipped {status.result.skippedFiles.length} file(s) that failed to load:
              <ul>
                {status.result.skippedFiles.map((f) => (
                  <li key={f.name}>
                    {f.name}: {f.error}
                  </li>
                ))}
              </ul>
            </div>
          )}
          <pre className="extract-output">{status.result.npcDB}</pre>
        </>
      )}
    </div>
  );
}
