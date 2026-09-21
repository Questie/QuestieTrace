import { useMemo } from "react";
import { extractAll } from "../../extract/index.js";
import { useAllSessions } from "../hooks/useAllSessions.js";

interface ExtractViewProps {
  fileName: string;
}

/**
 * "Extract" tab content: runs the full observe -> aggregate -> emit -> write
 * chain over ALL sessions of the selected trace file (not just the single
 * session picked in the session dropdown) and renders the resulting
 * Questie-format Lua for copy/download.
 *
 * Currently only the npc entity kind has observers wired up (see
 * src/extract/index.ts); quest/item/object will get their own output +
 * sub-navigation once their observers land.
 */
export function ExtractView({ fileName }: ExtractViewProps) {
  const { sessions, loading, error } = useAllSessions(fileName);

  const result = useMemo(() => {
    if (sessions.length === 0) return null;
    return extractAll(sessions, { sourceFileName: fileName });
  }, [sessions, fileName]);

  if (loading) {
    return <div className="loading">Loading sessions...</div>;
  }
  if (error) {
    return <div className="error-msg">Error: {error}</div>;
  }
  if (!result) {
    return <div className="loading">No sessions to extract from.</div>;
  }

  const handleCopy = () => {
    navigator.clipboard.writeText(result.npcDB);
  };

  const handleDownload = () => {
    const blob = new Blob([result.npcDB], { type: "text/plain" });
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
        <button onClick={handleCopy}>Copy to clipboard</button>
        <button onClick={handleDownload}>Download npcDB.lua</button>
      </div>
      <pre className="extract-output">{result.npcDB}</pre>
    </div>
  );
}
