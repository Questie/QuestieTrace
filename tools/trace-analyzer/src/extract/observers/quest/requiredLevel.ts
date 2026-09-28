// questKeys.requiredLevel - Questie field 4.
//
// Primary source: QuestAcceptLevel stream recorded by addon at QUEST_ACCEPTED
// (player's level at acceptance time). Take the minimum observed acceptance
// level across all sessions.
//
// Fallback: If QuestAcceptLevel is not available (older traces), correlate
// QUEST_ACCEPTED events with the UnitLevel("player") stream to find the
// player's level at acceptance time, then take the minimum across sessions.

import { getParamKeys, getStream, valueAt } from "../../../core/emulator";
import type { SessionRecord, EventEntry } from "../../../core/types";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

function unpackQuestId(args: unknown): number | null {
  if (args !== null && args !== undefined && typeof args === "object" && !Array.isArray(args)) {
    const obj = args as Record<string, unknown>;
    if (typeof obj.n === "number" && typeof obj[2] === "number") {
      return obj[2] as number;
    }
    if (typeof obj.n === "number" && typeof obj[1] === "number") {
      return obj[1] as number;
    }
  }
  return null;
}

function findPlayerLevelAtTime(session: SessionRecord, targetT: number): number | null {
  const unitLevelStream = getStream(session, "UnitLevel", "player");
  if (!unitLevelStream || unitLevelStream.length === 0) return null;

  const level = valueAt(unitLevelStream, targetT);
  return typeof level === "number" && Number.isFinite(level) ? level : null;
}

export function mergeRequiredLevel(observations: Observation<number>[]): number {
  let min = Infinity;
  for (const obs of observations) {
    if (obs.value < min) min = obs.value;
  }
  return min;
}

export const observeRequiredLevel: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];

  // --- Primary: QuestAcceptLevel (direct observation at acceptance) ---
  const acceptRoot = session.functions["QuestAcceptLevel"];
  if (acceptRoot && !Array.isArray(acceptRoot)) {
    const acceptObs = new Map<number, { level: number; t: number }>();

    for (const questIdKey of getParamKeys(acceptRoot)) {
      const questID = Number(questIdKey);
      if (!Number.isFinite(questID)) continue;

      const stream = getStream(session, "QuestAcceptLevel", questIdKey) ?? [];
      if (stream.length === 0) continue;

      let minLevel: number | null = null;
      let firstT: number | null = null;
      for (const entry of stream) {
        const level = typeof entry.v === "number" ? entry.v : null;
        if (level === null) continue;
        if (minLevel === null || level < minLevel) {
          minLevel = level;
        }
        if (firstT === null) firstT = entry.t;
      }

      if (minLevel !== null && firstT !== null) {
        acceptObs.set(questID, { level: minLevel, t: firstT });
      }
    }

    // Emit observations from QuestAcceptLevel
    for (const [entityId, { level, t }] of acceptObs) {
      observations.push({
        entityId,
        value: level,
        confidence: "high",
        provenance: { session: sessionLabel(session), t },
      });
    }
  }

  // --- Fallback: UnitLevel at QUEST_ACCEPTED event time (for traces without QuestAcceptLevel) ---
  if (observations.length > 0) {
    // Already have high-confidence data from QuestAcceptLevel
    return observations;
  }

  // Find all QUEST_ACCEPTED events
  const acceptedQuests = new Map<number, { level: number; t: number }>();

  for (const event of session.events) {
    if (event.e !== "QUEST_ACCEPTED") continue;

    const questId = unpackQuestId(event.a);
    if (questId === null || questId <= 0) continue;

    const playerLevel = findPlayerLevelAtTime(session, event.t);
    if (playerLevel === null) continue;

    const current = acceptedQuests.get(questId);
    if (current === undefined || playerLevel < current.level) {
      acceptedQuests.set(questId, { level: playerLevel, t: event.t });
    }
  }

  for (const [entityId, { level, t }] of acceptedQuests) {
    observations.push({
      entityId,
      value: level,
      confidence: "medium", // correlated from UnitLevel at acceptance event time
      provenance: { session: sessionLabel(session), t },
    });
  }

  return observations;
};