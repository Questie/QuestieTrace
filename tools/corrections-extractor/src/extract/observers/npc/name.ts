// npcKeys.name - Questie field 1.
// observeName keeps enUS names only; observeLocalizedName keeps every locale (for the lookup export).

import { getLocaleAt, unitNameAt } from "../../probes";
import { onlyLocale, sessionLabel, type FieldObserver, type Localized, type Observation } from "../../observation";
import { npcEncounters } from "./_encounters";

export const observeLocalizedName: FieldObserver<Localized<string>> = (session) => {
  const observations: Observation<Localized<string>>[] = [];
  for (const encounter of npcEncounters(session)) {
    const locale = getLocaleAt(session, encounter.t);
    if (locale === null) continue;
    const name = unitNameAt(session, encounter.token, encounter.t);
    if (name) {
      observations.push({
        entityId: encounter.npcID,
        value: { locale, value: name },
        confidence: "high",
        provenance: { session: sessionLabel(session), t: encounter.t },
      });
    }
  }
  return observations;
};

export const observeName: FieldObserver<string> = (session) => onlyLocale(observeLocalizedName(session), "enUS");
