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
  return emitRecords(facts);
}
