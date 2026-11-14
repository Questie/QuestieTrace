// questKeys.requiredLevel - Questie field 4.
//
// The player's level at each QUEST_ACCEPTED, read from the UnitLevel("player") stream. The
// lowest level across sessions wins.
//
// That stream only records changes, and UnitLevel("player") can still report the old level
// inside PLAYER_LEVEL_UP, so it shows a new level up to about a second late. A quest accepted
// in that gap would lower requiredLevel for good; the level PLAYER_LEVEL_UP announced is used
// when it is higher. Levels never drop within a session.
//
// Traces before schema 18 also hold a QuestAcceptLevel stream. It is ignored: on Forever it
// read QUEST_ACCEPTED's quest ID as a quest log index, recording levels against whichever
// quest sat at that position. Where it named the right quest, its level always equalled
// UnitLevel("player") at the event.

import { getStream, valueAt } from "../../../core/emulator";
import type { SessionRecord } from "../../../core/types";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

/** Era, TBC, Wrath and Mists send (questLogIndex, questID), Forever sends (questID) alone. */
function unpackQuestId(args: unknown): number | null {
  if (args !== null && args !== undefined && typeof args === "object" && !Array.isArray(args)) {
    const obj = args as Record<string, unknown>;
    const n = typeof obj.n === "number" ? obj.n : 0;
    if (n === 1) {
      const v = obj[1];
      return typeof v === "number" && v !== 0 ? v : null;
    }
    if (n === 2) {
      const v = obj[2];
      return typeof v === "number" && v !== 0 ? v : null;
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

export const observeRequiredLevel: FieldObserver<number> = (session) => {
  const acceptedQuests = new Map<number, { level: number; t: number }>();
  let levelUpLevel: number | null = null; // announced by the newest PLAYER_LEVEL_UP so far

  for (const event of session.events) {
    if (event.e === "PLAYER_LEVEL_UP") {
      const newLevel = event.a?.[1];
      if (typeof newLevel === "number" && Number.isFinite(newLevel)) levelUpLevel = newLevel;
      continue;
    }
    if (event.e !== "QUEST_ACCEPTED") continue;

    const questId = unpackQuestId(event.a);
    if (questId === null || questId <= 0) continue;

    const knownLevels = [findPlayerLevelAtTime(session, event.t), levelUpLevel].filter((level) => level !== null);
    if (knownLevels.length === 0) continue;
    const playerLevel = Math.max(...knownLevels);

    const current = acceptedQuests.get(questId);
    if (current === undefined || playerLevel < current.level) {
      acceptedQuests.set(questId, { level: playerLevel, t: event.t });
    }
  }

  const observations: Observation<number>[] = [];
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
