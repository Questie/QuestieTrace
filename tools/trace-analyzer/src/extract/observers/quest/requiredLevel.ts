// questKeys.requiredLevel - Questie field 4.
//
// No addon-side signal exists for this field in Classic WoW (there is no
// GetQuestRequiredLevel API). As a reasonable proxy, we use the lowest
// questLevel observed for this quest across all encounters — a quest's required
// level is typically <= its own level, so the minimum observed questLevel is a
// conservative lower bound.
//
// We cannot source this from a single encounter (the questLevel observer emits
// per-encounter values, but requiredLevel needs the cross-encounter minimum),
// so we read questLevel observations directly and take the minimum.

import { observeQuestLevel } from "./questLevel";
import type { FieldObserver, Observation } from "../../observation";

export const observeRequiredLevel: FieldObserver<number> = (session) => {
  const questLevelObs = observeQuestLevel(session);
  if (questLevelObs.length === 0) return [];

  // Group by entityId and take the minimum questLevel per quest
  const minByEntity: Map<number, number> = new Map();
  for (const obs of questLevelObs) {
    const current = minByEntity.get(obs.entityId);
    if (current === undefined || obs.value < current) {
      minByEntity.set(obs.entityId, obs.value);
    }
  }

  return [...minByEntity.entries()].map(([entityId, value]) => ({
    entityId,
    value,
    confidence: "medium", // derived from questLevel, not directly observed
    provenance: { session: questLevelObs[0].provenance.session, t: 0 },
  }));
};
