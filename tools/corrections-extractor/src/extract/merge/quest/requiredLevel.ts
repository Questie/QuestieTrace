// questKeys.requiredLevel - Questie field 4. Observed in observers/quest/requiredLevel.ts.
// The lowest level across sessions wins.

import type { Observation } from "../../observation";

export function mergeRequiredLevel(observations: Observation<number>[]): number {
  let min = Infinity;
  for (const obs of observations) {
    if (obs.value < min) min = obs.value;
  }
  return min;
}
