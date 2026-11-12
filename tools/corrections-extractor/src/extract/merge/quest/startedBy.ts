// questKeys.startedBy - Questie field 2. Observed in observers/quest/startedBy.ts.
//
// Custom merge: set-union per starter type of the starters seen in at least
// MIN_GIVER_SESSIONS sessions (see relations.ts); no value when there is none.

import type { Observation } from "../../observation";
import type { StartedByStarter } from "../../observers/quest/startedBy";
import { supportedIds } from "./relations";

export interface StartedByValue {
  creatures: number[];
  objects: number[];
  items: number[];
}

export function mergeStartedBy(observations: Observation<StartedByStarter>[]): StartedByValue | undefined {
  const creatures = supportedIds(observations, (s) => s.creatureId);
  const objects = supportedIds(observations, (s) => s.objectId);
  const items = supportedIds(observations, (s) => s.itemId);
  return creatures.length > 0 || objects.length > 0 || items.length > 0 ? { creatures, objects, items } : undefined;
}
