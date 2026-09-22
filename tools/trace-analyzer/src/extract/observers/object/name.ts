// objectKeys.name - Questie field 1. Same shape as observers/npc/name.ts.
// Only names from enUS locale are extracted. Other locales will be handled separately.

import { getLocaleAt, unitNameAt } from "../../probes";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import { objectEncounters } from "./_encounters";

export const observeName: FieldObserver<string> = (session) => {
  const observations: Observation<string>[] = [];
  for (const encounter of objectEncounters(session)) {
    const locale = getLocaleAt(session, encounter.t);
    if (locale !== "enUS") {
      continue; // Skip non-enUS names
    }
    const name = unitNameAt(session, encounter.token, encounter.t);
    if (name) {
      observations.push({
        entityId: encounter.objectID,
        value: name,
        confidence: "high",
        provenance: { session: sessionLabel(session), t: encounter.t },
      });
    }
  }
  return observations;
};
