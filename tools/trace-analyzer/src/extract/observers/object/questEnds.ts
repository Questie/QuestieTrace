// objectKeys.questEnds - Questie field 3.
// Inverse of quest.finishedBy.objects: for each object, which questIDs it finishes.
// Built from QUEST_COMPLETE + GameObject GUID correlation.
//
// Custom merge: set-union of questIDs per object.

import { getStream, valueAt } from "../../../core/emulator";
import { parseGuid } from "../../guid";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

export function mergeQuestEnds(observations: Observation<number>[]): number[] {
  const questIds = new Set<number>();
  for (const obs of observations) {
    questIds.add(obs.value);
  }
  return [...questIds].sort((a, b) => a - b);
}

/** For each quest-completion event, emit an observation keyed by the objectID that finished it. */
export const observeQuestEnds: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];

  for (let i = 0; i < session.events.length; i++) {
    const ev = session.events[i];

    if (ev.e !== "QUEST_COMPLETE") continue;

    const v = valueAt(getStream(session, "GetQuestID") ?? [], ev.t);
    const questID = typeof v === "number" && v !== 0 ? v : null;
    if (questID === null) continue;

    // A token's last value can be stale. Require an event-synchronous GUID sample
    // so only objects observed during QUEST_COMPLETE can be identified as finishers.
    for (const token of ["target", "npc"] as const) {
      const stream = getStream(session, "UnitGUID", token) ?? [];
      for (const entry of stream) {
        if (entry.t !== ev.t || typeof entry.v !== "string") continue;
        const parsed = parseGuid(entry.v);
        if (parsed?.kind === "object" && parsed.id) {
          observations.push({
            entityId: parsed.id,
            value: questID,
            confidence: "low",
            provenance: { session: sessionLabel(session), t: ev.t },
          });
        }
      }
    }
  }

  return observations;
};
