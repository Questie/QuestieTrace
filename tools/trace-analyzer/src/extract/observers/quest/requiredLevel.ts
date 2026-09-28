// questKeys.requiredLevel - Questie field 4.
//
// Primary source: QuestAcceptLevel stream recorded by addon at QUEST_ACCEPTED
// (player's level at acceptance time). Take the minimum observed acceptance
// level across all sessions.
//
// Fallback: If QuestAcceptLevel is not available (older traces), derive from
// the minimum questLevel observed across all encounters — a quest's required
// level is typically <= its own level, so the minimum observed questLevel is a
// conservative lower bound.

import { getParamKeys, getStream } from "../../../core/emulator";
import { observeQuestLevel } from "./questLevel";
import type { SessionRecord } from "../../../core/types";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

export const observeRequiredLevel: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];

  // --- Primary: QuestAcceptLevel (direct observation at acceptance) ---
  const acceptRoot = session.functions["QuestAcceptLevel"];
  if (acceptRoot && !Array.isArray(acceptRoot)) {
    const acceptObs = new Map<number, { level: number; t: number }>();

    for (const questIdKey of getParamKeys(acceptRoot)) {
      const questID = Number(questIdKey);
      if (!Number.isFinite(questID)) continue;

      const stream = getStream(session, "QuestAcceptLevel", questIdKey) ?? [];
      if (stream.length === 0) continue;

      let minLevel: number | null = null;
      let firstT = 0;
      for (const entry of stream) {
        const level = typeof entry.v === "number" ? entry.v : null;
        if (level === null) continue;
        if (minLevel === null || level < minLevel) {
          minLevel = level;
        }
        if (firstT === 0) firstT = entry.t;
      }

      if (minLevel !== null) {
        acceptObs.set(questID, { level: minLevel, t: firstT });
      }
    }

    // Emit observations from QuestAcceptLevel
    for (const [entityId, { level, t }] of acceptObs) {
      observations.push({
        entityId,
        value: level,
        confidence: "high",
        provenance: { session: sessionLabel(session), t },
      });
    }
  }

  // --- Fallback: questLevel (for traces without QuestAcceptLevel) ---
  if (observations.length > 0) {
    // Already have high-confidence data from QuestAcceptLevel
    return observations;
  }

  const questLevelObs = observeQuestLevel(session);
  if (questLevelObs.length === 0) return [];

  const minByEntity: Map<number, { level: number; t: number }> = new Map();
  for (const obs of questLevelObs) {
    const current = minByEntity.get(obs.entityId);
    if (current === undefined) {
      // First observation for this quest
      minByEntity.set(obs.entityId, { level: obs.value, t: obs.provenance.t });
    } else if (obs.value < current.level) {
      // Found a new minimum - update level but keep original t for the minimum
      minByEntity.set(obs.entityId, { level: obs.value, t: current.t });
    }
  }

  for (const [entityId, { level, t }] of minByEntity) {
    observations.push({
      entityId,
      value: level,
      confidence: "medium", // derived from questLevel, not directly observed
      provenance: { session: sessionLabel(session), t },
    });
  }

  return observations;
};