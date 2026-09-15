import { useRef, useEffect, useMemo, useState } from "react";
import type { SessionRecord, FunctionStreamEntry } from "../../core/types.js";
import { getStream, valueAt } from "../../core/emulator.js";
import { findMapArt, mapArtUrl } from "../../core/map-art.js";
import { useMapImage, preloadMapArt } from "../hooks/useMapImage.js";

interface PositionPlotProps {
  session: SessionRecord;
  currentTime: number;
}

interface Point {
  x: number;
  y: number;
  t: number;
  /**
   * Which map this sample's 0-1 coordinates belong to. Prefers the uiMapID and
   * falls back to the zone name, so a trace that records only GetZoneText still
   * segments correctly. Null when the trace identifies no map at all.
   */
  mapKey: string | null;
}

/** Identify a map the same way for samples and for the current time. */
function mapKeyOf(uiMapId: unknown, zoneName: unknown): string | null {
  if (typeof uiMapId === "number") return `id:${uiMapId}`;
  if (typeof zoneName === "string") return `zone:${zoneName}`;
  return null;
}

export function PositionPlot({ session, currentTime }: PositionPlotProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [showMap, setShowMap] = useState(true);

  // Streams identifying which map the 0-1 coordinates belong to
  const mapStream = useMemo(
    () => getStream(session, "C_Map.GetBestMapForUnit", "player"),
    [session]
  );
  const zoneStream = useMemo(() => getStream(session, "GetZoneText"), [session]);

  // Extract all position points, tagging each with the map it was recorded on
  const { points, stream } = useMemo(() => {
    const s = getStream(session, "C_Map.GetPlayerMapPosition", "player");
    if (!s) return { points: [] as Point[], stream: undefined };

    const pts: Point[] = [];
    for (const entry of s as FunctionStreamEntry[]) {
      if (entry.v && typeof entry.v === "object") {
        const pos = entry.v as { x?: number; y?: number };
        if (typeof pos.x === "number" && typeof pos.y === "number") {
          pts.push({
            x: pos.x,
            y: pos.y,
            t: entry.t,
            mapKey: mapKeyOf(
              mapStream ? valueAt(mapStream, entry.t) : undefined,
              zoneStream ? valueAt(zoneStream, entry.t) : undefined
            ),
          });
        }
      }
    }
    return { points: pts, stream: s as FunctionStreamEntry[] };
  }, [session, mapStream, zoneStream]);

  // Current position
  const currentPos = useMemo(() => {
    if (!stream) return null;
    const v = valueAt(stream, currentTime);
    if (v && typeof v === "object") {
      const pos = v as { x?: number; y?: number };
      if (typeof pos.x === "number" && typeof pos.y === "number") {
        return { x: pos.x, y: pos.y };
      }
    }
    return null;
  }, [stream, currentTime]);

  // Map art for whichever zone the player is in at the current time
  const { activeMapId, activeKey, art } = useMemo(() => {
    const m = mapStream ? valueAt(mapStream, currentTime) : undefined;
    const z = zoneStream ? valueAt(zoneStream, currentTime) : undefined;
    const mapId = typeof m === "number" ? m : null;
    const zoneName = typeof z === "string" ? z : null;
    return {
      activeMapId: mapId,
      activeKey: mapKeyOf(m, z),
      art: findMapArt(mapId, zoneName),
    };
  }, [mapStream, zoneStream, currentTime]);

  const { image, status } = useMapImage(art ? mapArtUrl(art) : null);
  const mapVisible = showMap && status === "loaded" && image !== null;

  // Every distinct map the session passes through, so crossing a zone boundary
  // swaps straight to the next backdrop instead of dropping to the grid while
  // it downloads.
  const sessionArtUrls = useMemo(() => {
    if (!mapStream) return [];
    const urls = new Set<string>();
    for (const entry of mapStream) {
      if (typeof entry.v !== "number") continue;
      const z = zoneStream ? valueAt(zoneStream, entry.t) : undefined;
      const a = findMapArt(entry.v, typeof z === "string" ? z : null);
      if (a) urls.add(mapArtUrl(a));
    }
    return [...urls];
  }, [mapStream, zoneStream]);

  useEffect(() => {
    preloadMapArt(sessionArtUrls);
  }, [sessionArtUrls]);

  // Draw
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || points.length === 0) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    // WoW map art is 3:2 by design (1002x668) and uiMap 0-1 coordinates span
    // exactly that rectangle, so the plot box matches the art box.
    const MAP_ASPECT = 1.5;
    const rect = canvas.parentElement!.getBoundingClientRect();
    const maxW = rect.width - 20;
    const maxH = rect.height - 60;
    // Fit the 3:2 rectangle within available space
    let w = maxW;
    let h = w / MAP_ASPECT;
    if (h > maxH) {
      h = maxH;
      w = h * MAP_ASPECT;
    }
    canvas.width = w;
    canvas.height = h;

    // Fixed 0-1 bounds (WoW map coordinates are always 0-1)
    const pad = 30;

    const toCanvasX = (x: number) => pad + x * (w - 2 * pad);
    const toCanvasY = (y: number) => pad + y * (h - 2 * pad);

    // Clear
    ctx.fillStyle = "#1a1a2e";
    ctx.fillRect(0, 0, w, h);

    if (mapVisible) {
      // Every asset is the native 1002x668 art, so it fills exactly the box the
      // coordinate transform maps into — toCanvasX/toCanvasY stay the sole
      // source of truth for alignment.
      ctx.drawImage(image, pad, pad, w - 2 * pad, h - 2 * pad);
    } else {
      // Draw grid (lines at 0, 0.25, 0.50, 0.75, 1.0)
      ctx.strokeStyle = "#2a2a3e";
      ctx.lineWidth = 0.5;
      for (let i = 0; i <= 4; i++) {
        const cx = toCanvasX(i / 4);
        const cy = toCanvasY(i / 4);
        ctx.beginPath();
        ctx.moveTo(cx, pad);
        ctx.lineTo(cx, h - pad);
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(pad, cy);
        ctx.lineTo(w - pad, cy);
        ctx.stroke();
      }
    }

    // Time range for gradient
    const maxT = points[points.length - 1].t;

    // Draw path segments with time gradient.
    //
    // Coordinates are normalised per map, so a sample only means anything on
    // the map it was recorded against: drawing another map's samples here would
    // trace the player through places they never went, and a segment spanning
    // two maps would teleport across the canvas. Both endpoints must therefore
    // belong to the map being shown right now.
    //
    // This holds whether the backdrop is art or the bare grid — the grid is
    // still that one map's 0-1 space. Gating it on the art being loaded is what
    // previously let a zone's route linger over the next zone's map while the
    // incoming image was still downloading.
    for (let i = 1; i < points.length; i++) {
      const prev = points[i - 1];
      const cur = points[i];
      if (prev.mapKey !== activeKey || cur.mapKey !== activeKey) continue;

      const frac = cur.t / (maxT || 1);
      // Blue (start) -> Red (end)
      const r = Math.round(50 + 180 * frac);
      const g = Math.round(80 * (1 - frac));
      const b = Math.round(200 * (1 - frac));
      ctx.strokeStyle = `rgba(${r},${g},${b},${mapVisible ? 0.85 : 0.6})`;
      ctx.lineWidth = mapVisible ? 2 : 1.5;
      ctx.beginPath();
      ctx.moveTo(toCanvasX(prev.x), toCanvasY(prev.y));
      ctx.lineTo(toCanvasX(cur.x), toCanvasY(cur.y));
      ctx.stroke();
    }

    // Draw current position
    if (currentPos) {
      const cx = toCanvasX(currentPos.x);
      const cy = toCanvasY(currentPos.y);

      // Glow
      ctx.beginPath();
      ctx.arc(cx, cy, 8, 0, Math.PI * 2);
      ctx.fillStyle = "rgba(255, 255, 100, 0.3)";
      ctx.fill();

      // Dot
      ctx.beginPath();
      ctx.arc(cx, cy, 4, 0, Math.PI * 2);
      ctx.fillStyle = "#ffff66";
      ctx.fill();
      ctx.strokeStyle = "#ffffff";
      ctx.lineWidth = 1;
      ctx.stroke();
    }

    // Axis labels
    ctx.fillStyle = "#666";
    ctx.font = "10px monospace";
    ctx.textAlign = "center";
    ctx.fillText("0", pad, h - 5);
    ctx.fillText("1", w - pad, h - 5);
    ctx.textAlign = "left";
    ctx.fillText("0", 2, pad - 5);
    ctx.fillText("1", 2, h - pad + 12);
    ctx.textAlign = "center";
    ctx.fillText("x", w / 2, h - 5);
    ctx.textAlign = "left";
    ctx.fillText("y", 2, h / 2);

    // Current coords overlay
    if (currentPos) {
      const label = `x: ${currentPos.x.toFixed(4)}, y: ${currentPos.y.toFixed(4)}`;
      ctx.font = "12px monospace";
      ctx.textAlign = "right";
      if (mapVisible) {
        // Keep the readout legible against bright map art.
        ctx.strokeStyle = "rgba(0,0,0,0.85)";
        ctx.lineWidth = 3;
        ctx.strokeText(label, w - 5, 15);
      }
      ctx.fillStyle = mapVisible ? "#fff" : "#ccc";
      ctx.fillText(label, w - 5, 15);
    }
  }, [points, currentPos, mapVisible, image, activeMapId]);

  if (points.length === 0) {
    return (
      <div className="position-plot">
        <div className="stream-empty">No position data available</div>
      </div>
    );
  }

  const toggleTitle = art
    ? `Show the ${art.zoneName} map behind the route`
    : `No map art registered for this zone (uiMapID ${activeMapId ?? "unknown"})`;

  return (
    <div className="position-plot">
      <div className="position-header">
        <div className="position-info">
          {points.length} position samples
          {currentPos && (
            <span>
              {" "}| Current: ({currentPos.x.toFixed(4)}, {currentPos.y.toFixed(4)})
            </span>
          )}
          {showMap && art && status === "loading" && (
            <span className="position-note"> | loading map...</span>
          )}
          {showMap && art && status === "error" && (
            <span className="position-note"> | map art unavailable</span>
          )}
          {art && mapVisible && (
            <span className="position-note"> | {art.zoneName}</span>
          )}
        </div>
        <label className="position-toggle" title={toggleTitle}>
          <input
            type="checkbox"
            checked={showMap}
            disabled={!art}
            onChange={(e) => setShowMap(e.target.checked)}
          />
          Show map
        </label>
      </div>
      <canvas ref={canvasRef} />
    </div>
  );
}
