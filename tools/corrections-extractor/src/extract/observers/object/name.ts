// objectKeys.name - Questie field 1. Same shape as observers/npc/name.ts.
// observeName keeps enUS names only; observeLocalizedName keeps every locale (for the lookup export).
//
// Looted objects are rarely targeted, so their names come from the object tooltip instead
// (GameObjectName, Modules/Trackers/GameObject.lua): an object has to be hovered to be looted,
// so the last tooltip before a loot names the looted object. The tooltip has no GUID.

import { getStream, valueAt } from "../../../core/emulator";
import { getLocaleAt, unitNameAt } from "../../probes";
import { onlyLocale, sessionLabel, type FieldObserver, type Localized, type Observation } from "../../observation";
import { objectEncounters } from "./_encounters";
import { objectLoots } from "./_loots";

export const observeLocalizedName: FieldObserver<Localized<string>> = (session) => {
  const observations: Observation<Localized<string>>[] = [];
  const observe = (objectID: number, t: number, name: string | null) => {
    const locale = getLocaleAt(session, t);
    if (locale === null || !name) return;
    observations.push({
      entityId: objectID,
      value: { locale, value: name },
      confidence: "high",
      provenance: { session: sessionLabel(session), t },
    });
  };

  for (const encounter of objectEncounters(session)) {
    observe(encounter.objectID, encounter.t, unitNameAt(session, encounter.token, encounter.t));
  }
  const tooltips = getStream(session, "GameObjectName");
  if (tooltips) {
    for (const loot of objectLoots(session)) {
      const name = valueAt(tooltips, loot.t);
      observe(loot.objectID, loot.t, typeof name === "string" ? name : null);
    }
  }
  return observations;
};

export const observeName: FieldObserver<string> = (session) => onlyLocale(observeLocalizedName(session), "enUS");
