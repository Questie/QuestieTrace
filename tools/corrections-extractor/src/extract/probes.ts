// Thin, pure wrappers over the existing emulator (`core/emulator.ts`) so
// extraction code reads like domain questions ("what was the player's zone at
// time t?") instead of raw stream plumbing.

import { emulate, getStream, valueAt } from "../core/emulator";
import type { SessionRecord } from "../core/types";
import { nearestByTime } from "./correlate";
import { areaIdForUiMapId } from "./resources/ui-map-to-area";

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

/**
 * `C_Map.GetBestMapForUnit("player")` at or before time `t`, translated to the
 * AreaTable areaID the Questie export keys zones by (the API reports uiMapIDs,
 * e.g. Elwynn Forest is uiMapID 1429 but areaID 12). Returns null when the
 * uiMapID has no corresponding area (continents, dungeons, unknown ids).
 */
export function playerZoneAt(session: SessionRecord, t: number): number | null {
  return areaIdForUiMapId(asNumber(valueAt(getStream(session, "C_Map.GetBestMapForUnit", "player") ?? [], t)));
}

/**
 * `C_Map.GetBestMapForUnit("player")` nearest time `t` (both directions),
 * within the shared correlation window. Used for positional approximation
 * (e.g. NPC/object spawn zone ~= player zone at interaction time).
 */
export function playerZoneNear(session: SessionRecord, t: number, windowSeconds?: number): number | null {
  return areaIdForUiMapId(asNumber(nearestByTime(getStream(session, "C_Map.GetBestMapForUnit", "player"), t, windowSeconds)));
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
 *
 * The WoW client returns a few NPC names with embedded line breaks (observed in
 * traces as `"Telenos \<newline>Leafwhisper"`); strip those breaks out entirely
 * (the name text around them is intact, usually with a space before the break).
 */
export function unitNameAt(session: SessionRecord, token: string, t: number): string | null {
  const raw = valueAt(getStream(session, "UnitName", token) ?? [], t);
  const unpacked = emulate(raw);
  const name = Array.isArray(unpacked) ? unpacked[0] : raw;
  if (typeof name !== "string") return null;
  return asNonEmptyString(name.replace(/[\r\n]+/g, "").trim());
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

/** `UnitFactionGroup("player")` at or before time `t` as Questie's "A"/"H", or null (e.g. unfactioned). */
export function playerFactionAt(session: SessionRecord, t: number): "A" | "H" | null {
  const raw = valueAt(getStream(session, "UnitFactionGroup", "player") ?? [], t);
  const faction = raw && typeof raw === "object" ? (raw as Record<number, unknown>)[1] : raw;
  if (faction === "Alliance") return "A";
  if (faction === "Horde") return "H";
  return null;
}

/** The locale-independent race file name of `UnitRace("player")` at or before time `t`, e.g. "Human". */
export function playerRaceAt(session: SessionRecord, t: number): string | null {
  const raw = valueAt(getStream(session, "UnitRace", "player") ?? [], t);
  if (!raw || typeof raw !== "object") return null;
  return asNonEmptyString((raw as Record<number, unknown>)[2]);
}

/** The first `GetServerTime()` sample of the session, i.e. the realm time around its start. */
export function sessionServerTime(session: SessionRecord): number | null {
  return asNumber(getStream(session, "GetServerTime")?.[0]?.v);
}

/** Completed-quest streams in `functionsDelta`, preferred first (see Modules/Trackers/CompletedQuests.lua). */
const COMPLETED_QUEST_STREAMS = ["C_QuestLog.GetAllCompletedQuestIDs", "GetQuestsCompleted"];

/**
 * The quests the player had completed at time `t` (session-relative `GetTime`, i.e. an event's
 * `t`): the stream's initial set with every change recorded at or before `t` applied. Undefined
 * when the session recorded no completed quests, which is not the same as an empty set.
 *
 * Changes are recorded shortly after the turn-in (median 0.15 s, 99th percentile 0.5 s). Measured
 * on trace-data in 2026-11, 1 of 7,521 accepts within 30 s of a turn-in came before its change,
 * so the predecessor of a quest accepted right after turning it in is practically always included.
 * The initial set can be empty while the client still loads it at login; the loaded quests then
 * arrive as one change.
 */
export function completedQuestsAt(session: SessionRecord, t: number): Set<number> | undefined {
  const streams = COMPLETED_QUEST_STREAMS.map((name) => session.functionsDelta?.[name]).filter((stream) => stream !== undefined);
  // A client can have the preferred API but leave it empty: then a recorded fallback has the data.
  const stream = streams.find((candidate) => asArray(candidate.initial).length > 0 || asArray(candidate.delta).length > 0) ?? streams[0];
  if (stream === undefined) return undefined;
  const completed = new Set<number>(asArray(stream.initial));
  for (const change of asArray(stream.delta)) {
    if (!(change.t <= t)) continue;
    for (const id of asArray(change.add)) completed.add(id);
    for (const id of asArray(change.remove)) completed.delete(id);
  }
  return completed;
}

function asArray<T>(value: T[] | undefined): T[] {
  return Array.isArray(value) ? value : [];
}
