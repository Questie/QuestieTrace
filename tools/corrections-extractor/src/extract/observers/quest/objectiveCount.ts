// Number of objectives C_QuestLog.GetQuestObjectives reports for a quest. Not a
// Questie field: finalize uses it to drop questKeys.objectives when not every
// objective could be matched to an id. Counting needs no text, so every locale
// is observed. "event" objectives are exported as questKeys.triggerEnd and
// "log" entries carry no entity, so neither belongs to questKeys.objectives.
// Every other type counts, including ones the matcher can't resolve (e.g.
// reputation): exporting the remaining ids would leave the quest incomplete.

import { emulate, getParamKeys, getStream } from "../../../core/emulator";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

const NON_OBJECTIVE_TYPES = new Set(["event", "log"]);

export const observeObjectiveCount: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];
  const root = session.functions["C_QuestLog.GetQuestObjectives"];
  if (!root || Array.isArray(root)) return observations;

  for (const questIdKey of getParamKeys(root)) {
    const questID = Number(questIdKey);
    if (!Number.isFinite(questID)) continue;

    for (const entry of getStream(session, "C_QuestLog.GetQuestObjectives", questIdKey) ?? []) {
      const objectives = emulate(entry.v);
      if (!Array.isArray(objectives) || objectives.length === 0) continue;
      const count = objectives.filter(
        (objective) => objective !== null && typeof objective === "object" && !NON_OBJECTIVE_TYPES.has(String((objective as { type?: unknown }).type)),
      ).length;
      observations.push({
        entityId: questID,
        value: count,
        confidence: "high",
        provenance: { session: sessionLabel(session), t: entry.t },
      });
    }
  }

  return observations;
};
