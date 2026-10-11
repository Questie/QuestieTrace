// npc/object questStarts/questEnds are the inverse of quest startedBy/finishedBy.
// Deriving them from the exported quest records keeps both sides consistent.

export interface QuestRelations {
  questStarts?: number[];
  questEnds?: number[];
}

interface QuestStarters {
  creatures: number[];
  objects: number[];
}

export function invertQuestRelations(quests: Map<number, Record<string, unknown>>): {
  npcs: Map<number, QuestRelations>;
  objects: Map<number, QuestRelations>;
} {
  const npcs = new Map<number, QuestRelations>();
  const objects = new Map<number, QuestRelations>();

  function add(target: Map<number, QuestRelations>, id: number, field: keyof QuestRelations, questId: number): void {
    const relations = target.get(id) ?? {};
    (relations[field] ??= []).push(questId);
    target.set(id, relations);
  }

  const sortedQuestIds = [...quests.keys()].sort((a, b) => a - b);
  // One pass per field, so questStarts is always created (and exported) before questEnds.
  for (const [questField, relationField] of [["startedBy", "questStarts"], ["finishedBy", "questEnds"]] as const) {
    for (const questId of sortedQuestIds) {
      const relation = quests.get(questId)![questField] as QuestStarters | undefined;
      for (const id of relation?.creatures ?? []) add(npcs, id, relationField, questId);
      for (const id of relation?.objects ?? []) add(objects, id, relationField, questId);
    }
  }

  return { npcs, objects };
}
