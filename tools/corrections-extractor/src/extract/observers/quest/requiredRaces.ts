// questKeys.requiredRaces - Questie field 6.
// type: bitmask, default: 0 (every race)
//
// Observes the player's faction for every quest a session saw. The race mask is merged in
// merge/quest/requiredRaces.ts.

import { getStream } from "../../../core/emulator";
import { playerFactionAt } from "../../probes";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import type { Faction } from "../npc/friendlyToFaction";
import { questEncounters } from "./_encounters";

interface QuestListEntry {
  questID?: number;
}

function acceptedQuestId(args: unknown): number | null {
  if (!args || typeof args !== "object") return null;
  const packed = args as Record<string, unknown>;
  const value = packed.n === 1 ? packed[1] : packed[2];
  return typeof value === "number" && value > 0 ? value : null;
}

/** The player's faction for every quest the session saw: accepted, in the quest log, in a quest frame or in a gossip list. */
export const observeQuestFactions: FieldObserver<Faction> = (session) => {
  const sightings: { questID: number; t: number }[] = questEncounters(session);
  for (const event of session.events) {
    if (event.e !== "QUEST_ACCEPTED") continue;
    const questID = acceptedQuestId(event.a);
    if (questID !== null) sightings.push({ questID, t: event.t });
  }
  for (const entry of getStream(session, "QuestLog") ?? []) {
    if (!Array.isArray(entry.v)) continue;
    for (const questID of entry.v) {
      if (typeof questID === "number" && questID > 0) sightings.push({ questID, t: entry.t });
    }
  }
  for (const name of ["C_GossipInfo.GetAvailableQuests", "C_GossipInfo.GetActiveQuests"]) {
    for (const entry of getStream(session, name) ?? []) {
      if (!Array.isArray(entry.v)) continue;
      for (const quest of entry.v as QuestListEntry[]) {
        if (typeof quest?.questID === "number" && quest.questID > 0) sightings.push({ questID: quest.questID, t: entry.t });
      }
    }
  }

  // A sighting before the first faction sample still belongs to the character's faction.
  // Counting it can only cancel a restriction, never cause one.
  const firstFaction = (getStream(session, "UnitFactionGroup", "player") ?? []).map((entry) => playerFactionAt(session, entry.t)).find((faction) => faction !== null) ?? null;
  const observations: Observation<Faction>[] = [];
  const seen = new Set<string>();
  for (const { questID, t } of sightings) {
    const faction = playerFactionAt(session, t) ?? firstFaction;
    if (!faction) continue;
    const key = `${questID}:${faction}`;
    if (seen.has(key)) continue;
    seen.add(key);
    observations.push({ entityId: questID, value: faction, confidence: "high", provenance: { session: sessionLabel(session), t } });
  }
  return observations;
};
