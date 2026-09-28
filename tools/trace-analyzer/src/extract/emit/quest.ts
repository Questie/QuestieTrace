// Fact -> named record for the quest entity kind. See emit/npc.ts for the general shape.

import type { Fact } from "../aggregate";
import type { QuestObjectivesValue } from "../observers/quest/objectives";
import type { QuestObjectivesTextValue } from "../observers/quest/objectivesText";
import type { QuestTriggerEndValue } from "../observers/quest/triggerEnd";
import { emitRecords } from "./records";

export interface QuestFacts {
  [field: string]: Map<number, Fact<unknown>>;
  name: Map<number, Fact<string>>;
  questLevel: Map<number, Fact<number>>;
  requiredLevel: Map<number, Fact<number>>;
  zoneOrSort: Map<number, Fact<number>>;
  objectives: Map<number, Fact<QuestObjectivesValue>>;
  objectivesText: Map<number, Fact<QuestObjectivesTextValue>>;
  triggerEnd: Map<number, Fact<QuestTriggerEndValue>>;
  startedBy: Map<number, Fact<{ creatures: number[]; objects: number[]; items: number[] }>>;
  finishedBy: Map<number, Fact<{ creatures: number[]; objects: number[] }>>;
}

export function emitQuestRecords(facts: QuestFacts): Map<number, Record<string, unknown>> {
  // Clamp requiredLevel to questLevel: requiredLevel should never exceed questLevel
  // (except for extremely rare exceptions which we cap for data quality)
  const clampedFacts: QuestFacts = { ...facts };
  if (facts.requiredLevel && facts.questLevel) {
    const clampedRequiredLevel = new Map<number, Fact<number>>();
    for (const [questId, reqFact] of facts.requiredLevel) {
      const questLevelFact = facts.questLevel.get(questId);
      if (questLevelFact && reqFact.value > questLevelFact.value) {
        clampedRequiredLevel.set(questId, {
          ...reqFact,
          value: questLevelFact.value,
        });
      } else {
        clampedRequiredLevel.set(questId, reqFact);
      }
    }
    clampedFacts.requiredLevel = clampedRequiredLevel;
  }
  return emitRecords(clampedFacts);
}
