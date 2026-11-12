// questKeys.objectives - Questie field 10. Observed in observers/quest/objectives.ts.

import type { Observation } from "../../observation";
import type { ObjectiveKind, ObjectiveMatch } from "../../observers/quest/objectives";

// Questie questKeys.objectives positional indices
// {creatureObjective, objectObjective, itemObjective, reputationObjective,
// killCreditObjective, spellObjective}. Only these three of the six positions
// are ever populated.
export const CREATURE_OBJECTIVE_INDEX = 1;
export const OBJECT_OBJECTIVE_INDEX = 2;
export const ITEM_OBJECTIVE_INDEX = 3;

/** Positional Questie objectives table: {1: creatures, 2: objects, 3: items}. */
export type QuestObjectivesValue = Partial<
  Record<typeof CREATURE_OBJECTIVE_INDEX | typeof OBJECT_OBJECTIVE_INDEX | typeof ITEM_OBJECTIVE_INDEX, number[]>
>;

/** Merge matched objectives across observations into Questie's positional buckets. */
export function mergeQuestObjectives(observations: Observation<ObjectiveMatch[]>[]): QuestObjectivesValue {
  // Order by the objective's index in the C_QuestLog.GetQuestObjectives result,
  // since unmatched objectives leave gaps in each observation's match list.
  // Different observations may resolve the same objective slot to different
  // ids (e.g. several creatures sharing a name), so each slot is reduced to a
  // single id: the one matched in the most sessions, lowest id on ties.
  const sessionsBySlot: Record<ObjectiveKind, Map<number, Map<number, Set<number | string>>>> = {
    monster: new Map(),
    object: new Map(),
    item: new Map(),
  };
  for (const observation of observations) {
    for (const match of observation.value) {
      const slots = sessionsBySlot[match.kind];
      const candidates = slots.get(match.index) ?? new Map<number, Set<number | string>>();
      const sessions = candidates.get(match.id) ?? new Set<number | string>();
      sessions.add(observation.provenance.sessionKey ?? observation.provenance.session);
      candidates.set(match.id, sessions);
      slots.set(match.index, candidates);
    }
  }

  const pick = (candidates: Map<number, Set<number | string>>) =>
    [...candidates].sort((a, b) => b[1].size - a[1].size || a[0] - b[0])[0][0];

  const ordered = (kind: ObjectiveKind) => {
    const ids = [...sessionsBySlot[kind]].sort((a, b) => a[0] - b[0]).map(([, candidates]) => pick(candidates));
    return [...new Set(ids)];
  };

  const creatures = ordered("monster");
  const objects = ordered("object");
  const items = ordered("item");

  const value: QuestObjectivesValue = {};
  if (creatures.length > 0) value[CREATURE_OBJECTIVE_INDEX] = creatures;
  if (objects.length > 0) value[OBJECT_OBJECTIVE_INDEX] = objects;
  if (items.length > 0) value[ITEM_OBJECTIVE_INDEX] = items;
  return value;
}
