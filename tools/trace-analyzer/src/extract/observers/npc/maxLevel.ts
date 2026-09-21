// npcKeys.maxLevel - Questie field 5. See minLevel.ts for why this mirrors it.

import { unitLevelAt } from "../../probes";
import type { FieldObserver, Observation } from "../../observation";
import { npcEncounters } from "./_encounters";

export const observeMaxLevel: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];
  for (const encounter of npcEncounters(session)) {
    const level = unitLevelAt(session, encounter.token, encounter.t);
    if (level !== null) {
      observations.push({
        entityId: encounter.npcID,
        value: level,
        confidence: "medium",
        provenance: { session: session.name, t: encounter.t },
      });
    }
  }
  return observations;
};
