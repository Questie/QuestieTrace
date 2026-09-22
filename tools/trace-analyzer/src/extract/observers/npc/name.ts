// npcKeys.name - Questie field 1.
// Only names from enUS locale are extracted. Other locales will be handled separately.

import { getLocaleAt, unitNameAt } from "../../probes";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import { npcEncounters } from "./_encounters";

export const observeName: FieldObserver<string> = (session) => {
  const observations: Observation<string>[] = [];
  for (const encounter of npcEncounters(session)) {
    const locale = getLocaleAt(session, encounter.t);
    if (locale !== "enUS") {
      continue; // Skip non-enUS names
    }
    const name = unitNameAt(session, encounter.token, encounter.t);
    if (name) {
      observations.push({
        entityId: encounter.npcID,
        value: name,
        confidence: "high",
        provenance: { session: sessionLabel(session), t: encounter.t },
      });
    }
  }
  return observations;
};
