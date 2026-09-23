// objectKeys.zoneID - Questie field 5.
// type: int, default: 0
// comment: "guess as to where this object is most common"
//
// Derived from the spawns observations: the zone ID that appears most frequently
// across all observed spawn points for this object.

import type { FieldObserver, Observation } from "../../observation";
import { observeSpawns, type SpawnObservationValue } from "./spawns";

export const observeZoneID: FieldObserver<SpawnObservationValue> = (session) => {
  return observeSpawns(session);
};
