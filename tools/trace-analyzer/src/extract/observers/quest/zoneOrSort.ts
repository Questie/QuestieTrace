// questKeys.zoneOrSort - Questie field 17.
//
// QuestLogZone records the human-readable quest-log header immediately before
// each active quest. The header is an AreaTable name for zone groups and a
// QuestSort name for special groups. Convert that title back to the signed DB2
// ID expected by Questie.

import { getStream, valueAt } from "../../../core/emulator";
import { zoneOrSortIdForName } from "../../resources/zone-or-sort";
import { questEncounters } from "./_encounters";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

export const observeZoneOrSort: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];

  for (const encounter of questEncounters(session)) {
    const zoneStream = getStream(session, "QuestLogZone", encounter.questID);
    const headerTitle = valueAt(zoneStream ?? [], encounter.t);
    const zoneOrSort = zoneOrSortIdForName(headerTitle);
    if (zoneOrSort === null) continue;

    observations.push({
      entityId: encounter.questID,
      value: zoneOrSort,
      // The quest-log association is observed, but the ID is resolved through
      // the bundled DB2 name lookup rather than returned directly by WoW.
      confidence: "medium",
      provenance: { session: sessionLabel(session), t: encounter.t },
    });
  }

  return observations;
};
