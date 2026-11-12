// questKeys.finishedBy - Questie field 3. Observed in observers/quest/finishedBy.ts.
//
// Custom merge: set-union per finisher type of the finishers seen in at least
// MIN_GIVER_SESSIONS sessions (see relations.ts); no value when there is none.

import type { Observation } from "../../observation";
import type { FinishedByFinisher } from "../../observers/quest/finishedBy";
import { supportedIds } from "./relations";

export interface FinishedByValue {
  creatures: number[];
  objects: number[];
}

export function mergeFinishedBy(observations: Observation<FinishedByFinisher>[]): FinishedByValue | undefined {
  const creatures = supportedIds(observations, (f) => f.creatureId);
  const objects = supportedIds(observations, (f) => f.objectId);
  return creatures.length > 0 || objects.length > 0 ? { creatures, objects } : undefined;
}
