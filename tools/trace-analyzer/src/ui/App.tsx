import { useState, useEffect, useMemo } from "react";
import { useTraceFiles, useSessionList, useSession } from "./hooks/useSession.js";
import { useTimeline } from "./hooks/useTimeline.js";
import { getStream, valueAt, emulate, formatTime } from "../core/emulator.js";
import { Timeline } from "./components/Timeline.js";
import { StreamViewer } from "./components/StreamViewer.js";
import { EventLog } from "./components/EventLog.js";
import { PositionPlot } from "./components/PositionPlot.js";
import { ExtractView } from "./components/ExtractView.js";
import type { SessionRecord, SessionSummary } from "../core/types.js";
import "./App.css";

type Tab = "streams" | "events" | "position" | "extract";

/** In-progress sessions bundled via Core.BuildExportPayload() have no `name` yet. */
function sessionLabel(s: SessionSummary): string {
  return s.name ?? `Session ${s.index + 1} (unsaved)`;
}

function PlayerIdentity({ session, t }: { session: SessionRecord; t: number }) {
  const info = useMemo(() => {
    const raceStream = getStream(session, "UnitRace", "player");
    const classStream = getStream(session, "UnitClass", "player");
    const levelStream = getStream(session, "UnitLevel", "player");
    const zoneStream = getStream(session, "GetZoneText");
    const sexStream = getStream(session, "UnitSex", "player");

    const raceVal = raceStream ? emulate(valueAt(raceStream, t)) : null;
    const classVal = classStream ? emulate(valueAt(classStream, t)) : null;
    const level = levelStream ? valueAt(levelStream, t) : null;
    const zone = zoneStream ? valueAt(zoneStream, t) : null;
    const sex = sexStream ? valueAt(sexStream, t) : null;

    const raceName = Array.isArray(raceVal) ? String(raceVal[0]) : null;
    const classDisplayName = Array.isArray(classVal) ? String(classVal[0]) : null;
    const sexSymbol = sex === 2 ? "\u2642" : sex === 3 ? "\u2640" : "";
    const levelNum = typeof level === "number" ? level : null;
    const zoneName = typeof zone === "string" ? zone : null;

    return { raceName, classDisplayName, levelNum, zoneName, sexSymbol };
  }, [session, t]);

  return (
    <div className="player-identity">
      {info.raceName && <span>{info.raceName}</span>}
      {info.classDisplayName && <span>{info.classDisplayName}</span>}
      {info.sexSymbol && <span>{info.sexSymbol}</span>}
      {info.levelNum != null && <span>Lv.{info.levelNum}</span>}
      {info.zoneName && <span>- {info.zoneName}</span>}
    </div>
  );
}

export function App() {
  const { files, loading: filesLoading, error: filesError } = useTraceFiles();
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const [selectedSessionIndex, setSelectedSessionIndex] = useState<number | null>(null);
  const [activeTab, setActiveTab] = useState<Tab>("streams");

  const {
    sessions,
    loading: sessionsLoading,
    error: sessionsError,
  } = useSessionList(selectedFile);
  const {
    session,
    loading: sessionLoading,
    error: sessionError,
  } = useSession(selectedFile, selectedSessionIndex);

  // Auto-select first file if only one
  useEffect(() => {
    if (files.length > 0 && !selectedFile) {
      setSelectedFile(files[0].name);
    }
  }, [files, selectedFile]);

  // Auto-select first session when file changes
  useEffect(() => {
    if (sessions.length > 0) {
      setSelectedSessionIndex(sessions[0].index);
    } else {
      setSelectedSessionIndex(null);
    }
  }, [sessions]);

  const duration = session?.duration ?? 0;
  const timeline = useTimeline(duration);

  if (filesLoading) return <div className="loading">Loading trace files...</div>;
  if (filesError) return <div className="error-msg">Error: {filesError}</div>;
  if (files.length === 0)
    return <div className="error-msg">No .lua files found in Traces/</div>;

  return (
    <>
      {/* Header */}
      <div className="app-header">
        <h1>Trace Analyzer</h1>

        {/* File selector */}
        <select
          className="file-select"
          value={selectedFile ?? ""}
          onChange={(e) => {
            setSelectedFile(e.target.value || null);
            setSelectedSessionIndex(null);
          }}
        >
          {files.length > 1 && <option value="">Select file...</option>}
          {files.map((f) => (
            <option key={f.name} value={f.name}>
              {f.name}
            </option>
          ))}
        </select>

        {/* Session selector */}
        {sessions.length > 1 && (
          <select
            value={selectedSessionIndex ?? ""}
            onChange={(e) => setSelectedSessionIndex(e.target.value === "" ? null : Number(e.target.value))}
          >
            <option value="">Select session...</option>
            {sessions.map((s) => (
              <option key={s.index} value={s.index}>
                {sessionLabel(s)} ({formatTime(s.duration ?? 0)})
              </option>
            ))}
          </select>
        )}
        {sessions.length === 1 && (
          <span style={{ fontFamily: "monospace", fontSize: 12, color: "#888" }}>
            {sessionLabel(sessions[0])}
          </span>
        )}

        {session && <PlayerIdentity session={session} t={timeline.currentTime} />}
      </div>

      {/* Timeline */}
      {session && (
        <Timeline
          duration={duration}
          currentTime={timeline.currentTime}
          playing={timeline.playing}
          playSpeed={timeline.playSpeed}
          onSeek={timeline.seek}
          onToggle={timeline.toggle}
          onSpeedChange={timeline.setPlaySpeed}
        />
      )}

      {/* Tabs */}
      <div className="tab-bar">
        <button
          className={activeTab === "streams" ? "active" : ""}
          onClick={() => setActiveTab("streams")}
        >
          Streams
        </button>
        <button
          className={activeTab === "events" ? "active" : ""}
          onClick={() => setActiveTab("events")}
        >
          Events
        </button>
        <button
          className={activeTab === "position" ? "active" : ""}
          onClick={() => setActiveTab("position")}
        >
          Position
        </button>
        <button
          className={activeTab === "extract" ? "active" : ""}
          onClick={() => setActiveTab("extract")}
        >
          Extract
        </button>
      </div>

      {/* Content */}
      <div className="tab-content">
        {(sessionsLoading || sessionLoading) && (
          <div className="loading">Loading...</div>
        )}
        {(sessionsError || sessionError) && (
          <div className="error-msg">
            Error: {sessionsError || sessionError}
          </div>
        )}
        {session && (
          <>
            {activeTab === "streams" && (
              <StreamViewer
                session={session}
                currentTime={timeline.currentTime}
                onSeek={timeline.seek}
              />
            )}
            {activeTab === "events" && (
              <EventLog
                session={session}
                currentTime={timeline.currentTime}
                onSeek={timeline.seek}
              />
            )}
            {activeTab === "position" && (
              <PositionPlot
                session={session}
                currentTime={timeline.currentTime}
              />
            )}
          </>
        )}
        {activeTab === "extract" && selectedFile && <ExtractView fileName={selectedFile} />}
      </div>
    </>
  );
}
