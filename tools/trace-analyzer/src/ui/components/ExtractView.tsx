import { useState } from "react";

interface ExtractResult {
  npcFixes: string;
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
 * /api/extract/npc) and renders the resulting ForeverTraceNpcFixes.lua
 * corrections module for copy/download.
 *
 * Combining across all trace files - not just the one selected in the file
 * dropdown - is the whole point of this tool: more traces means more
 * confidence in the aggregated facts. Extraction happens server-side because
 * transferring full session JSON for dozens of multi-MB trace files to the
 * browser just to extract a handful of facts doesn't scale.
 *
 * This is a CORRECTIONS module (only fields we actually have data for),
 * meant to be layered on top of Questie's base DB - not a full DB dump.
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

  const handleCopy = (npcFixes: string) => {
    navigator.clipboard.writeText(npcFixes);
  };

  const handleDownload = (npcFixes: string) => {
    const blob = new Blob([npcFixes], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "ForeverTraceNpcFixes.lua";
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
            <button onClick={() => handleCopy(status.result.npcFixes)}>Copy to clipboard</button>
            <button onClick={() => handleDownload(status.result.npcFixes)}>Download ForeverTraceNpcFixes.lua</button>
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
          <pre className="extract-output">{status.result.npcFixes}</pre>
        </>
      )}
    </div>
  );
}
