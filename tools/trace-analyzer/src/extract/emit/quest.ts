// Fact -> named record for the quest entity kind. See emit/npc.ts for the general shape.

import type { Fact } from "../aggregate";
import { emitRecords } from "./records";

export interface QuestFacts {
  [field: string]: Map<number, Fact<string>>;
  name: Map<number, Fact<string>>;
}

export function emitQuestRecords(facts: QuestFacts): Map<number, Record<string, unknown>> {
  return emitRecords(facts);
}
