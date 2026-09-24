// npcKeys.questStarts - Questie field 10.
// Inverse of quest.startedBy.creatures: for each NPC, which questIDs it starts.
// Built from the same QUEST_DETAIL/QUEST_ACCEPTED + UnitGUID("questnpc"/"npc") correlation
// as the quest startedBy observer, but keyed by npcID instead of questID.
//
// Custom merge: set-union of questIDs per npc.

import { getStream, valueAt } from "../../../core/emulator";
import { parseGuid } from "../../guid";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import type { SessionRecord } from "../../../core/types";

export function mergeQuestStarts(observations: Observation<number>[]): number[] {
  const questIds = new Set<number>();
  for (const obs of observations) {
    questIds.add(obs.value);
  }
  return [...questIds].sort((a, b) => a - b);
}

/** For each quest-start event, emit an observation keyed by the npcID that started it. */
export const observeQuestStarts: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];

  for (let i = 0; i < session.events.length; i++) {
    const ev = session.events[i];
    let questID: number | null = null;

    if (ev.e === "QUEST_DETAIL") {
      const v = valueAt(getStream(session, "GetQuestID") ?? [], ev.t);
      questID = typeof v === "number" && v !== 0 ? v : null;
    } else if (ev.e === "QUEST_ACCEPTED") {
      // QUEST_ACCEPTED has both one-argument (questID) and two-argument
      // (questID, ...) forms across supported clients.
      const v = ev.a.n === 1 ? ev.a[1] : ev.a[2];
      questID = typeof v === "number" && v !== 0 ? v : null;
    } else {
      continue;
    }
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
