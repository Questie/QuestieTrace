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

    // Check target token (common for objects), then npc token
    const targetGuid = valueAt(getStream(session, "UnitGUID", "target") ?? [], ev.t);
    const npcGuid = valueAt(getStream(session, "UnitGUID", "npc") ?? [], ev.t);

    for (const guid of [targetGuid, npcGuid]) {
      if (typeof guid !== "string") continue;
      const parsed = parseGuid(guid);
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

  return observations;
};
