// npcKeys.questEnds - Questie field 11.
// Inverse of quest.finishedBy.creatures: for each NPC, which questIDs it finishes.
// Built from the same QUEST_COMPLETE/QUEST_TURNED_IN + UnitGUID("questnpc"/"npc") correlation
// as the quest finishedBy observer, but keyed by npcID instead of questID.
// Also includes C_GossipInfo.GetActiveQuests snapshots (Active = in log, can be turned in at this NPC).
//
// Custom merge: set-union of questIDs per npc.

import { getStream, valueAt } from "../../../core/emulator";
import { parseGuid } from "../../../core/guid";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import type { SessionRecord } from "../../../core/types";

export function mergeQuestEnds(observations: Observation<number>[]): number[] {
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
  const tokens = ["questnpc", "npc"] as const;

  for (const token of tokens) {
    const stream = getStream(session, "UnitGUID", token);
    if (!stream) continue;

    const guid = valueAt(stream, t);

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

/** For each quest-completion event, emit an observation keyed by the npcID that finished it. */
export const observeQuestEnds: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];

  // 1) Walk events looking for quest-completion events
  for (let i = 0; i < session.events.length; i++) {
    const ev = session.events[i];
    let questID: number | null = null;

    if (ev.e === "QUEST_COMPLETE") {
      const v = valueAt(getStream(session, "GetQuestID") ?? [], ev.t);
      questID = typeof v === "number" && v !== 0 ? v : null;
    } else if (ev.e === "QUEST_TURNED_IN") {
      const args = ev.a;
      if (args && typeof args.n === "number" && args.n >= 1) {
        const v = args[1];
        questID = typeof v === "number" && v !== 0 ? v : null;
      }
    } else {
      continue;
    }

    if (questID === null) continue;

    pushNpcObservations(session, questID, ev.t, observations);
  }

  // 2) Gossip ACTIVE quests = quests in log that can be FINISHED at this NPC
  // (not Available quests - those are for start, see questStarts)
  const activeStream = getStream(session, "C_GossipInfo.GetActiveQuests");
  if (activeStream) {
    for (const entry of activeStream) {
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
