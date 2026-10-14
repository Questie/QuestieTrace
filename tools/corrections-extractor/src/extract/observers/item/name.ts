// itemKeys.name - Questie field 1.
//
// Unlike npc/object, item id + name both come from the same signal - a
// `GetLootSlotLink(slot)` item hyperlink (see Modules/Trackers/Loot.lua) - so no
// GUID parsing or time correlation with a separate name stream is needed.
// observeName keeps enUS names only; observeLocalizedName keeps every locale (for the lookup export).

import { getParamKeys, getStream } from "../../../core/emulator";
import type { SessionRecord } from "../../../core/types";
import { getLocaleAt } from "../../probes";
import { parseItemLink } from "../../itemLink";
import { onlyLocale, sessionLabel, type FieldObserver, type Localized, type Observation } from "../../observation";

export const observeLocalizedName: FieldObserver<Localized<string>> = (session) => {
  const observations: Observation<Localized<string>>[] = [];
  const linkStream = session.functions["GetLootSlotLink"];
  if (!linkStream) return observations;

  for (const slot of getParamKeys(linkStream)) {
    const entries = getStream(session, "GetLootSlotLink", slot);
    if (!entries) continue;
    for (const entry of entries) {
      if (typeof entry.v !== "string") continue;
      const locale = getLocaleAt(session, entry.t);
      if (locale === null) continue;
      const parsed = parseItemLink(entry.v);
      if (!parsed) continue;
      observations.push({
        entityId: parsed.itemID,
        value: { locale, value: parsed.name },
        confidence: "high",
        provenance: { session: sessionLabel(session), t: entry.t },
      });
    }
  }
  return observations;
};

export const observeName: FieldObserver<string> = (session) => onlyLocale(observeLocalizedName(session), "enUS");
