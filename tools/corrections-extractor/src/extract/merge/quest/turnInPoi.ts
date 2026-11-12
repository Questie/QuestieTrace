// Attaches quest turn-in POIs (observers/quest/turnInPoi.ts) to the quest's finisher
// as spawn observations with the `questpoi` token.

import type { Observation } from "../../observation";
import type { TurnInPoiValue } from "../../observers/quest/turnInPoi";
import type { FinishedByValue } from "./finishedBy";

export interface TurnInSpawnObservation extends Observation<TurnInPoiValue> {
  token: "questpoi";
}

/**
 * Converts quest turn-in POIs into spawn observations for the quest's single
 * finisher. Quests with zero or multiple finishers are skipped, since the POI
 * can't be attributed unambiguously.
 */
export function turnInPoiSpawns(
  pois: Observation<TurnInPoiValue>[],
  finishedBy: Map<number, { value: FinishedByValue }>,
): { npcs: TurnInSpawnObservation[]; objects: TurnInSpawnObservation[] } {
  const npcs: TurnInSpawnObservation[] = [];
  const objects: TurnInSpawnObservation[] = [];

  for (const poi of pois) {
    const finisher = finishedBy.get(poi.entityId)?.value;
    if (!finisher) continue;
    const { creatures, objects: objectIds } = finisher;
    if (creatures.length + objectIds.length !== 1) continue;

    const target = creatures.length === 1 ? npcs : objects;
    target.push({ ...poi, entityId: creatures[0] ?? objectIds[0], token: "questpoi" });
  }

  return { npcs, objects };
}
