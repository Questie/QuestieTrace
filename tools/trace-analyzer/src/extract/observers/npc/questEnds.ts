// npcKeys.questEnds - Questie field 11.
// Inverse of quest.finishedBy.creatures: for each NPC, which questIDs it finishes.
// Built from the same QUEST_COMPLETE + UnitGUID("questnpc"/"npc") correlation
// as the quest finishedBy observer, but keyed by npcID instead of questID.
//
// Custom merge: set-union of questIDs per npc.

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

/** For each quest-completion event, emit an observation keyed by the npcID that finished it. */
export const observeQuestEnds: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];

  for (let i = 0; i < session.events.length; i++) {
    const ev = session.events[i];

    if (ev.e !== "QUEST_COMPLETE") continue;

    const v = valueAt(getStream(session, "GetQuestID") ?? [], ev.t);
    const questID = typeof v === "number" && v !== 0 ? v : null;
    if (questID === null) continue;

    // Check questnpc token first, then npc token
    const questnpcGuid = valueAt(getStream(session, "UnitGUID", "questnpc") ?? [], ev.t);
    const npcGuid = valueAt(getStream(session, "UnitGUID", "npc") ?? [], ev.t);

    for (const guid of [questnpcGuid, npcGuid]) {
      if (typeof guid !== "string") continue;
      const parsed = parseGuid(guid);
      if (parsed?.kind === "npc" && parsed.id) {
        observations.push({
          entityId: parsed.id,
          value: questID,
          confidence: "medium",
          provenance: { session: sessionLabel(session), t: ev.t },
        });
      }
    }
  }

  return observations;
};
