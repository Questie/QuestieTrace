// npcKeys.rank - Questie field 6.
// type: int, default: 0
// comment: "see https://github.com/cmangos/issues/wiki/creature_template#rank"
//
// Source: UnitClassification[token] from Modules/Trackers/UnitState.lua, paired
// with the NPC's GUID (see _unitState.ts).
//
// Merged in merge/npc/rank.ts.

import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import { unitStateEncounters } from "./_unitState";

const RANK_BY_CLASSIFICATION: Record<string, number> = {
  normal: 0,
  trivial: 0,
  minus: 0,
  elite: 1,
  rareelite: 2,
  worldboss: 3,
  rare: 4,
};

export const observeRank: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];
  const seen = new Set<string>();
  for (const encounter of unitStateEncounters(session, "UnitClassification")) {
    if (typeof encounter.value !== "string" || !Object.hasOwn(RANK_BY_CLASSIFICATION, encounter.value)) continue;
    const rank = RANK_BY_CLASSIFICATION[encounter.value];
    const key = `${encounter.npcID}:${rank}`;
    if (seen.has(key)) continue;
    seen.add(key);
    observations.push({
      entityId: encounter.npcID,
      value: rank,
      confidence: "high",
      provenance: { session: sessionLabel(session), t: encounter.t },
    });
  }
  return observations;
};
