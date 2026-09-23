// npcKeys.spawns - Questie field 7.
// type: table {[zoneID(int)] = {coordPair(floatVector2D),...},...}
//
// Observed by correlating npc encounters with the player's map position at the
// same time. The `questnpc` token is preferred because it fires when the player
// interacts with a quest-giving NPC (click "accept"/"details"), which means the
// NPC is guaranteed to be right next to the player. The `npc` and `target` tokens
// are used as fallback but may capture the NPC at a distance.
//
// Output shape: Observation<SpawnObservationValue> where each observation
// represents one spawn point seen at a specific zone/coordinate, keyed by npcID.
// The custom merge (mergeSpawns) accumulates these into the final
// {[zoneID]: {{x,y},...},...} table.

import type { SessionRecord } from "../../../core/types";
import type { FieldObserver, Observation } from "../../observation";
import { npcEncounters } from "./_encounters";
import { playerZoneAt, playerPosAt } from "../../probes";

/** Token priority for spawn positioning: questnpc > npc > target.
 *
 * questnpc fires on QUEST_DETAIL/QUEST_ACCEPTED when interacting with a
 * quest NPC — the player must be adjacent to the NPC for the interaction to
 * succeed, so the position is a high-quality spawn point.
 *
 * npc/target tokens fire whenever the player targets or mobs an NPC, which
 * can happen from a distance; these are still useful but less precise. */
const TOKEN_PRIORITY: Record<string, number> = {
  questnpc: 3,
  npc: 2,
  target: 1,
};

/** Tagged observation that carries the unit token for priority sorting. */
export interface TaggedSpawnObservation extends Observation<SpawnObservationValue> {
  token: string;
}

export interface SpawnObservationValue {
  zoneID: number;
  x: number;
  y: number;
}

export const observeSpawns: FieldObserver<SpawnObservationValue> = (session) => {
  const observations: TaggedSpawnObservation[] = [];
  const encounters = npcEncounters(session);

  for (const encounter of encounters) {
    const zoneID = playerZoneAt(session, encounter.t);
    if (zoneID === null) continue;

    const pos = playerPosAt(session, encounter.t);
    if (pos === null) continue;

    observations.push({
      entityId: encounter.npcID,
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
