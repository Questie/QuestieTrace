// questKeys.startedBy - Questie field 2.
// Builds { creatures = {npcIDs...}, objects = {objectIDs...}, items = {itemIDs...} }
// from quest-start events correlated with UnitGUID observations.
//
// Sources:
//   - QUEST_DETAIL event: questStartItemID (item starter) + GetQuestID() at same t (quest)
//   - QUEST_ACCEPTED event: questId from the event payload
//   - C_GossipInfo.GetAvailableQuests() snapshots: questID + UnitGUID("questnpc"/"npc") at snapshot t
//     (Available = can be accepted/started at this NPC)
//   - At those times, UnitGUID("questnpc") / UnitGUID("npc") → creature starter
//   - At same times, UnitGUID for GameObject tokens → object starter
//   - questnpc takes priority over npc (player may target something else while talking to quest NPC)
//   - "target" is intentionally excluded: unlike questnpc/npc, it can point at anything the player
//     happens to be targeting and is not tied to the quest interaction.
//
// Custom merge: set-union per starter type across all observations.

import { getStream, valueAt } from "../../../core/emulator";
import { parseGuid } from "../../../core/guid";
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

export interface StartedByStarter {
  creatureId?: number;
  objectId?: number;
  itemId?: number;
}

export interface StartedByValue {
  creatures: number[];
  objects: number[];
  items: number[];
}

export function mergeStartedBy(observations: Observation<StartedByStarter>[]): StartedByValue {
  const creatures = new Set<number>();
  const objects = new Set<number>();
  const items = new Set<number>();

  for (const obs of observations) {
    const s = obs.value;
    if (s.creatureId) creatures.add(s.creatureId);
    if (s.objectId) objects.add(s.objectId);
    if (s.itemId) items.add(s.itemId);
  }

  return {
    creatures: [...creatures].sort((a, b) => a - b),
    objects: [...objects].sort((a, b) => a - b),
    items: [...items].sort((a, b) => a - b),
  };
}

function pushGuidObservations(
  session: SessionRecord,
  questID: number,
  t: number,
  observations: Observation<StartedByStarter>[],
): void {
  const tokens = ["questnpc", "npc"] as const;

  for (const token of tokens) {
    const stream = getStream(session, "UnitGUID", token);
    if (!stream) continue;

    const guid = valueAt(stream, t);

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

export const observeStartedBy: FieldObserver<StartedByStarter> = (session) => {
  const observations: Observation<StartedByStarter>[] = [];

  // 1) Classic quest-start events (QUEST_DETAIL, QUEST_ACCEPTED)
  for (let i = 0; i < session.events.length; i++) {
    const ev = session.events[i];
    let questID: number | null = null;
    let starterItemID: number | null = null;

    if (ev.e === "QUEST_DETAIL") {
      const args = ev.a;
      if (args && typeof args.n === "number" && args.n >= 1) {
        starterItemID = typeof args[1] === "number" ? args[1] : null;
      }
      questID = questIdAt(session, ev.t);
    } else if (ev.e === "QUEST_ACCEPTED") {
      const v = ev.a.n === 1 ? ev.a[1] : ev.a[2];
      questID = typeof v === "number" && v !== 0 ? v : null;
    }

    if (questID === null) continue;

    pushGuidObservations(session, questID, ev.t, observations);

    if (starterItemID !== null && starterItemID > 0) {
      observations.push({
        entityId: questID,
        value: { itemId: starterItemID },
        confidence: "high",
        provenance: { session: sessionLabel(session), t: ev.t },
      });
    }
  }

  // 2) Gossip AVAILABLE quests = quests that can be STARTED (accepted) at this NPC
  // (not Active quests - those are for turn-in, see finishedBy)
  const availableStream = getStream(session, "C_GossipInfo.GetAvailableQuests");
  if (availableStream) {
    for (const entry of availableStream) {
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
