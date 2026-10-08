// objectKeys.questStarts - Questie field 2.
// Inverse of quest.startedBy.objects: for each object, which questIDs it starts.
// Built from QUEST_DETAIL/QUEST_ACCEPTED + GameObject GUID correlation.
//
// Custom merge: set-union of questIDs per object.

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

/** For each quest-start event, emit an observation keyed by the objectID that started it. */
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
