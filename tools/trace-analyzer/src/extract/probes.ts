// Thin, pure wrappers over the existing emulator (`core/emulator.ts`) so
// extraction code reads like domain questions ("what was the player's zone at
// time t?") instead of raw stream plumbing.

import { emulate, getStream, valueAt } from "../core/emulator";
import type { SessionRecord } from "../core/types";
import { nearestByTime } from "./correlate";

export interface MapPosition {
  x: number;
  y: number;
}

/** Convert C_Map's normalized 0-1 coordinates to Questie's 0-100 percentages. */
export function toQuestieMapPosition(position: MapPosition): MapPosition {
  return {
    x: Math.round(position.x * 10_000) / 100,
    y: Math.round(position.y * 10_000) / 100,
  };
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

/**
 * `UnitName(token)` at or before time `t`.
 *
 * UnitName returns packed args (name, realm) - `emulate()` unpacks the packed
 * `{1: name, 2: realm, n: 2}` shape into `[name, realm]` so we can take the name.
 */
export function unitNameAt(session: SessionRecord, token: string, t: number): string | null {
  const raw = valueAt(getStream(session, "UnitName", token) ?? [], t);
  const unpacked = emulate(raw);
  const name = Array.isArray(unpacked) ? unpacked[0] : raw;
  return asNonEmptyString(name);
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

/** `C_QuestLog.GetInfo(questID).title` at or before time `t`. */
export function questInfoTitleAt(session: SessionRecord, questID: number, t: number): string | null {
  const value = valueAt(getStream(session, "C_QuestLog.GetInfo", questID) ?? [], t);
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  return asNonEmptyString((value as Record<string, unknown>).title);
}

/** `GetLocale()` at or before time `t`, e.g. "enUS", "deDE", "frFR". */
export function getLocaleAt(session: SessionRecord, t: number): string | null {
  return asNonEmptyString(valueAt(getStream(session, "GetLocale", "player") ?? [], t));
}
