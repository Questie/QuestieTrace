// questKeys.finishedBy - Questie field 3.
// Builds { creatures = {npcIDs...}, objects = {objectIDs...} }
// from quest-completion events correlated with UnitGUID observations.
// Note: NO items field (quests are not completed via items).
//
// Sources:
//   - QUEST_COMPLETE event: questId + GetQuestID() at same t (quest)
//   - At that time, UnitGUID("questnpc") / UnitGUID("npc") / UnitGUID("target") → finisher
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

export const observeFinishedBy: FieldObserver<FinishedByFinisher> = (session) => {
  const observations: Observation<FinishedByFinisher>[] = [];

  // Walk events looking for quest-completion events
  for (let i = 0; i < session.events.length; i++) {
    const ev = session.events[i];

    if (ev.e !== "QUEST_COMPLETE") continue;

    // Get the questID from GetQuestID() at this time
    const questID = questIdAt(session, ev.t);
    if (questID === null) continue;

    // At the event time, check what NPC/Object GUIDs were active
    const t = ev.t;

    // Check questnpc token first, then npc token, then target
    const questnpcGuid = valueAt(getStream(session, "UnitGUID", "questnpc") ?? [], t);
    const npcGuid = valueAt(getStream(session, "UnitGUID", "npc") ?? [], t);
    const targetGuid = valueAt(getStream(session, "UnitGUID", "target") ?? [], t);

    const guidsToCheck = [questnpcGuid, npcGuid, targetGuid].filter(Boolean);

    for (const guid of guidsToCheck) {
      if (typeof guid !== "string") continue;
      // Parse the GUID to extract entity ID and kind
      const parsed = parseGuid(guid);
      if (!parsed || !parsed.id) continue;

      if (parsed.kind === "npc" && parsed.id) {
        observations.push({
          entityId: questID,
          value: { creatureId: parsed.id },
          confidence: "medium",
          provenance: { session: sessionLabel(session), t: ev.t },
        });
      } else if (parsed.kind === "object" && parsed.id) {
        observations.push({
          entityId: questID,
          value: { objectId: parsed.id },
          confidence: "low",
          provenance: { session: sessionLabel(session), t: ev.t },
        });
      }
    }
  }

  return observations;
};
