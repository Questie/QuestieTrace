// questKeys.finishedBy - Questie field 3.
// Builds { creatures = {npcIDs...}, objects = {objectIDs...} }
// from quest-completion events correlated with UnitGUID observations.
// Note: NO items field (quests are not completed via items).
//
// Sources:
//   - QUEST_COMPLETE event: questId + GetQuestID() at same t (quest)
//   - QUEST_TURNED_IN event: questId from event payload (arg 1)
//   - GOSSIP_SHOW: replay the last observed C_GossipInfo.GetActiveQuests() list
//     against the current questnpc/npc (Active = in log, can be turned in here).
//   - At those times, UnitGUID("questnpc") / UnitGUID("npc") → finisher
//   - Use questnpc when it identifies a giver; npc is only a fallback, not a second giver.
//
// Custom merge: set-union per finisher type across all observations.

import { AMBIGUOUS_INTERACTION, questEventLookup, type InteractionLookup } from "./_interaction";
import { parseGuid } from "../../guid";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import type { SessionRecord } from "../../../core/types";

function questIdAt(lookup: InteractionLookup): number | null {
  const v = lookup("GetQuestID");
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
  lookup: InteractionLookup,
  observations: Observation<FinishedByFinisher>[],
): void {
  const tokens = ["questnpc", "npc"] as const;

  for (const token of tokens) {
    const guid = lookup("UnitGUID", token);
    if (guid === AMBIGUOUS_INTERACTION) return;

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
      return;
    } else if (parsed.kind === "object" && parsed.id) {
      observations.push({
        entityId: questID,
        value: { objectId: parsed.id },
        confidence: "low",
        provenance: { session: sessionLabel(session), t },
      });
      return;
    }
  }
}

export const observeFinishedBy: FieldObserver<FinishedByFinisher> = (session) => {
  const observations: Observation<FinishedByFinisher>[] = [];

  for (let i = 0; i < session.events.length; i++) {
    const ev = session.events[i];
    if (ev.e !== "QUEST_COMPLETE" && ev.e !== "QUEST_TURNED_IN" && ev.e !== "GOSSIP_SHOW") continue;
    const lookup = questEventLookup(session, i);
    if (!lookup) continue;

    // Reuse the last observed list at each interaction, not just when its contents change.
    // Missing samples retain last-observed state; failed probes cannot be distinguished.
    if (ev.e === "GOSSIP_SHOW") {
      const quests = lookup("C_GossipInfo.GetActiveQuests") as GossipQuestEntry[] | undefined;
      if (Array.isArray(quests)) {
        for (const quest of quests) {
          if (typeof quest?.questID === "number" && quest.questID > 0) {
            pushGuidObservations(session, quest.questID, ev.t, lookup, observations);
          }
        }
      }
      continue;
    }

    let questID: number | null = null;

    if (ev.e === "QUEST_COMPLETE") {
      questID = questIdAt(lookup);
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

    pushGuidObservations(session, questID, ev.t, lookup, observations);
  }

  return observations;
};
