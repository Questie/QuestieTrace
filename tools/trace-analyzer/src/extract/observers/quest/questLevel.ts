// questKeys.questLevel - Questie field 5.
// Read from two possible streams, both keyed by questID:
//   - GetQuestLogTitle(questId) tuple position 2 (the "level" field)
//   - C_QuestLog.GetInfo(questId).level (modern API, table return)
// The questID parameter is the same questID from the encounter anchor.

import { emulate, getStream, valueAt } from "../../../core/emulator";
import { questEncounters } from "./_encounters";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

function readLevelFromGetQuestLogTitle(session: any, questID: number, t: number): number | null {
  const stream = getStream(session, "GetQuestLogTitle", questID);
  if (!stream || stream.length === 0) return null;

  const entry = stream[stream.length - 1];
  const tuple = emulate(entry.v);
  if (!Array.isArray(tuple) || tuple.length < 2) return null;

  const level = tuple[1]; // position 2 (0-indexed: 1) = level
  return typeof level === "number" && Number.isFinite(level) ? level : null;
}

function readLevelFromGetInfo(session: any, questID: number, t: number): number | null {
  const stream = getStream(session, "C_QuestLog.GetInfo", questID);
  if (!stream || stream.length === 0) return null;

  // valueAt gives us the most recent value at or before time t
  const v = valueAt(stream, t);
  if (!v || typeof v !== "object" || Array.isArray(v)) return null;

  const level = (v as Record<string, unknown>).level;
  return typeof level === "number" && Number.isFinite(level) ? level : null;
}

export const observeQuestLevel: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];
  for (const encounter of questEncounters(session)) {
    // Try C_QuestLog.GetInfo first (modern API), fall back to GetQuestLogTitle
    let questLevel = readLevelFromGetInfo(session, encounter.questID, encounter.t);
    if (questLevel === null) {
      questLevel = readLevelFromGetQuestLogTitle(session, encounter.questID, encounter.t);
    }
    if (questLevel === null) continue;

    observations.push({
      entityId: encounter.questID,
      value: questLevel,
      confidence: "high",
      provenance: { session: sessionLabel(session), t: encounter.t },
    });
  }
  return observations;
};
