// npcKeys.zoneID - Questie field 9.
// type: int, default: 0
// comment: "guess as to where this NPC is most common"
//
// Derived from the spawns observations: the zone ID that appears most frequently
// across all observed spawn points for this NPC. Falls back to the schema default
// (0) when no spawn data exists.
//
// Uses a custom merge that tallies zone frequency and picks the most common one.

import type { FieldObserver, Observation } from "../../observation";
import { observeSpawns, type SpawnObservationValue } from "./spawns";

export const observeZoneID: FieldObserver<SpawnObservationValue> = (session) => {
  // Delegate to observeSpawns - we consume the same underlying data but at
  // aggregate time we derive zoneID from the frequency distribution.
  return observeSpawns(session);
};
