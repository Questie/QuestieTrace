// Fact -> named record for the item entity kind. See emit/npc.ts for the general shape.

import type { Fact } from "../aggregate";
import { emitRecords } from "./records";

export interface ItemFacts {
  [field: string]: Map<number, Fact<unknown>>;
  name: Map<number, Fact<string>>;
  npcDrops: Map<number, Fact<number[]>>;
  objectDrops: Map<number, Fact<number[]>>;
}

export function emitItemRecords(facts: ItemFacts): Map<number, Record<string, unknown>> {
  return emitRecords(facts);
}
