// questKeys.finishedBy - Questie field 3.
// Builds { creatures = {npcIDs...}, objects = {objectIDs...} }
// from quest-completion events correlated with UnitGUID observations.
// Note: NO items field (quests are not completed via items).
//
// Sources:
//   - QUEST_COMPLETE event: questId + GetQuestID() at same t (quest)
//   - QUEST_TURNED_IN event: questId from event payload (arg 1)
//   - C_GossipInfo.GetActiveQuests() snapshots: questID + UnitGUID("questnpc"/"npc"/"target") at snapshot t
//     (Active = in log, can be turned in/finished at this NPC)
//   - At those times, UnitGUID("questnpc") / UnitGUID("npc") / UnitGUID("target") → finisher
//
// Custom merge: set-union per finisher type across all observations.

import { getStream, valueAt } from "../../../core/emulator";
import { parseGuid } from "../../guid";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import type { SessionRecord } from "../../../core/types";

function questIdAt(session: SessionRecord, t: number): number | null {
  const stream = getStream(session, "GetQuestID");
  if (!stream) return null;
  const v = valueAt(stream, t);
  return typeof v === "number" && v !== 0 ? v : null;
}

interface GossipQuestEntry {
  questID?: number;
}

export interface FinishedByFinisher {
  creatureId?: number;
  objectId?: number;
}

export interface FinishedByValue {
  creatures: number[];
  objects: number[];
}

export function mergeFinishedBy(observations: Observation<FinishedByFinisher>[]): FinishedByValue {
  const creatures = new Set<number>();
  const objects = new Set<number>();

  for (const obs of observations) {
    const f = obs.value;
    if (f.creatureId) creatures.add(f.creatureId);
    if (f.objectId) objects.add(f.objectId);
  }

  return {
    creatures: [...creatures].sort((a, b) => a - b),
    objects: [...objects].sort((a, b) => a - b),
  };
}

function pushGuidObservations(
  session: SessionRecord,
  questID: number,
  t: number,
  observations: Observation<FinishedByFinisher>[],
): void {
  const questnpcGuid = valueAt(getStream(session, "UnitGUID", "questnpc") ?? [], t);
  const npcGuid = valueAt(getStream(session, "UnitGUID", "npc") ?? [], t);
  const targetGuid = valueAt(getStream(session, "UnitGUID", "target") ?? [], t);

  const guidsToCheck = [questnpcGuid, npcGuid, targetGuid].filter(Boolean);

  for (const guid of guidsToCheck) {
    if (typeof guid !== "string") continue;
    const parsed = parseGuid(guid);
    if (!parsed || !parsed.id) continue;

    if (parsed.kind === "npc" && parsed.id) {
      observations.push({
        entityId: questID,
        value: { creatureId: parsed.id },
        confidence: "medium",
        provenance: { session: sessionLabel(session), t },
      });
    } else if (parsed.kind === "object" && parsed.id) {
      observations.push({
        entityId: questID,
        value: { objectId: parsed.id },
        confidence: "low",
        provenance: { session: sessionLabel(session), t },
      });
    }
  }
}

export const observeFinishedBy: FieldObserver<FinishedByFinisher> = (session) => {
  const observations: Observation<FinishedByFinisher>[] = [];

  // 1) Walk events looking for quest-completion events
  for (let i = 0; i < session.events.length; i++) {
    const ev = session.events[i];
    let questID: number | null = null;

    if (ev.e === "QUEST_COMPLETE") {
      questID = questIdAt(session, ev.t);
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

    pushGuidObservations(session, questID, ev.t, observations);
  }

  // 2) Gossip ACTIVE quests = quests in log that can be FINISHED (turned in) at this NPC
  // (not Available quests - those are for start, see startedBy)
  const activeStream = getStream(session, "C_GossipInfo.GetActiveQuests");
  if (activeStream) {
    for (const entry of activeStream) {
      const questList = entry.v as GossipQuestEntry[] | undefined;
      if (!Array.isArray(questList)) continue;

      for (const q of questList) {
        const questID = q.questID;
        if (typeof questID !== "number" || questID === 0) continue;

        pushGuidObservations(session, questID, entry.t, observations);
      }
    }
  }

  return observations;
};
