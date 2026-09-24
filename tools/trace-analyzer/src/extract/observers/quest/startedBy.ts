// questKeys.startedBy - Questie field 2.
// Builds { creatures = {npcIDs...}, objects = {objectIDs...}, items = {itemIDs...} }
// from quest-start events correlated with UnitGUID observations.
//
// Sources:
//   - QUEST_DETAIL event: questStartItemID (item starter) + GetQuestID() at same t (quest)
//   - QUEST_ACCEPTED event: questId from the event payload
//   - At those times, UnitGUID("questnpc") / UnitGUID("npc") → creature starter
//   - At same times, UnitGUID for GameObject tokens → object starter
//
// Custom merge: set-union per starter type across all observations.

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



export const observeStartedBy: FieldObserver<StartedByStarter> = (session) => {
  const observations: Observation<StartedByStarter>[] = [];

  // Walk events looking for quest-start events
  for (let i = 0; i < session.events.length; i++) {
    const ev = session.events[i];
    let questID: number | null = null;
    let starterItemID: number | null = null;

    if (ev.e === "QUEST_DETAIL") {
      // QUEST_DETAIL: questStartItemID (arg 1)
      const args = ev.a;
      if (args && typeof args.n === "number" && args.n >= 1) {
        starterItemID = typeof args[1] === "number" ? args[1] : null;
      }
      // Get the questID from GetQuestID() at this time
      questID = questIdAt(session, ev.t);
    } else if (ev.e === "QUEST_ACCEPTED") {
      const v = ev.a.n === 1 ? ev.a[1] : ev.a[2];
      questID = typeof v === "number" && v !== 0 ? v : null;
    }

    if (questID === null) continue;

    // At the event time, check what NPC/Object GUIDs were active
    const t = ev.t;

    // Check questnpc token first, then npc token
    const questnpcGuid = valueAt(getStream(session, "UnitGUID", "questnpc") ?? [], t);
    const npcGuid = valueAt(getStream(session, "UnitGUID", "npc") ?? [], t);
    const targetGuid = valueAt(getStream(session, "UnitGUID", "target") ?? [], t);

    // Also check GameObject tokens - use the same GUID streams
    // GameObjects appear on UnitGUID("target") or UnitGUID("npc") depending on context
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

    // Quest start item from QUEST_DETAIL event
    if (starterItemID !== null && starterItemID > 0) {
      observations.push({
        entityId: questID,
        value: { itemId: starterItemID },
        confidence: "high",
        provenance: { session: sessionLabel(session), t: ev.t },
      });
    }
  }

  return observations;
};

