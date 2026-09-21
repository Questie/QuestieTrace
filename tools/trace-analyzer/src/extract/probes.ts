// Thin, pure wrappers over the existing emulator (`core/emulator.ts`) so
// extraction code reads like domain questions ("what was the player's zone at
// time t?") instead of raw stream plumbing.

import { getStream, valueAt } from "../core/emulator";
import type { SessionRecord } from "../core/types";
import { nearestByTime } from "./correlate";

export interface MapPosition {
  x: number;
  y: number;
}

function asNumber(v: unknown): number | null {
  return typeof v === "number" ? v : null;
}

function asNonEmptyString(v: unknown): string | null {
  return typeof v === "string" && v.length > 0 ? v : null;
}

function asMapPosition(v: unknown): MapPosition | null {
  if (!v || typeof v !== "object") return null;
  const obj = v as { x?: unknown; y?: unknown };
  if (typeof obj.x !== "number" || typeof obj.y !== "number") return null;
  return { x: obj.x, y: obj.y };
}

/** `C_Map.GetBestMapForUnit("player")` at or before time `t`. */
export function playerZoneAt(session: SessionRecord, t: number): number | null {
  return asNumber(valueAt(getStream(session, "C_Map.GetBestMapForUnit", "player") ?? [], t));
}

/**
 * `C_Map.GetBestMapForUnit("player")` nearest time `t` (both directions),
 * within the shared correlation window. Used for positional approximation
 * (e.g. NPC/object spawn zone ~= player zone at interaction time).
 */
export function playerZoneNear(session: SessionRecord, t: number, windowSeconds?: number): number | null {
  return asNumber(nearestByTime(getStream(session, "C_Map.GetBestMapForUnit", "player"), t, windowSeconds));
}

/** `C_Map.GetPlayerMapPosition("player")` at or before time `t`. */
export function playerPosAt(session: SessionRecord, t: number): MapPosition | null {
  return asMapPosition(valueAt(getStream(session, "C_Map.GetPlayerMapPosition", "player") ?? [], t));
}

/** `C_Map.GetPlayerMapPosition("player")` nearest time `t`, within the window. */
export function playerPosNear(session: SessionRecord, t: number, windowSeconds?: number): MapPosition | null {
  return asMapPosition(nearestByTime(getStream(session, "C_Map.GetPlayerMapPosition", "player"), t, windowSeconds));
}

/** `UnitGUID(token)` at or before time `t`, e.g. token = "target"/"npc"/"questnpc". */
export function unitGuidAt(session: SessionRecord, token: string, t: number): string | null {
  return asNonEmptyString(valueAt(getStream(session, "UnitGUID", token) ?? [], t));
}

/** `UnitName(token)` at or before time `t`. */
export function unitNameAt(session: SessionRecord, token: string, t: number): string | null {
  return asNonEmptyString(valueAt(getStream(session, "UnitName", token) ?? [], t));
}

/** `UnitLevel(token)` at or before time `t`. */
export function unitLevelAt(session: SessionRecord, token: string, t: number): number | null {
  const v = asNumber(valueAt(getStream(session, "UnitLevel", token) ?? [], t));
  return v && v > 0 ? v : null;
}

/** `GetQuestID()` at or before time `t` (0 means "no active quest frame", normalized to null). */
export function questIdAt(session: SessionRecord, t: number): number | null {
  const v = asNumber(valueAt(getStream(session, "GetQuestID") ?? [], t));
  return v && v !== 0 ? v : null;
}

/** `GetTitleText()` at or before time `t` (quest detail/gossip frame title). */
export function questTitleAt(session: SessionRecord, t: number): string | null {
  return asNonEmptyString(valueAt(getStream(session, "GetTitleText") ?? [], t));
}
