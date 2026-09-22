// questKeys.name - Questie field 1.
// Only names from enUS locale are extracted. Other locales will be handled separately.

import { getLocaleAt, questTitleAt } from "../../probes";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import { questEncounters } from "./_encounters";

export const observeName: FieldObserver<string> = (session) => {
  const observations: Observation<string>[] = [];
  for (const encounter of questEncounters(session)) {
    const locale = getLocaleAt(session, encounter.t);
    if (locale !== "enUS") {
      continue; // Skip non-enUS names
    }
    const title = questTitleAt(session, encounter.t);
    if (title) {
      observations.push({
        entityId: encounter.questID,
        value: title,
        confidence: "high",
        provenance: { session: sessionLabel(session), t: encounter.t },
      });
    }
  }
  return observations;
};
