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
  { key: "npc", label: "NPC", downloadName: "ForeverTraceNpcFixes.lua" },
  { key: "quest", label: "Quest", downloadName: "ForeverTraceQuestFixes.lua" },
  { key: "item", label: "Item", downloadName: "ForeverTraceItemFixes.lua" },
  { key: "object", label: "Object", downloadName: "ForeverTraceObjectFixes.lua" },
];

type Status =
  | { kind: "idle" }
  | { kind: "loading"; entity?: EntitySection["key"] }
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
  const [activeTab, setActiveTab] = useState<EntitySection["key"]>("npc");

  const handleGenerateAll = () => {
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

  const handleGenerateEntity = (entity: EntitySection["key"]) => {
    setStatus({ kind: "loading", entity });
    fetch(`/api/extract/${entity}`)
      .then((r) => r.json())
      .then((data: ExtractResult & { error?: string }) => {
        if (data.error) throw new Error(data.error);
        if (status.kind === "done") {
          setStatus({ kind: "done", results: { ...status.results, [entity]: data } });
        } else {
          const result = { [entity]: data } as Record<EntitySection["key"], ExtractResult>;
          // Fill in missing entities with placeholder data (so tabs all exist)
          for (const section of SECTIONS) {
            if (!(section.key in result)) {
              result[section.key] = { fixes: "", sessionCount: 0, fileCount: 0, skippedFiles: [] };
            }
          }
          setStatus({ kind: "done", results: result });
        }
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

  const isLoading = status.kind === "loading";
  const isDone = status.kind === "done";
  const activeResult = isDone ? status.results[activeTab] : null;
  const isGeneratingEntity = status.kind === "loading" && status.entity !== undefined ? status.entity : null;
  const hasResults = isDone;

  return (
    <div className="extract-view">
      <div className="extract-toolbar">
        <button onClick={handleGenerateAll} disabled={isLoading}>
          {isLoading && !status.entity ? "Generating all..." : "Generate all"}
        </button>
      </div>

      {status.kind === "error" && <div className="error-msg">Error: {status.message}</div>}

      {hasResults && (
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
        </>
      )}

      <div className="extract-tabs">
        <div className="extract-tab-list">
          {SECTIONS.map((section) => (
            <button
              key={section.key}
              className={`extract-tab ${activeTab === section.key ? "active" : ""}`}
              onClick={() => setActiveTab(section.key)}
            >
              {section.label}
            </button>
          ))}
        </div>

        <div className="extract-tab-content">
          {activeResult ? (
            <>
              <div className="extract-tab-header">
                <button
                  onClick={() => handleGenerateEntity(activeTab)}
                  disabled={isGeneratingEntity === activeTab}
                >
                  {isGeneratingEntity === activeTab ? "Generating..." : "Generate"}
                </button>
                <button onClick={() => handleCopy(activeResult.fixes)} disabled={!activeResult.fixes}>
                  Copy to clipboard
                </button>
                <button
                  onClick={() => handleDownload(activeResult.fixes, SECTIONS.find((s) => s.key === activeTab)!.downloadName)}
                  disabled={!activeResult.fixes}
                >
                  Download
                </button>
              </div>
              <pre className="extract-output">{activeResult.fixes || "(No data — click Generate to extract)"}</pre>
            </>
          ) : (
            <>
              <div className="extract-tab-header">
                <button
                  onClick={() => handleGenerateEntity(activeTab)}
                  disabled={isGeneratingEntity === activeTab}
                >
                  {isGeneratingEntity === activeTab ? "Generating..." : "Generate"}
                </button>
              </div>
              <div className="extract-tab-placeholder">
                <p>Click "Generate all" to extract from all trace files,</p>
                <p>or use the per-entity "Generate" button after selecting a tab.</p>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

