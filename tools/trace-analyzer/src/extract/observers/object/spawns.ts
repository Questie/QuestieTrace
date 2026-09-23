// objectKeys.spawns - Questie field 4.
// type: table {[zoneID(int)] = {coordPair(floatVector2D),...},...}
//
// Observed by correlating object encounters with the player's map position at the
// same time. The `target` token is the primary source for objects (when the player
// targets/interacts with a GameObject like a chest or mine node).
//
// Output shape: Observation<SpawnObservationValue> where each observation
// represents one spawn point seen at a specific zone/coordinate, keyed by objectID.
// The custom merge (mergeSpawns) accumulates these into the final
// {[zoneID]: {{x,y},...},...} table.

import type { SessionRecord } from "../../../core/types";
import type { FieldObserver, Observation } from "../../observation";
import { objectEncounters } from "./_encounters";
import { playerZoneAt, playerPosAt } from "../../probes";

export interface SpawnObservationValue {
  zoneID: number;
  x: number;
  y: number;
}

/** Tagged observation that carries the unit token for priority sorting. */
export interface TaggedSpawnObservation extends Observation<SpawnObservationValue> {
  token: string;
}

export const observeSpawns: FieldObserver<SpawnObservationValue> = (session) => {
  const observations: TaggedSpawnObservation[] = [];
  const encounters = objectEncounters(session);

  for (const encounter of encounters) {
    const zoneID = playerZoneAt(session, encounter.t);
    if (zoneID === null) continue;

    const pos = playerPosAt(session, encounter.t);
    if (pos === null) continue;

    observations.push({
      entityId: encounter.objectID,
      value: {
        zoneID,
        x: pos.x,
        y: pos.y,
      },
      confidence: "high",
      provenance: {
        session: session.name ?? "(unnamed session)",
        t: encounter.t,
      },
      token: encounter.token,
    });
  }

  return observations as Observation<SpawnObservationValue>[];
};
