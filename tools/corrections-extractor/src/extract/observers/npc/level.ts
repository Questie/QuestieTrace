// npcKeys.minLevel / npcKeys.maxLevel - Questie fields 4 and 5.
//
// Source: UnitLevel[token] from Modules/Trackers/UnitState.lua, paired with the
// NPC's GUID (see _unitState.ts). Levels below 1 (-1 is "??", e.g. skull bosses) are skipped.
//
// Merged in merge/npc/level.ts.

import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import { unitStateEncounters } from "./_unitState";

export const observeLevel: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];
  const seen = new Set<string>();
  for (const encounter of unitStateEncounters(session, "UnitLevel")) {
    const level = encounter.value;
    if (typeof level !== "number" || level < 1) continue;
    const key = `${encounter.npcID}:${level}`;
    if (seen.has(key)) continue;
    seen.add(key);
    observations.push({
      entityId: encounter.npcID,
      value: level,
      confidence: "high",
      provenance: { session: sessionLabel(session), t: encounter.t },
    });
  }
  return observations;
};
