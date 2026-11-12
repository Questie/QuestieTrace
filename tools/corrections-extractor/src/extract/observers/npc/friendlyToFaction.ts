// npcKeys.friendlyToFaction - Questie field 13.
// type: string, default: nil
// comment: 'Contains "A" and/or "H" depending on NPC being friendly towards those factions. nil if hostile to both.'
//
// Questie treats neutral NPCs (e.g. wildlife) as friendly, so a faction counts as
// friendly when UnitReaction("player", npc) is 4 (neutral) or higher.
//
// Source: UnitReaction["player"][token] from Modules/Trackers/UnitState.lua, paired
// with the NPC's GUID (see _unitState.ts). A wrong pairing is possible; the strict
// merge in merge/npc/friendlyToFaction.ts is what guards against that.

import { playerFactionAt, sessionServerTime } from "../../probes";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import { unitStateEncounters } from "./_unitState";

export type Faction = "A" | "H";

export interface FactionReactionValue {
  faction: Faction;
  friendly: boolean;
  /** GetServerTime() at the start of the session, used to order sessions; null when not recorded. */
  sessionTime: number | null;
}

const FRIENDLY_REACTION_MIN = 4;

export const observeFriendlyToFaction: FieldObserver<FactionReactionValue> = (session) => {
  const observations: Observation<FactionReactionValue>[] = [];
  const seen = new Set<string>();
  const sessionTime = sessionServerTime(session);
  for (const encounter of unitStateEncounters(session, "UnitReaction", "player")) {
    const faction = playerFactionAt(session, encounter.t);
    if (!faction) continue;
    if (typeof encounter.value !== "number") continue;
    const friendly = encounter.value >= FRIENDLY_REACTION_MIN;
    const key = `${encounter.npcID}:${faction}:${friendly}`;
    if (seen.has(key)) continue;
    seen.add(key);
    observations.push({
      entityId: encounter.npcID,
      value: { faction, friendly, sessionTime },
      confidence: "high",
      provenance: { session: sessionLabel(session), t: encounter.t },
    });
  }
  return observations;
};
