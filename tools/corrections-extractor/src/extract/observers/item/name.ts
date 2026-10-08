// itemKeys.name - Questie field 1.
//
// Unlike npc/object, item id + name both come from the same signal - a
// `GetLootSlotLink(slot)` item hyperlink (see Modules/Trackers/Loot.lua) - so no
// GUID parsing or time correlation with a separate name stream is needed.
// Only names from enUS locale are extracted. Other locales will be handled separately.

import { getParamKeys, getStream } from "../../../core/emulator";
import type { SessionRecord } from "../../../core/types";
import { getLocaleAt } from "../../probes";
import { parseItemLink } from "../../itemLink";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

export const observeName: FieldObserver<string> = (session) => {
  const observations: Observation<string>[] = [];
  const linkStream = session.functions["GetLootSlotLink"];
  if (!linkStream) return observations;

  for (const slot of getParamKeys(linkStream)) {
    const entries = getStream(session, "GetLootSlotLink", slot);
    if (!entries) continue;
    for (const entry of entries) {
      if (typeof entry.v !== "string") continue;
      const locale = getLocaleAt(session, entry.t);
      if (locale !== "enUS") {
        continue; // Skip non-enUS names
      }
      const parsed = parseItemLink(entry.v);
      if (!parsed) continue;
      observations.push({
        entityId: parsed.itemID,
        value: parsed.name,
        confidence: "high",
        provenance: { session: sessionLabel(session), t: entry.t },
      });
    }
  }
  return observations;
};
