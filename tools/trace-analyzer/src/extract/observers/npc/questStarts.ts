// npcKeys.questStarts - Questie field 10.
// Inverse of quest.startedBy.creatures: for each NPC, which questIDs it starts.
// Built from the same QUEST_DETAIL/QUEST_ACCEPTED + UnitGUID("questnpc"/"npc") correlation
// as the quest startedBy observer, but keyed by npcID instead of questID.
// Also includes C_GossipInfo.GetAvailableQuests snapshots (Available = can be started at this NPC).
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

interface GossipQuestEntry {
  questID?: number;
}

function pushNpcObservations(
  session: SessionRecord,
  questID: number,
  t: number,
  observations: Observation<number>[],
): void {
  const questnpcGuid = valueAt(getStream(session, "UnitGUID", "questnpc") ?? [], t);
  const npcGuid = valueAt(getStream(session, "UnitGUID", "npc") ?? [], t);

  for (const guid of [questnpcGuid, npcGuid]) {
    if (typeof guid !== "string") continue;
    const parsed = parseGuid(guid);
    if (parsed?.kind === "npc" && parsed.id) {
      observations.push({
        entityId: parsed.id,
        value: questID,
        confidence: "medium",
        provenance: { session: sessionLabel(session), t },
      });
    }
  }
}

/** For each quest-start event, emit an observation keyed by the npcID that started it. */
export const observeQuestStarts: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];

  // 1) Classic quest-start events (QUEST_DETAIL, QUEST_ACCEPTED)
  for (let i = 0; i < session.events.length; i++) {
    const ev = session.events[i];
    let questID: number | null = null;

    if (ev.e === "QUEST_DETAIL") {
      const v = valueAt(getStream(session, "GetQuestID") ?? [], ev.t);
      questID = typeof v === "number" && v !== 0 ? v : null;
    } else if (ev.e === "QUEST_ACCEPTED") {
      const v = ev.a.n === 1 ? ev.a[1] : ev.a[2];
      questID = typeof v === "number" && v !== 0 ? v : null;
    } else {
      continue;
    }
    if (questID === null) continue;

    pushNpcObservations(session, questID, ev.t, observations);
  }

  // 2) Gossip AVAILABLE quests = quests that can be STARTED at this NPC
  // (not Active quests - those are for turn-in, see questEnds)
  const availableStream = getStream(session, "C_GossipInfo.GetAvailableQuests");
  if (availableStream) {
    for (const entry of availableStream) {
      const questList = entry.v as GossipQuestEntry[] | undefined;
      if (!Array.isArray(questList)) continue;

      for (const q of questList) {
        const questID = q.questID;
        if (typeof questID !== "number" || questID === 0) continue;

        pushNpcObservations(session, questID, entry.t, observations);
      }
    }
  }

  return observations;
};
