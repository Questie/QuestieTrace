// Fact -> named record for the quest entity kind. See emit/npc.ts for the general shape.

import type { Fact } from "../aggregate";
import { emitRecords } from "./records";

export interface QuestFacts {
  [field: string]: Map<number, Fact<unknown>>;
  name: Map<number, Fact<string>>;
  questLevel: Map<number, Fact<number>>;
  requiredLevel: Map<number, Fact<number>>;
  zoneOrSort: Map<number, Fact<number>>;
  startedBy: Map<number, Fact<{ creatures: number[]; objects: number[]; items: number[] }>>;
  finishedBy: Map<number, Fact<{ creatures: number[]; objects: number[] }>>;
}

export function emitQuestRecords(facts: QuestFacts): Map<number, Record<string, unknown>> {
  return emitRecords(facts);
}
