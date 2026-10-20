// questKeys.startedBy - Questie field 2.
// Builds { creatures = {npcIDs...}, objects = {objectIDs...}, items = {itemIDs...} }
// from quest-start events correlated with UnitGUID observations.
//
// Sources:
//   - QUEST_DETAIL event: questStartItemID (item starter) + GetQuestID() at same t (quest)
//   - QUEST_ACCEPTED event: questId from the event payload
//   - GOSSIP_SHOW: replay the last observed C_GossipInfo.GetAvailableQuests() list
//     against the current questnpc/npc (Available = can be accepted/started here).
//   - At those times, UnitGUID("questnpc") / UnitGUID("npc") → creature starter
//   - At same times, UnitGUID for GameObject tokens → object starter
//   - Use questnpc when it identifies a giver; npc is only a fallback, not a second giver.
//   - Object starters always show up on questnpc. An object on npc alone is whatever the player
//     is using (e.g. a Mailbox while a party member shares a quest), so it never falls back.
//   - "target" is intentionally excluded: unlike questnpc/npc, it can point at anything the player
//     happens to be targeting and is not tied to the quest interaction.
//
// Custom merge: set-union per starter type across all observations.

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
  lookup: InteractionLookup,
  observations: Observation<StartedByStarter>[],
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
    } else if (parsed.kind === "object" && parsed.id && token === "questnpc") {
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

export const observeStartedBy: FieldObserver<StartedByStarter> = (session) => {
  const observations: Observation<StartedByStarter>[] = [];

  for (let i = 0; i < session.events.length; i++) {
    const ev = session.events[i];
    if (ev.e !== "QUEST_DETAIL" && ev.e !== "QUEST_ACCEPTED" && ev.e !== "GOSSIP_SHOW") continue;
    const lookup = questEventLookup(session, i);
    if (!lookup) continue;

    // Lists are change-only: each GOSSIP_SHOW can expose the same quests at a different NPC.
    // As with GUID replay, a failed probe is indistinguishable from an unchanged value.
    if (ev.e === "GOSSIP_SHOW") {
      const quests = lookup("C_GossipInfo.GetAvailableQuests") as GossipQuestEntry[] | undefined;
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
    let starterItemID: number | null = null;

    if (ev.e === "QUEST_DETAIL") {
      const args = ev.a;
      if (args && typeof args.n === "number" && args.n >= 1) {
        starterItemID = typeof args[1] === "number" ? args[1] : null;
      }
      questID = questIdAt(lookup);
    } else if (ev.e === "QUEST_ACCEPTED") {
      const v = ev.a.n === 1 ? ev.a[1] : ev.a[2];
      questID = typeof v === "number" && v !== 0 ? v : null;
    }

    if (questID === null) continue;

    pushGuidObservations(session, questID, ev.t, lookup, observations);

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
