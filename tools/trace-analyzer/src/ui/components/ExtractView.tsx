import { useState } from "react";

interface ExtractResult {
  fixes: string;
  sessionCount: number;
  fileCount: number;
  skippedFiles: { name: string; error: string }[];
}

interface EntitySection {
  key: "npc" | "quest" | "item" | "object";
  label: string;
  downloadName: string;
}

const SECTIONS: EntitySection[] = [
  { key: "npc", label: "NPC Fixes", downloadName: "ForeverTraceNpcFixes.lua" },
  { key: "quest", label: "Quest Fixes", downloadName: "ForeverTraceQuestFixes.lua" },
  { key: "item", label: "Item Fixes", downloadName: "ForeverTraceItemFixes.lua" },
  { key: "object", label: "Object Fixes", downloadName: "ForeverTraceObjectFixes.lua" },
];

type Status =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "done"; results: Record<EntitySection["key"], ExtractResult> }
  | { kind: "error"; message: string };

/**
 * "Extract" tab content: on demand, runs the full observe -> aggregate ->
 * emit -> write chain over EVERY trace file in Traces/ (server-side, one
 * request per entity kind via /api/extract/{npc,quest,item,object}) and
 * renders each resulting ForeverTrace*Fixes.lua corrections module for
 * copy/download.
 *
 * Combining across all trace files - not just the one selected in the file
 * dropdown - is the whole point of this tool: more traces means more
 * confidence in the aggregated facts. Extraction happens server-side because
 * transferring full session JSON for dozens of multi-MB trace files to the
 * browser just to extract a handful of facts doesn't scale.
 *
 * These are CORRECTIONS modules (only fields we actually have data for),
 * meant to be layered on top of Questie's base DB - not a full DB dump.
 */
export function ExtractView() {
  const [status, setStatus] = useState<Status>({ kind: "idle" });

  const handleGenerate = () => {
    setStatus({ kind: "loading" });
    Promise.all(
      SECTIONS.map((section) =>
        fetch(`/api/extract/${section.key}`)
          .then((r) => r.json())
          .then((data: ExtractResult & { error?: string }) => {
            if (data.error) throw new Error(data.error);
            return [section.key, data] as const;
          }),
      ),
    )
      .then((entries) => {
        const results = Object.fromEntries(entries) as Record<EntitySection["key"], ExtractResult>;
        setStatus({ kind: "done", results });
      })
      .catch((e) => setStatus({ kind: "error", message: e.message }));
  };

  const handleCopy = (fixes: string) => {
    navigator.clipboard.writeText(fixes);
  };

  const handleDownload = (fixes: string, downloadName: string) => {
    const blob = new Blob([fixes], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = downloadName;
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div className="extract-view">
      <div className="extract-toolbar">
        <button onClick={handleGenerate} disabled={status.kind === "loading"}>
          {status.kind === "loading" ? "Generating..." : "Generate"}
        </button>
      </div>

      {status.kind === "error" && <div className="error-msg">Error: {status.message}</div>}

      {status.kind === "done" && (
        <>
          <div className="extract-summary">
            {status.results.npc.sessionCount} session(s) from {status.results.npc.fileCount} trace file(s)
          </div>
          {status.results.npc.skippedFiles.length > 0 && (
            <div className="extract-warning">
              Skipped {status.results.npc.skippedFiles.length} file(s) that failed to load:
              <ul>
                {status.results.npc.skippedFiles.map((f) => (
                  <li key={f.name}>
                    {f.name}: {f.error}
                  </li>
                ))}
              </ul>
            </div>
          )}
          {SECTIONS.map((section) => {
            const result = status.results[section.key];
            return (
              <div className="extract-section" key={section.key}>
                <div className="extract-section-header">
                  <h3>{section.label}</h3>
                  <button onClick={() => handleCopy(result.fixes)}>Copy to clipboard</button>
                  <button onClick={() => handleDownload(result.fixes, section.downloadName)}>
                    Download {section.downloadName}
                  </button>
                </div>
                <pre className="extract-output">{result.fixes}</pre>
              </div>
            );
          })}
        </>
      )}
    </div>
  );
}
