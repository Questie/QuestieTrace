// npcKeys.minLevel - Questie field 4.
//
// A single trace only ever sees one level for a given npcID sighting, so this
// field and maxLevel.ts both observe the same UnitLevel(token) value; the real
// min/max range only emerges once `aggregate.ts` sees enough sessions.

import { unitLevelAt } from "../../probes";
import type { FieldObserver, Observation } from "../../observation";
import { npcEncounters } from "./_encounters";

export const observeMinLevel: FieldObserver<number> = (session) => {
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
